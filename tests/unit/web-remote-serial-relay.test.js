/**
 * @vitest-environment node
 */
// SerialRelaySink (backend/web-remote/serial-relay.js): the web remote's end
// of the TCode browser serial relay, driven with fake Web Serial and
// WebSocket objects (no browser, network or hardware).

import { describe, it, expect, beforeEach } from 'vitest';
import {
  SerialRelaySink, STORAGE_KEY, DEFAULT_BAUD, RECONNECT_MS, relayUrl, looksLikeDesktopChromium,
} from '../../backend/web-remote/serial-relay.js';

class Emitter {
  constructor() { this._l = {}; }
  addEventListener(name, fn) { (this._l[name] ||= []).push(fn); }
  fire(name, evt = {}) { for (const fn of this._l[name] || []) fn(evt); }
}

class FakeSocket extends Emitter {
  static instances = [];
  constructor(url) {
    super();
    this.url = url;
    this.sent = [];
    this.closed = false;
    FakeSocket.instances.push(this);
  }
  send(data) { this.sent.push(JSON.parse(data)); }
  close() { if (this.closed) return; this.closed = true; this.fire('close'); }
  // test helpers
  open() { this.fire('open'); }
  receive(data) { this.fire('message', { data }); }
  drop() { this.closed = true; this.fire('close'); }
}

class FakePort {
  constructor(info = { usbVendorId: 0x1a86, usbProductId: 0x7523 }) {
    this.info = info;
    this.opens = [];
    this.closes = 0;
    this.written = [];
    this.readable = null;
    this.writable = null;
    this.failWrites = false;
  }
  getInfo() { return { ...this.info }; }
  async open({ baudRate }) {
    if (this.readable) throw new Error('already open');
    this.opens.push(baudRate);
    const port = this;
    let pull = null;
    const chunks = [];
    this._push = (chunk) => { if (pull) { const p = pull; pull = null; p({ value: chunk, done: false }); } else chunks.push(chunk); };
    this._end = (err) => {
      port.readable = null;
      if (pull) { const p = pull; pull = null; err ? p(Promise.reject(err)) : p({ value: undefined, done: true }); }
    };
    this.readable = {
      getReader() {
        return {
          read() {
            if (chunks.length) return Promise.resolve({ value: chunks.shift(), done: false });
            return new Promise((resolve) => { pull = resolve; });
          },
          async cancel() { if (pull) { const p = pull; pull = null; p({ value: undefined, done: true }); } },
          releaseLock() {},
        };
      },
    };
    this.writable = {
      getWriter() {
        return {
          write(bytes) {
            if (port.failWrites) return Promise.reject(new Error('write failed'));
            port.written.push(new TextDecoder().decode(bytes));
            return Promise.resolve();
          },
          async abort() {},
          releaseLock() {},
        };
      },
    };
  }
  async close() { this.closes++; this.readable = null; this.writable = null; }
  // test helpers
  output(text) { this._push(new TextEncoder().encode(text)); }
  unplug() { this._end(new Error('device lost')); }
}

class FakeSerial extends Emitter {
  constructor() { super(); this.granted = []; this.next = null; this.requests = 0; }
  async requestPort() { this.requests++; if (!this.next) throw Object.assign(new Error('none'), { name: 'NotFoundError' }); this.granted.push(this.next); return this.next; }
  async getPorts() { return [...this.granted]; }
}

function memoryStorage(initial) {
  const map = new Map(initial ? [[STORAGE_KEY, JSON.stringify(initial)]] : []);
  return { getItem: (k) => (map.has(k) ? map.get(k) : null), setItem: (k, v) => map.set(k, v), saved: () => JSON.parse(map.get(STORAGE_KEY)) };
}

const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); await new Promise((r) => setTimeout(r, 0)); };

function makeSink({ protocol = 'http:', stored, serial = new FakeSerial() } = {}) {
  const storage = memoryStorage(stored);
  const timers = [];
  const states = [];
  const sink = new SerialRelaySink({
    serial,
    WebSocketImpl: FakeSocket,
    location: { protocol, host: '192.168.2.220:5123' },
    storage,
    // Like the browser's timers: called as a method of another object they
    // throw "Illegal invocation" (window.setTimeout stored on the sink).
    setTimeout: function fakeSetTimeout(fn, ms) {
      if (this !== undefined && this !== globalThis) throw new TypeError('Illegal invocation');
      const t = { fn, ms }; timers.push(t); return t;
    },
    clearTimeout: function fakeClearTimeout(t) {
      if (this !== undefined && this !== globalThis) throw new TypeError('Illegal invocation');
      const i = timers.indexOf(t); if (i >= 0) timers.splice(i, 1);
    },
    onState: (s) => states.push(s),
  });
  return { sink, serial, storage, timers, states };
}

const lastSocket = () => FakeSocket.instances[FakeSocket.instances.length - 1];

beforeEach(() => { FakeSocket.instances = []; });

describe('support helpers', () => {
  it('reports support, insecure and unsupported', () => {
    expect(SerialRelaySink.support({ serial: {}, isSecureContext: true })).toEqual({ supported: true, reason: null });
    expect(SerialRelaySink.support({ serial: undefined, isSecureContext: false }).reason).toBe('insecure');
    expect(SerialRelaySink.support({ serial: undefined, isSecureContext: true }).reason).toBe('unsupported');
  });

  it('builds the relay URL from the page scheme', () => {
    expect(relayUrl({ protocol: 'https:', host: 'h:1' })).toBe('wss://h:1/api/remote/tcode-relay?role=sink');
    expect(relayUrl({ protocol: 'http:', host: 'h:1' })).toBe('ws://h:1/api/remote/tcode-relay?role=sink');
  });

  it('recognises desktop Chromium only', () => {
    expect(looksLikeDesktopChromium('Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 Chrome/150.0 Safari/537.36')).toBe(true);
    expect(looksLikeDesktopChromium('Mozilla/5.0 (Windows NT 10.0) Chrome/150.0 Safari/537.36 Edg/150.0')).toBe(true);
    expect(looksLikeDesktopChromium('Mozilla/5.0 (Linux; Android 14) Chrome/150.0 Mobile Safari/537.36')).toBe(false);
    expect(looksLikeDesktopChromium('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0) Version/18.0 Mobile/15E148 Safari/604.1')).toBe(false);
  });
});

describe('connect()', () => {
  it('opens the port at the stored baud and attaches with wss on https', async () => {
    const { sink, serial, storage } = makeSink({ protocol: 'https:', stored: { baudRate: 921600 } });
    const port = new FakePort();
    serial.next = port;
    await sink.connect();
    expect(port.opens).toEqual([921600]);
    expect(lastSocket().url).toBe('wss://192.168.2.220:5123/api/remote/tcode-relay?role=sink');
    expect(storage.saved()).toEqual({ device: { usbVendorId: 0x1a86, usbProductId: 0x7523 }, enabled: true, baudRate: 921600 });
    lastSocket().open();
    expect(lastSocket().sent).toEqual([
      { type: 'port', open: true, info: { usbVendorId: 0x1a86, usbProductId: 0x7523 }, baudRate: 921600 },
    ]);
    expect(sink.state).toMatchObject({ relay: 'open', port: 'open', baudRate: 921600, kicked: false });
  });

  it('uses ws on http and the default baud when none is stored', async () => {
    const { sink, serial } = makeSink();
    const port = new FakePort();
    serial.next = port;
    await sink.connect();
    expect(port.opens).toEqual([DEFAULT_BAUD]);
    expect(lastSocket().url.startsWith('ws://192.168.2.220:5123/')).toBe(true);
  });

  it('does nothing when the picker is cancelled', async () => {
    const { sink, serial } = makeSink();
    expect(await sink.connect()).toBe(false);
    expect(serial.requests).toBe(1);
    expect(FakeSocket.instances).toHaveLength(0);
    expect(sink.state.error).toBe(null);
  });
});

describe('relay traffic', () => {
  async function connected(opts) {
    const ctx = makeSink(opts);
    const port = new FakePort();
    ctx.serial.next = port;
    await ctx.sink.connect();
    const ws = lastSocket();
    ws.open();
    ws.sent.length = 0;
    return { ...ctx, port, ws };
  }

  it('writes raw frames verbatim and in order; JSON frames are not written', async () => {
    const { port, ws } = await connected();
    ws.receive('L05000I16 R05000I16\n');
    ws.receive('L09999I20\n');
    ws.receive('{"type":"unknown"}');
    ws.receive('L00000I16\n');
    await flush();
    expect(port.written).toEqual(['L05000I16 R05000I16\n', 'L09999I20\n', 'L00000I16\n']);
  });

  it('reopens the port on a config with a new baud and remembers it', async () => {
    const { port, ws, storage, sink } = await connected();
    ws.receive(JSON.stringify({ type: 'config', baudRate: 115200 }));
    await flush();
    expect(port.opens).toEqual([115200]);
    ws.receive(JSON.stringify({ type: 'config', baudRate: 250000 }));
    await flush();
    expect(port.opens).toEqual([115200, 250000]);
    expect(port.closes).toBe(1);
    expect(storage.saved().baudRate).toBe(250000);
    expect(sink.state.baudRate).toBe(250000);
    expect(ws.sent.filter((m) => m.type === 'port')).toEqual([
      { type: 'port', open: false, info: { usbVendorId: 0x1a86, usbProductId: 0x7523 }, baudRate: null },
      { type: 'port', open: true, info: { usbVendorId: 0x1a86, usbProductId: 0x7523 }, baudRate: 250000 },
    ]);
    ws.receive('L05000\n');
    await flush();
    expect(port.written).toEqual(['L05000\n']);
  });

  it('splits device output into trimmed reply lines', async () => {
    const { port, ws } = await connected();
    port.output('TCode v0.3\r\nOSR2 ');
    port.output('firmware\n\n  \n');
    port.output(`${'x'.repeat(300)}\npartial`);
    await flush();
    const replies = ws.sent.filter((m) => m.type === 'reply').map((m) => m.text);
    expect(replies).toEqual(['TCode v0.3', 'OSR2 firmware', 'x'.repeat(256)]);
  });

  it('closes and reports the port when a write fails', async () => {
    const { port, ws, sink } = await connected();
    port.failWrites = true;
    ws.receive('L05000\n');
    await flush();
    expect(sink.state.port).toBe('closed');
    expect(sink.state.error).toBe('write failed');
    expect(port.closes).toBe(1);
    expect(ws.sent.at(-1)).toMatchObject({ type: 'port', open: false });
  });

  it('reports the port closed when the device goes away', async () => {
    const { port, ws, sink } = await connected();
    port.unplug();
    await flush();
    expect(sink.state.port).toBe('closed');
    expect(ws.sent.at(-1)).toMatchObject({ type: 'port', open: false, baudRate: null });
  });
});

describe('auto-recover', () => {
  const remembered = { device: { usbVendorId: 0x1a86, usbProductId: 0x7523 }, enabled: true, baudRate: 115200 };

  it('resume() reopens the remembered port without requestPort', async () => {
    const serial = new FakeSerial();
    const other = new FakePort({ usbVendorId: 0x2341, usbProductId: 0x0043 });
    const mine = new FakePort();
    serial.granted.push(other, mine);
    const { sink } = makeSink({ stored: remembered, serial });
    await sink.resume();
    expect(serial.requests).toBe(0);
    expect(mine.opens).toEqual([115200]);
    expect(other.opens).toEqual([]);
    expect(FakeSocket.instances).toHaveLength(1);
    expect(sink.state.port).toBe('open');
  });

  it('resume() falls back to the only granted port, and does nothing when not enabled', async () => {
    const serial = new FakeSerial();
    const only = new FakePort({ usbVendorId: 0x10c4, usbProductId: 0xea60 });
    serial.granted.push(only);
    const { sink } = makeSink({ stored: remembered, serial });
    await sink.resume();
    expect(only.opens).toEqual([115200]);

    const serial2 = new FakeSerial();
    const port2 = new FakePort();
    serial2.granted.push(port2);
    const off = makeSink({ stored: { ...remembered, enabled: false }, serial: serial2 });
    await off.sink.resume();
    expect(port2.opens).toEqual([]);
    expect(FakeSocket.instances).toHaveLength(1);
  });

  it('marks the port closed on a USB disconnect and reopens it on connect', async () => {
    const serial = new FakeSerial();
    const port = new FakePort();
    serial.granted.push(port);
    const { sink } = makeSink({ stored: remembered, serial });
    await sink.resume();
    lastSocket().open();
    serial.fire('disconnect', { target: port });
    await flush();
    expect(sink.state.port).toBe('closed');
    const replugged = new FakePort();
    serial.fire('connect', { target: replugged });
    await flush();
    expect(replugged.opens).toEqual([115200]);
    expect(sink.state.port).toBe('open');
    expect(lastSocket().sent.at(-1)).toMatchObject({ type: 'port', open: true });
  });

  it('ignores a USB connect of another device', async () => {
    const serial = new FakeSerial();
    const port = new FakePort();
    serial.granted.push(port);
    const { sink } = makeSink({ stored: remembered, serial });
    await sink.resume();
    serial.fire('disconnect', { target: port });
    await flush();
    const stranger = new FakePort({ usbVendorId: 0x2341, usbProductId: 0x0043 });
    serial.fire('connect', { target: stranger });
    await flush();
    expect(stranger.opens).toEqual([]);
  });

  it('reconnects the relay 3 s after a drop', async () => {
    const { sink, serial, timers } = makeSink();
    serial.next = new FakePort();
    await sink.connect();
    lastSocket().open();
    lastSocket().drop();
    expect(sink.state.relay).toBe('off');
    expect(timers.map((t) => t.ms)).toEqual([RECONNECT_MS]);
    timers.shift().fn();
    expect(FakeSocket.instances).toHaveLength(2);
    lastSocket().open();
    expect(sink.state.relay).toBe('open');
    expect(lastSocket().sent).toEqual([expect.objectContaining({ type: 'port', open: true })]);
  });

  it('stops reconnecting when kicked and releases the device', async () => {
    const { sink, serial, timers, storage } = makeSink();
    const port = new FakePort();
    serial.next = port;
    await sink.connect();
    const ws = lastSocket();
    ws.open();
    ws.receive('{"type":"kicked"}');
    ws.drop();
    await flush();
    expect(timers).toHaveLength(0);
    expect(sink.state).toMatchObject({ kicked: true, relay: 'off', port: 'closed' });
    expect(port.closes).toBe(1);
    expect(storage.saved().enabled).toBe(false);
    // A click takes it back.
    serial.next = port;
    await sink.connect();
    expect(sink.state.kicked).toBe(false);
    expect(FakeSocket.instances).toHaveLength(2);
  });
});

describe('disconnect()', () => {
  it('closes port and socket, clears enabled and keeps the device', async () => {
    const { sink, serial, storage, timers } = makeSink();
    const port = new FakePort();
    serial.next = port;
    await sink.connect();
    const ws = lastSocket();
    ws.open();
    await sink.disconnect();
    expect(port.closes).toBe(1);
    expect(ws.closed).toBe(true);
    expect(ws.sent.at(-1)).toMatchObject({ type: 'port', open: false });
    expect(timers).toHaveLength(0);
    expect(storage.saved()).toEqual({ device: { usbVendorId: 0x1a86, usbProductId: 0x7523 }, enabled: false, baudRate: DEFAULT_BAUD });
    expect(sink.state).toMatchObject({ relay: 'off', port: 'closed' });
  });
});
