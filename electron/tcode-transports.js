// Main-process TCode transports.
//
// Originally TCode shipped with a hardcoded `serialport` transport — every
// IPC handler reached for the same global `tcodePort` SerialPort instance.
// Community feedback (GGEZGitGud, 2026-05-15) asked for a UDP option:
// "2.4GHz should be more stable than Bluetooth". GGEZGitGud also asked
// whether WebSocket would be easy to integrate — same protocol formatter,
// different wire.
//
// This module separates "how do I get bytes onto a device" from "what
// bytes do I send" (the formatter still lives in
// `renderer/js/tcode-manager.js`). Each transport implements the same
// shape so the IPC dispatcher in `main.js` can route to whichever one
// the user picked at connect time.
//
// Transport contract:
//   async connect(opts): Promise<{ success: boolean, error?: string }>
//   async disconnect(): Promise<void>
//   send(command: string): boolean   // sync write; false on failure
//   get connected(): boolean
//   onDisconnect(callback): void     // single subscriber; replaces prior
//   destroy(): void                  // hard teardown, idempotent

const dgram = require('dgram');
const dns = require('dns');
const net = require('net');

// Some Electron / Node sandboxes expose WebSocket globally (Node 22+).
// Fall back gracefully — UDP can still be selected even if WS isn't
// available in this environment.
const HAS_GLOBAL_WS = typeof globalThis.WebSocket !== 'undefined';

// WebSocket reconnect policy after a drop (user, 2026-11-03): retry every
// 5 s for the first 30 minutes the device is gone, then every 60 s.
const FAST_RETRY_MS = 5000;
const FAST_WINDOW_MS = 30 * 60 * 1000;
const SLOW_RETRY_MS = 60 * 1000;
// The same failure repeated is logged at most this often (with a count of
// the attempts in between); a different failure is logged at once.
const FAST_FAILURE_LOG_MS = 60 * 1000;
const SLOW_FAILURE_LOG_MS = 10 * 60 * 1000;
// Diagnosis of a failed attempt. The runtime's WebSocket (undici) reports
// every failure as a bare ErrorEvent plus close code 1006, so the cause
// comes from our own DNS lookup and TCP probe of the same host and port.
const LOOKUP_TIMEOUT_MS = 6000;
const PROBE_TIMEOUT_MS = 3000;

function retryPlanText() {
  return `retrying every ${FAST_RETRY_MS / 1000}s for ${FAST_WINDOW_MS / 60000}m, then every ${SLOW_RETRY_MS / 1000}s`;
}

const CLOSE_CODES = {
  1000: 'normal',
  1001: 'going away',
  1002: 'protocol error',
  1003: 'unsupported data',
  1005: 'no status',
  1006: 'abnormal, no close frame',
  1007: 'invalid data',
  1008: 'policy violation',
  1009: 'message too big',
  1011: 'server error',
  1012: 'service restart',
  1013: 'try again later',
  1015: 'TLS failure',
};

function fmtDuration(ms) {
  if (!Number.isFinite(ms) || ms < 0) return '?';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return h ? `${h}h${String(m).padStart(2, '0')}m` : `${m}m${String(s % 60).padStart(2, '0')}s`;
}

function fmtBytes(n) {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / (1024 * 1024)).toFixed(1)}MB`;
}

function closeText(ev) {
  const code = ev?.code;
  const name = CLOSE_CODES[code];
  const reason = ev?.reason ? ` reason=${JSON.stringify(ev.reason)}` : '';
  const clean = typeof ev?.wasClean === 'boolean' ? ` clean=${ev.wasClean}` : '';
  return `close=${code ?? '?'}${name ? ` (${name})` : ''}${reason}${clean}`;
}

function parseTarget(url) {
  const u = new URL(url);
  return {
    host: u.hostname.replace(/^\[|\]$/g, ''),
    port: Number(u.port) || (u.protocol === 'wss:' ? 443 : 80),
  };
}

// IPv4 first: devices on the LAN (MiraBot over mDNS) answer on IPv4, and
// `localhost` resolving to ::1 first would point the TCP probe at the
// wrong socket.
async function lookupHost(host) {
  const all = await dns.promises.lookup(host, { all: true });
  return all.find((a) => a.family === 4) || all[0];
}

/** TCP connect to host:port and hang up: `{ ok, code?, ms }`, never rejects. */
function probeTcp(host, port, timeoutMs) {
  return new Promise((resolve) => {
    const t0 = Date.now();
    let done = false;
    const sock = net.connect({ host, port });
    const finish = (result) => {
      if (done) return;
      done = true;
      sock.destroy();
      resolve({ ...result, ms: Date.now() - t0 });
    };
    sock.setTimeout(timeoutMs, () => finish({ ok: false, code: 'ETIMEDOUT' }));
    sock.once('connect', () => finish({ ok: true }));
    sock.once('error', (err) => finish({ ok: false, code: err.code || err.message }));
  });
}

function describeTcpFailure(where, code) {
  switch (code) {
    case 'ECONNREFUSED':
      return `${where} refused the connection (ECONNREFUSED): the device is up but its WebSocket server isn't listening`;
    case 'ETIMEDOUT':
      return `no answer from ${where} within ${fmtDuration(PROBE_TIMEOUT_MS)} (ETIMEDOUT)`;
    case 'EHOSTUNREACH':
    case 'EHOSTDOWN':
    case 'ENETUNREACH':
      return `${where} is unreachable (${code})`;
    default:
      return `TCP connect to ${where} failed (${code})`;
  }
}

class SerialTransport {
  constructor({ log = console } = {}) {
    this._port = null;
    this._log = log;
    this._onDisconnect = null;
  }

  async connect({ path, baudRate = 115200 } = {}) {
    try {
      // Hard-reset any previous instance — the SerialPort 'close' event
      // would normally clear `_port`, but a fresh connect attempt should
      // not depend on that event having fired yet.
      if (this._port) {
        try { this._port.removeAllListeners(); } catch {}
        try { if (this._port.isOpen) this._port.close(); } catch {}
        this._port = null;
      }
      const { SerialPort } = require('serialport');
      this._port = new SerialPort({
        path,
        baudRate,
        dataBits: 8,
        stopBits: 1,
        parity: 'none',
        autoOpen: false,
      });
      return await new Promise((resolve) => {
        this._port.open((err) => {
          if (err) {
            this._log.warn?.('[TCode/serial] Open failed:', err.message);
            this._port = null;
            resolve({ success: false, error: err.message });
            return;
          }
          this._port.on('close', () => {
            this._log.info?.('[TCode/serial] Port closed');
            this._port = null;
            if (this._onDisconnect) this._onDisconnect();
          });
          this._port.on('error', (e) => {
            this._log.warn?.('[TCode/serial] Port error:', e.message);
          });
          this._log.info?.(`[TCode/serial] Connected to ${path} @ ${baudRate}`);
          resolve({ success: true });
        });
      });
    } catch (err) {
      this._log.warn?.('[TCode/serial] Connect error:', err.message);
      return { success: false, error: err.message };
    }
  }

  async disconnect() {
    if (this._port && this._port.isOpen) {
      try { this._port.close(); } catch {}
    }
    this._port = null;
  }

  send(command) {
    if (!this._port || !this._port.isOpen) return false;
    try {
      this._port.write(command);
      return true;
    } catch (err) {
      this._log.debug?.('[TCode/serial] Write error:', err.message);
      return false;
    }
  }

  get connected() { return !!(this._port && this._port.isOpen); }

  onDisconnect(cb) { this._onDisconnect = cb; }

  destroy() {
    this._onDisconnect = null;
    if (this._port) {
      try { this._port.removeAllListeners(); } catch {}
      try { if (this._port.isOpen) this._port.close(); } catch {}
      this._port = null;
    }
  }
}

class UdpTransport {
  /**
   * UDP transport for 2.4GHz wireless TCode receivers (ESP-NOW bridges,
   * NodeMCU/ESP32 hosts running a UDP→serial loopback firmware, etc.).
   * Connectionless — `connect()` just resolves the remote address and
   * opens the local socket; the first `send()` is when bytes actually go
   * out. Disconnect callback fires only on socket-level errors.
   *
   * @param opts.dgramFactory optional override for tests (returns a
   *   `dgram.Socket` lookalike with .send/.close/.bind/.on).
   */
  constructor({ log = console, dgramFactory } = {}) {
    this._socket = null;
    this._host = null;
    this._port = null;
    this._log = log;
    this._onDisconnect = null;
    this._dgramFactory = dgramFactory || (() => dgram.createSocket('udp4'));
  }

  async connect({ host, port } = {}) {
    if (!host || typeof host !== 'string') {
      return { success: false, error: 'host required' };
    }
    const numericPort = Number(port);
    if (!Number.isInteger(numericPort) || numericPort <= 0 || numericPort > 65535) {
      return { success: false, error: 'port must be 1-65535' };
    }
    // Tear down any prior socket — connection state is host+port; a fresh
    // call replaces the remote.
    await this.disconnect();
    try {
      this._socket = this._dgramFactory();
      this._host = host;
      this._port = numericPort;
      this._socket.on('error', (err) => {
        this._log.warn?.('[TCode/udp] Socket error:', err.message);
        this._fireDisconnect();
      });
      // Bind to an ephemeral local port so the OS doesn't lazy-bind on
      // first send (which can race with rapid teardown).
      return await new Promise((resolve) => {
        this._socket.bind(() => {
          this._log.info?.(`[TCode/udp] Bound, target ${host}:${numericPort}`);
          resolve({ success: true });
        });
      });
    } catch (err) {
      this._log.warn?.('[TCode/udp] Connect error:', err.message);
      this._socket = null;
      this._host = null;
      this._port = null;
      return { success: false, error: err.message };
    }
  }

  async disconnect() {
    if (!this._socket) return;
    const s = this._socket;
    this._socket = null;
    this._host = null;
    this._port = null;
    await new Promise((resolve) => {
      try { s.close(() => resolve()); }
      catch { resolve(); }
    });
  }

  send(command) {
    if (!this._socket || !this._host || !this._port) return false;
    try {
      const buf = Buffer.from(command, 'utf8');
      this._socket.send(buf, 0, buf.length, this._port, this._host, (err) => {
        if (err) this._log.debug?.('[TCode/udp] Send error:', err.message);
      });
      return true;
    } catch (err) {
      this._log.debug?.('[TCode/udp] Send threw:', err.message);
      return false;
    }
  }

  get connected() { return !!this._socket; }

  onDisconnect(cb) { this._onDisconnect = cb; }

  _fireDisconnect() {
    if (this._socket) {
      try { this._socket.close(); } catch {}
      this._socket = null;
      this._host = null;
      this._port = null;
    }
    if (this._onDisconnect) this._onDisconnect();
  }

  destroy() {
    this._onDisconnect = null;
    if (this._socket) {
      try { this._socket.close(); } catch {}
      this._socket = null;
      this._host = null;
      this._port = null;
    }
  }
}

class WebsocketTransport {
  /**
   * WebSocket client transport — connects to a WS server hosted by the
   * TCode device. Send is sync-returning-bool to match the contract;
   * delivery is best-effort (no ack model in the TCode protocol).
   *
   * Keeps trying until the device answers or the user disconnects, from
   * the first connect on: every 5 s for the first 30 minutes the device
   * is unreachable, then every 60 s. Covers a MiraBot that is switched on
   * later, rebooting or leaving Wi-Fi, and MFP consumers like restim
   * restarting mid-video. A first connect that fails resolves with
   * `{ success: false, retrying: true, error }` and the transport stays
   * alive; `onStatus` reports when it gets through.
   *
   * Logging (electron-log → main.log): every connect, drop and reconnect
   * carries the URL, resolved IP, close code, uptime / downtime, commands
   * and bytes sent and the attempt count. A failed attempt is explained by
   * a DNS lookup and a TCP probe of the same host:port (the runtime's
   * WebSocket reports no cause); repeats of the same failure are folded
   * into one line per minute (per 10 minutes once retries slow down).
   *
   * @param opts.WebSocketImpl optional override for tests — must match
   *   the browser WebSocket shape (constructor `(url)`, `.send(data)`,
   *   `.close()`, `.readyState`, `.onopen/.onmessage/.onclose/.onerror`).
   * @param opts.setTimeoutImpl optional override for tests — must match
   *   the `setTimeout(fn, ms) -> id` shape.
   * @param opts.clearTimeoutImpl optional override for tests.
   * @param opts.nowImpl optional clock override for tests (`() => ms`).
   * @param opts.lookupImpl optional DNS override (`host -> {address, family}`).
   * @param opts.probeImpl optional TCP probe override
   *   (`(host, port, timeoutMs) -> {ok, code?, ms}`).
   * @param opts.reconnect (default true) — false: no retries, a failed
   *   connect or a drop is final.
   */
  constructor({
    log = console, WebSocketImpl, setTimeoutImpl, clearTimeoutImpl, reconnect = true,
    nowImpl, lookupImpl, probeImpl,
  } = {}) {
    this._ws = null;
    this._log = log;
    this._onDisconnect = null;
    this._url = null;
    this._target = null;
    this._reconnectEnabled = !!reconnect;
    this._reconnectTimer = null;
    this._reconnectAttempts = 0;
    this._nextDelay = null;
    this._stopped = false;
    this._hasEverConnected = false;
    // Outage bookkeeping: when the open connection dropped, whether the
    // switch to slow retries was logged, and the failure-folding state.
    this._downSince = null;
    this._slowLogged = false;
    this._failLog = null;
    // Per-connection stats for the drop / disconnect lines.
    this._session = null;
    this._lastIp = null;
    // Live state for status(): latest DNS answer, last failure, last drop.
    this._lastResolved = null;
    this._lastError = null;
    this._lastDrop = null;
    this._nextRetryAt = null;
    this._onStatus = null;
    this._setTimeout = setTimeoutImpl || ((typeof setTimeout !== 'undefined') ? setTimeout : null);
    this._clearTimeout = clearTimeoutImpl || ((typeof clearTimeout !== 'undefined') ? clearTimeout : null);
    this._now = nowImpl || Date.now;
    this._lookup = lookupImpl || lookupHost;
    this._probe = probeImpl || probeTcp;
    // Distinguish "caller did not pass WebSocketImpl" (undefined → fall
    // back to global) from "caller explicitly passed null" (no runtime,
    // tested explicitly).
    if (WebSocketImpl === undefined) {
      this._WebSocket = HAS_GLOBAL_WS ? globalThis.WebSocket : null;
    } else {
      this._WebSocket = WebSocketImpl;
    }
  }

  /** Delay before the next reconnect: 5 s for the first 30 min down, then 60 s. */
  _retryDelayMs() {
    const down = this._downSince == null ? 0 : this._now() - this._downSince;
    return down < FAST_WINDOW_MS ? FAST_RETRY_MS : SLOW_RETRY_MS;
  }

  async connect({ url } = {}) {
    if (!this._WebSocket) {
      return { success: false, error: 'WebSocket runtime not available' };
    }
    if (!url || typeof url !== 'string' || !/^wss?:\/\//.test(url)) {
      return { success: false, error: 'url must start with ws:// or wss://' };
    }
    let target;
    try {
      target = parseTarget(url);
    } catch {
      return { success: false, error: `invalid url: ${url}` };
    }
    // User-initiated connect starts from a clean slate; the internal
    // reconnect path (`_openSocket`) keeps the outage state so the retry
    // interval can slow down after 30 minutes.
    this._stopped = false;
    this._reconnectAttempts = 0;
    this._hasEverConnected = false;
    this._downSince = null;
    this._slowLogged = false;
    this._failLog = null;
    this._lastError = null;
    this._lastDrop = null;
    this._clearReconnectTimer();
    this._url = url;
    this._target = target;
    const opening = this._openSocket();
    this._emitStatus();
    return opening;
  }

  _clearReconnectTimer() {
    if (this._reconnectTimer && this._clearTimeout) this._clearTimeout(this._reconnectTimer);
    this._reconnectTimer = null;
  }

  /**
   * Open a fresh WebSocket and resolve when it either opens or fails.
   * Shared between user-initiated `connect()` and the reconnect timer. A
   * DNS lookup of the host runs alongside, for the IP in the log and to
   * explain a failure. Handlers of a socket that is no longer current
   * only settle its promise.
   */
  _openSocket() {
    const url = this._url;
    if (this._ws) {
      const prior = this._ws;
      this._ws = null;
      try { prior.close(); } catch {}
    }
    const startedAt = this._now();
    const diag = this._resolveHost();
    let ws;
    try {
      ws = new this._WebSocket(url);
    } catch (err) {
      this._log.warn?.(`[TCode/ws] Connect error: ${err.message} url=${url}`);
      if (this._inReconnectSession() && this._downSince != null) this._scheduleReconnect();
      return Promise.resolve({ success: false, error: err.message });
    }
    this._ws = ws;
    return new Promise((resolve) => {
      let opened = false;
      let settled = false;
      const settle = (result) => {
        if (settled) return;
        settled = true;
        resolve(result);
      };
      ws.onopen = () => {
        if (this._ws !== ws) return;
        opened = true;
        this._onOpen(startedAt, diag);
        settle({ success: true });
      };
      // undici's ErrorEvent carries no detail and is always followed by a
      // close event; the close handler logs the explained failure.
      ws.onerror = () => {};
      ws.onclose = (ev) => {
        if (this._ws !== ws) {
          settle({ success: false, error: 'connection replaced or closed' });
          return;
        }
        this._ws = null;
        if (opened) {
          this._onDrop(ev);
          return;
        }
        if (this._inReconnectSession()) {
          const attempt = this._reconnectAttempts;
          const initial = this._downSince == null;
          if (initial) this._downSince = this._now();
          this._scheduleReconnect();
          this._explainFailure(diag, ev, startedAt).then((f) => {
            this._lastError = { text: f.text, at: this._now() };
            if (initial) {
              this._failLog = { cause: f.cause, at: this._now(), suppressed: 0 };
              this._log.warn?.(`[TCode/ws] Connect failed: ${f.text} url=${url} ${f.meta}; ${retryPlanText()}`);
              settle({ success: false, retrying: true, error: f.text });
            } else {
              this._logReconnectFailure(attempt, f);
            }
            this._emitStatus();
          });
          if (!initial) settle({ success: false, error: 'reconnect attempt failed' });
          return;
        }
        this._explainFailure(diag, ev, startedAt).then((f) => {
          this._log.warn?.(`[TCode/ws] Connect failed: ${f.text} url=${url} ${f.meta}`);
          settle({ success: false, error: f.text });
        });
      };
    });
  }

  _inReconnectSession() {
    return this._reconnectEnabled && !this._stopped;
  }

  /** DNS lookup of the target host, bounded: `{ ok, address?, family?, code?, ms }`. */
  _resolveHost() {
    const t0 = this._now();
    const { host } = this._target;
    return new Promise((resolve) => {
      let done = false;
      const finish = (result) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        resolve({ ...result, ms: this._now() - t0 });
      };
      const timer = setTimeout(() => finish({ ok: false, code: 'lookup timeout' }), LOOKUP_TIMEOUT_MS);
      timer.unref?.();
      Promise.resolve()
        .then(() => this._lookup(host))
        .then(
          (r) => finish({ ok: true, address: r.address, family: r.family }),
          (err) => finish({ ok: false, code: err?.code || err?.message || 'lookup failed' }),
        );
    });
  }

  /**
   * Why did an attempt fail? `{ cause, text, meta }`: `cause` is the key
   * repeated failures are folded by, `text` the human explanation (also the
   * error a first connect returns), `meta` the numbers.
   */
  async _explainFailure(diag, ev, startedAt) {
    const tookMs = this._now() - startedAt;
    const { host, port } = this._target;
    const close = closeText(ev);
    try {
      const d = await diag;
      this._lastResolved = d.ok ? d.address : null;
      if (!d.ok) {
        return {
          cause: `dns:${d.code}`,
          text: `${host} does not resolve (${d.code}): the device is off or not on this network`,
          meta: `${close} dns=${fmtDuration(d.ms)} took=${fmtDuration(tookMs)}`,
        };
      }
      const where = `${d.address}:${port}`;
      const p = await this._probe(d.address, port, PROBE_TIMEOUT_MS);
      const meta = `ip=${d.address} ${close} dns=${fmtDuration(d.ms)} tcp=${fmtDuration(p.ms)} took=${fmtDuration(tookMs)}`;
      if (p.ok) {
        return {
          cause: `ws:${ev?.code}@${d.address}`,
          text: `TCP ${where} accepts connections but the WebSocket handshake failed`,
          meta,
        };
      }
      return { cause: `tcp:${p.code}@${d.address}`, text: describeTcpFailure(where, p.code), meta };
    } catch (err) {
      return {
        cause: `diag:${err?.message}`,
        text: `connection failed (diagnosis error: ${err?.message})`,
        meta: `${close} took=${fmtDuration(tookMs)}`,
      };
    }
  }

  _onOpen(startedAt, diag) {
    const now = this._now();
    const openMs = now - startedAt;
    const isReconnect = this._hasEverConnected;
    const downMs = this._downSince == null ? null : now - this._downSince;
    const attempts = this._reconnectAttempts;
    const folded = this._failLog?.suppressed || 0;
    this._hasEverConnected = true;
    this._reconnectAttempts = 0;
    this._downSince = null;
    this._nextRetryAt = null;
    this._slowLogged = false;
    this._failLog = null;
    const session = { openedAt: now, ip: null, sent: 0, bytes: 0, lastSendAt: null, sendErrors: 0 };
    this._session = session;
    this._emitStatus();
    diag.then((d) => {
      session.ip = d.ok ? d.address : null;
      let ip = d.ok ? d.address : `? (${d.code})`;
      if (d.ok && this._lastIp && this._lastIp !== d.address) ip += ` (was ${this._lastIp})`;
      if (d.ok) this._lastIp = d.address;
      const foldedText = folded ? ` (${folded} failed attempts not logged)` : '';
      if (isReconnect) {
        this._log.info?.(`[TCode/ws] Reconnected url=${this._url} ip=${ip} down=${fmtDuration(downMs)} attempts=${attempts} open=${fmtDuration(openMs)}${foldedText}`);
      } else if (downMs != null) {
        this._log.info?.(`[TCode/ws] Connected url=${this._url} ip=${ip} after waiting ${fmtDuration(downMs)} attempts=${attempts} open=${fmtDuration(openMs)}${foldedText}`);
      } else {
        this._log.info?.(`[TCode/ws] Connected url=${this._url} ip=${ip} dns=${fmtDuration(d.ms)} open=${fmtDuration(openMs)}`);
      }
      this._emitStatus();
    });
  }

  _sessionText(now) {
    const s = this._session;
    if (!s) return '';
    const lastSend = s.lastSendAt == null ? 'never' : `${fmtDuration(now - s.lastSendAt)} ago`;
    const errors = s.sendErrors ? ` sendErrors=${s.sendErrors}` : '';
    return `ip=${s.ip ?? '?'} up=${fmtDuration(now - s.openedAt)} sent=${s.sent} cmds/${fmtBytes(s.bytes)} lastSend=${lastSend}${errors}`;
  }

  /** The open connection closed without the user asking. */
  _onDrop(ev) {
    const now = this._now();
    const stats = this._sessionText(now);
    this._session = null;
    const line = `[TCode/ws] Connection lost url=${this._url} ${closeText(ev)} ${stats}`;
    this._lastDrop = { at: now, close: closeText(ev) };
    if (this._reconnectEnabled && !this._stopped) {
      this._downSince = now;
      this._log.warn?.(`${line}; ${retryPlanText()}`);
      this._scheduleReconnect();
    } else {
      this._log.warn?.(line);
      if (this._onDisconnect) this._onDisconnect();
    }
    this._emitStatus();
  }

  /** Log a failed reconnect, folding repeats of the same cause. */
  _logReconnectFailure(attempt, f) {
    if (this._stopped || this._downSince == null) return;
    const now = this._now();
    const interval = this._nextDelay === SLOW_RETRY_MS ? SLOW_FAILURE_LOG_MS : FAST_FAILURE_LOG_MS;
    const prev = this._failLog;
    if (prev && prev.cause === f.cause && now - prev.at < interval) {
      prev.suppressed++;
      return;
    }
    const folded = prev?.suppressed ? ` (+${prev.suppressed} similar not logged)` : '';
    this._failLog = { cause: f.cause, at: now, suppressed: 0 };
    this._log.warn?.(`[TCode/ws] Reconnect attempt ${attempt} failed: ${f.text} url=${this._url} ${f.meta} down=${fmtDuration(now - this._downSince)} next=${fmtDuration(this._nextDelay)}${folded}`);
  }

  _scheduleReconnect() {
    if (!this._setTimeout) return;
    const delay = this._retryDelayMs();
    if (delay === SLOW_RETRY_MS && !this._slowLogged) {
      this._slowLogged = true;
      this._log.warn?.(`[TCode/ws] Still down after ${fmtDuration(this._now() - this._downSince)} (${this._reconnectAttempts} attempts) url=${this._url}: retrying every ${fmtDuration(SLOW_RETRY_MS)} from now on`);
    }
    this._reconnectAttempts++;
    this._nextDelay = delay;
    this._nextRetryAt = this._now() + delay;
    this._reconnectTimer = this._setTimeout(() => {
      this._reconnectTimer = null;
      this._nextRetryAt = null;
      if (this._stopped || !this._url) return;
      this._openSocket();
    }, delay);
  }

  /** Log what a user-initiated stop ends: an open connection or an outage. */
  _logStop() {
    const now = this._now();
    if (this._session) {
      this._log.info?.(`[TCode/ws] Closed by user url=${this._url} ${this._sessionText(now)}`);
    } else if (this._downSince != null) {
      this._log.info?.(`[TCode/ws] Reconnecting stopped by user url=${this._url} down=${fmtDuration(now - this._downSince)} attempts=${this._reconnectAttempts}`);
    }
    this._session = null;
    this._downSince = null;
    this._nextRetryAt = null;
  }

  /**
   * Snapshot for the status API and the desktop UI. `state`:
   * `connecting` (first attempt in flight), `connected`, `waiting` (never
   * got through yet, retrying), `reconnecting` (was connected, retrying)
   * or `stopped`.
   */
  status() {
    const now = this._now();
    const iso = (ms) => (ms == null ? null : new Date(ms).toISOString());
    const s = this._session;
    let state;
    if (this._stopped || !this._url) state = 'stopped';
    else if (this.connected) state = 'connected';
    else if (this._downSince != null) state = this._hasEverConnected ? 'reconnecting' : 'waiting';
    else state = 'connecting';
    const retrying = state === 'waiting' || state === 'reconnecting';
    return {
      transport: 'websocket',
      state,
      url: this._url,
      ip: s?.ip ?? this._lastResolved ?? this._lastIp,
      connectedSince: iso(s?.openedAt),
      downSince: iso(this._downSince),
      attempts: this._reconnectAttempts,
      retryEveryMs: retrying ? this._nextDelay : null,
      nextRetryAt: retrying ? iso(this._nextRetryAt) : null,
      lastError: this._lastError ? { text: this._lastError.text, at: iso(this._lastError.at) } : null,
      lastDisconnect: this._lastDrop ? { at: iso(this._lastDrop.at), close: this._lastDrop.close } : null,
      sent: s ? {
        commands: s.sent,
        bytes: s.bytes,
        lastSendAgoMs: s.lastSendAt == null ? null : now - s.lastSendAt,
        errors: s.sendErrors,
      } : null,
    };
  }

  /** Single subscriber, called with `status()` whenever it changes. */
  onStatus(cb) { this._onStatus = cb; }

  _emitStatus() {
    if (!this._onStatus) return;
    try { this._onStatus(this.status()); } catch (err) {
      this._log.debug?.('[TCode/ws] status listener error:', err.message);
    }
  }

  async disconnect() {
    // Mark as user-initiated so the close handler doesn't schedule
    // a reconnect.
    this._stopped = true;
    this._clearReconnectTimer();
    this._logStop();
    this._emitStatus();
    this._reconnectAttempts = 0;
    if (!this._ws) return;
    const ws = this._ws;
    this._ws = null;
    try { ws.close(); } catch {}
  }

  send(command) {
    if (!this._ws) return false;
    // WebSocket.OPEN === 1. Reference via numeric literal so a mocked
    // WebSocketImpl in tests doesn't need to provide the OPEN static.
    if (this._ws.readyState !== 1) return false;
    try {
      this._ws.send(command);
      const s = this._session;
      if (s) {
        s.sent++;
        s.bytes += command.length;
        s.lastSendAt = this._now();
      }
      return true;
    } catch (err) {
      if (this._session) this._session.sendErrors++;
      this._log.debug?.('[TCode/ws] Send error:', err.message);
      return false;
    }
  }

  get connected() { return !!this._ws && this._ws.readyState === 1; }

  /** Was a reconnect scheduled? Exposed for tests. */
  get reconnecting() { return this._reconnectTimer !== null; }

  onDisconnect(cb) { this._onDisconnect = cb; }

  destroy() {
    this._onDisconnect = null;
    this._onStatus = null;
    this._stopped = true;
    this._clearReconnectTimer();
    this._logStop();
    if (this._ws) {
      const ws = this._ws;
      this._ws = null;
      try { ws.close(); } catch {}
    }
  }
}

/**
 * Build a transport instance by name. Throws on unknown name so the IPC
 * caller fails loudly instead of silently doing nothing.
 */
function createTransport(kind, opts = {}) {
  switch (kind) {
    case 'serial': return new SerialTransport(opts);
    case 'udp': return new UdpTransport(opts);
    case 'websocket':
    case 'ws':
      return new WebsocketTransport(opts);
    default:
      throw new Error(`Unknown TCode transport: ${kind}`);
  }
}

module.exports = {
  SerialTransport,
  UdpTransport,
  WebsocketTransport,
  createTransport,
};
