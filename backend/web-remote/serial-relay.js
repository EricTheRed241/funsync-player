// TCode browser serial relay — the web remote's end ("sink").
//
// A TCode device plugged into THIS computer is opened with Web Serial and
// attached to the backend relay (`/api/remote/tcode-relay?role=sink`). The
// desktop app keeps syncing scripts; this tab is a dumb pipe: every raw
// frame from the relay is written to the port, every line of device output
// goes back as `{type:'reply'}`, and port open/close is reported as
// `{type:'port'}`. JSON frames from the relay (first char `{`) are control:
// `config` (baud rate) and `kicked` (another browser took over).
//
// Browser objects are injected so the logic runs under vitest with fakes.

export const RELAY_PATH = '/api/remote/tcode-relay';
export const DEFAULT_BAUD = 115200;
export const RECONNECT_MS = 3000;
export const STORAGE_KEY = 'funsync.serialRelay';
const MAX_REPLY = 256;
// Writes still pending in the port's writer. TCode is latest-wins, so when
// the device can't keep up, new frames are dropped instead of queued.
const MAX_PENDING_WRITES = 32;
// Device output without a newline is flushed once it gets this long.
const MAX_LINE_BUFFER = 1024;

/** The `{usbVendorId, usbProductId}` of a port, keys present only when known. */
function portInfo(port) {
  let info = {};
  try { info = port?.getInfo?.() || {}; } catch { /* treat as unknown */ }
  const out = {};
  if (Number.isInteger(info.usbVendorId)) out.usbVendorId = info.usbVendorId;
  if (Number.isInteger(info.usbProductId)) out.usbProductId = info.usbProductId;
  return out;
}

function sameDevice(port, device) {
  if (!device || !Number.isInteger(device.usbVendorId)) return false;
  const info = portInfo(port);
  return info.usbVendorId === device.usbVendorId && info.usbProductId === device.usbProductId;
}

/** Relay URL on the page's own host: wss on an https page, ws otherwise. */
export function relayUrl(location) {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${location.host}${RELAY_PATH}?role=sink`;
}

/** Desktop Chrome / Edge (not Android, not iOS): the browsers with Web Serial. */
export function looksLikeDesktopChromium(userAgent) {
  const ua = String(userAgent || '');
  return /Chrome\/|Edg\//.test(ua) && !/Mobile|Android/.test(ua);
}

export class SerialRelaySink {
  /**
   * Whether this page can use Web Serial: `navigator.serial` present →
   * supported; otherwise 'insecure' when the page isn't a secure context
   * (https fixes it), else 'unsupported' (browser without Web Serial).
   */
  static support({ serial, isSecureContext }) {
    if (serial) return { supported: true, reason: null };
    return { supported: false, reason: isSecureContext ? 'unsupported' : 'insecure' };
  }

  constructor({
    serial, WebSocketImpl, location, storage, isSecureContext = true,
    setTimeout: setTimer = globalThis.setTimeout, clearTimeout: clearTimer = globalThis.clearTimeout,
    onState = () => {},
  }) {
    this._serial = serial || null;
    this._WebSocket = WebSocketImpl;
    this._location = location;
    this._storage = storage;
    this._secure = !!isSecureContext;
    // Called as plain functions: the browser's setTimeout throws "Illegal
    // invocation" when called as a method of another object.
    this._setTimeout = (fn, ms) => setTimer(fn, ms);
    this._clearTimeout = (id) => clearTimer(id);
    this._onState = onState;

    const saved = this._load();
    this._device = saved.device;
    this._baud = saved.baudRate;
    this._enabled = false;

    this._ws = null;
    this._relay = 'off';
    this._retryTimer = null;
    this._kicked = false;

    this._port = null;          // the open SerialPort
    this._portState = 'none';
    this._writer = null;
    this._reader = null;
    this._readDone = null;
    this._pending = 0;
    this._closing = false;
    this._portOps = Promise.resolve(); // open/close run one at a time
    this._error = null;
    this._eventsBound = false;
    this._encoder = new TextEncoder();
  }

  get state() {
    return {
      supported: !!this._serial,
      secure: this._secure,
      relay: this._relay,
      port: this._portState,
      device: this._device ? { ...this._device } : null,
      baudRate: this._baud,
      error: this._error,
      kicked: this._kicked,
    };
  }

  /** User click: pick a device, remember it, open it and attach to the relay. */
  async connect() {
    if (!this._serial) return false;
    this._bindSerialEvents();
    let port;
    try {
      port = await this._serial.requestPort();
    } catch (err) {
      // NotFoundError = the user closed the picker without choosing.
      this._error = err?.name === 'NotFoundError' ? null : errorText(err);
      this._emit();
      return false;
    }
    this._kicked = false;
    this._error = null;
    this._enabled = true;
    const info = portInfo(port);
    this._device = Number.isInteger(info.usbVendorId) ? info : null;
    this._save();
    this._openRelay();
    await this._queue(() => this._open(port));
    return this._portState === 'open';
  }

  /** Page load: reopen the remembered device without a click when enabled. */
  async resume() {
    if (!this._serial) return;
    this._bindSerialEvents();
    if (!this._load().enabled) return;
    this._enabled = true;
    this._openRelay();
    const port = await this._findRemembered();
    if (port && this._enabled) await this._queue(() => this._open(port));
  }

  /** User: close port and relay, stop auto-reconnect; the device stays remembered. */
  async disconnect() {
    this._enabled = false;
    this._save();
    this._cancelRetry();
    await this._queue(() => this._close());
    const ws = this._ws;
    this._ws = null;
    this._relay = 'off';
    try { ws?.close(); } catch { /* already closed */ }
    this._emit();
  }

  // --- storage ---------------------------------------------------------

  _load() {
    let saved = null;
    try { saved = JSON.parse(this._storage?.getItem(STORAGE_KEY) || 'null'); } catch { /* corrupt → defaults */ }
    const device = saved?.device && Number.isInteger(saved.device.usbVendorId)
      ? { usbVendorId: saved.device.usbVendorId, usbProductId: saved.device.usbProductId }
      : null;
    const baudRate = Number.isInteger(saved?.baudRate) && saved.baudRate > 0 ? saved.baudRate : DEFAULT_BAUD;
    return { device, enabled: saved?.enabled === true, baudRate };
  }

  _save() {
    try {
      this._storage?.setItem(STORAGE_KEY, JSON.stringify({
        device: this._device, enabled: this._enabled, baudRate: this._baud,
      }));
    } catch { /* storage full or blocked: only the auto-resume is lost */ }
  }

  // --- relay socket ----------------------------------------------------

  _openRelay() {
    if (this._ws || !this._enabled) return;
    this._cancelRetry();
    let ws;
    try {
      ws = new this._WebSocket(relayUrl(this._location));
    } catch (err) {
      this._error = errorText(err);
      this._scheduleRetry();
      this._emit();
      return;
    }
    this._ws = ws;
    this._relay = 'connecting';
    this._emit();

    ws.addEventListener('open', () => {
      if (this._ws !== ws) return;
      this._relay = 'open';
      this._reportPort();
      this._emit();
    });
    ws.addEventListener('message', (evt) => {
      if (this._ws === ws) this._onRelayMessage(evt.data);
    });
    ws.addEventListener('close', () => {
      if (this._ws !== ws) return;
      this._ws = null;
      this._relay = 'off';
      this._scheduleRetry();
      this._emit();
    });
    ws.addEventListener('error', () => { /* 'close' follows */ });
  }

  _onRelayMessage(data) {
    if (typeof data !== 'string' || data.length === 0) return;
    if (data[0] !== '{') { this._write(data); return; }
    let msg;
    try { msg = JSON.parse(data); } catch { return; }
    if (msg?.type === 'config') this._onConfig(msg.baudRate);
    else if (msg?.type === 'kicked') this._onKicked();
  }

  _onConfig(baudRate) {
    if (!Number.isInteger(baudRate) || baudRate <= 0 || baudRate === this._baud) return;
    this._baud = baudRate;
    this._save();
    this._emit();
    this._queue(async () => {
      const port = this._port;
      if (!port) return;
      await this._close();
      await this._open(port);
    });
  }

  /** Another browser attached: let go of the device and stay down until a click. */
  _onKicked() {
    this._kicked = true;
    this._enabled = false;
    this._save();
    this._cancelRetry();
    this._emit();
    this._queue(() => this._close());
  }

  _send(msg) {
    const ws = this._ws;
    if (!ws || this._relay !== 'open') return;
    try { ws.send(JSON.stringify(msg)); } catch { /* close follows */ }
  }

  _reportPort() {
    const open = this._portState === 'open';
    this._send({
      type: 'port',
      open,
      info: open ? portInfo(this._port) : (this._device ? { ...this._device } : null),
      baudRate: open ? this._baud : null,
    });
  }

  _scheduleRetry() {
    if (!this._enabled || this._kicked || this._retryTimer) return;
    this._retryTimer = this._setTimeout(() => {
      this._retryTimer = null;
      this._openRelay();
    }, RECONNECT_MS);
  }

  _cancelRetry() {
    if (this._retryTimer) this._clearTimeout(this._retryTimer);
    this._retryTimer = null;
  }

  // --- serial port -----------------------------------------------------

  _bindSerialEvents() {
    if (this._eventsBound || !this._serial?.addEventListener) return;
    this._eventsBound = true;
    this._serial.addEventListener('connect', (e) => this._onUsbConnect(e));
    this._serial.addEventListener('disconnect', (e) => {
      if (this._port && e?.target === this._port) this._queue(() => this._close());
    });
  }

  async _onUsbConnect(e) {
    if (!this._enabled || this._port) return;
    const target = e?.target;
    const port = typeof target?.getInfo === 'function'
      ? (sameDevice(target, this._device) || !this._device ? target : null)
      : await this._findRemembered();
    if (port && this._enabled && !this._port) await this._queue(() => this._open(port));
  }

  /** The remembered device among the granted ports, else the only granted port. */
  async _findRemembered() {
    let ports = [];
    try { ports = await this._serial.getPorts(); } catch { return null; }
    return ports.find((p) => sameDevice(p, this._device)) || (ports.length === 1 ? ports[0] : null);
  }

  _queue(fn) {
    const run = this._portOps.then(fn);
    this._portOps = run.catch(() => {});
    return run;
  }

  async _open(port) {
    if (this._port) await this._close();
    try {
      await port.open({ baudRate: this._baud });
    } catch (err) {
      this._portState = 'closed';
      this._error = errorText(err);
      this._reportPort();
      this._emit();
      return;
    }
    this._port = port;
    this._writer = port.writable.getWriter();
    this._pending = 0;
    this._portState = 'open';
    this._error = null;
    this._readDone = this._readLoop(port);
    this._reportPort();
    this._emit();
  }

  async _close() {
    const port = this._port;
    if (!port) return;
    this._closing = true;
    const writer = this._writer;
    this._writer = null;
    try { await this._reader?.cancel(); } catch { /* port already gone */ }
    try { await this._readDone; } catch { /* loop never throws */ }
    try { await writer?.abort(); } catch { /* port already gone */ }
    try { writer?.releaseLock(); } catch { /* not locked */ }
    try { await port.close(); } catch { /* unplugged */ }
    this._port = null;
    this._reader = null;
    this._readDone = null;
    this._closing = false;
    this._portState = 'closed';
    this._reportPort();
    this._emit();
  }

  /** One writer, never awaited per frame; a failed write closes the port. */
  _write(text) {
    const writer = this._writer;
    if (!writer || this._pending >= MAX_PENDING_WRITES) return;
    this._pending++;
    writer.write(this._encoder.encode(text)).then(
      () => { if (this._writer === writer) this._pending--; },
      (err) => {
        if (this._writer !== writer) return;
        this._error = errorText(err);
        this._queue(() => this._close());
      },
    );
  }

  async _readLoop(port) {
    const decoder = new TextDecoder();
    let buf = '';
    // A non-fatal read error (framing, overrun) swaps in a new `readable`,
    // so keep reading while the port has one and is still ours.
    while (port.readable && this._port === port && !this._closing) {
      const reader = port.readable.getReader();
      this._reader = reader;
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += decoder.decode(value, { stream: true });
          buf = this._sendLines(buf);
        }
      } catch {
        // fatal errors leave `port.readable` null and end the loop
      } finally {
        try { reader.releaseLock(); } catch { /* already released */ }
      }
      if (this._closing) break;
    }
    // Ended without close(): the device went away.
    if (this._port === port && !this._closing) this._queue(() => this._close());
  }

  /** Send each complete line as a reply; return the unfinished rest. */
  _sendLines(buf) {
    const parts = buf.split(/\r?\n|\r/);
    let rest = parts.pop();
    if (rest.length > MAX_LINE_BUFFER) { parts.push(rest); rest = ''; }
    for (const part of parts) {
      const text = part.trim().slice(0, MAX_REPLY);
      if (text) this._send({ type: 'reply', text });
    }
    return rest;
  }

  _emit() {
    try { this._onState(this.state); } catch { /* UI errors never break the relay */ }
  }
}

function errorText(err) {
  return String(err?.message || err || 'Error');
}
