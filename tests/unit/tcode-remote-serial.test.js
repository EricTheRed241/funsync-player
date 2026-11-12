/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// RemoteSerialTransport (desktop side of the backend's TCode serial relay)
// and the manager's stop() on it. Mocked WebSocket, fake timers/DNS/probe:
// no network, no hardware.

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { createRequire } from 'node:module';
import { TCodeManager } from '../../renderer/js/tcode-manager.js';

const require = createRequire(import.meta.url);
// eslint-disable-next-line import/no-dynamic-require
const { RemoteSerialTransport, createTransport } = require('../../electron/tcode-transports.js');

class MockWebSocket {
  constructor(url) {
    this.url = url;
    this.readyState = 0;
    this.sent = [];
    MockWebSocket.instances.push(this);
  }
  send(data) { this.sent.push(data); }
  close() {
    this.readyState = 3;
    if (this.onclose) this.onclose({ code: 1000 });
  }
  _open() { this.readyState = 1; this.onopen?.({}); }
  _drop() { this.readyState = 3; this.onclose?.({ code: 1006 }); }
  _msg(obj) { this.onmessage?.({ data: JSON.stringify(obj) }); }
}
MockWebSocket.instances = [];

const URL = 'ws://127.0.0.1:5198/api/remote/tcode-relay?role=desktop';
const SINK_OPEN = {
  type: 'sink', attached: true, ip: '192.168.2.50', portOpen: true,
  port: { usbVendorId: 0x1a86, usbProductId: 0x7523 }, baudRate: 115200, since: '2026-11-12T00:00:00Z',
};
const SINK_ATTACHED = { ...SINK_OPEN, portOpen: false, port: null, baudRate: null };
const SINK_NONE = { type: 'sink', attached: false, ip: null, portOpen: false, port: null, baudRate: null, since: null };

function setup() {
  let now = 0;
  let nextId = 1;
  const timers = new Map();
  const lines = [];
  const log = {
    info: (l) => lines.push(l), warn: (l) => lines.push(l), debug: () => {},
  };
  const t = new RemoteSerialTransport({
    log,
    WebSocketImpl: MockWebSocket,
    setTimeoutImpl: (fn, ms) => { const id = nextId++; timers.set(id, { fn, at: now + ms }); return id; },
    clearTimeoutImpl: (id) => { timers.delete(id); },
    nowImpl: () => now,
    lookupImpl: async () => ({ address: '127.0.0.1', family: 4 }),
    probeImpl: async () => ({ ok: false, code: 'ECONNREFUSED', ms: 1 }),
  });
  const states = [];
  t.onStatus((s) => states.push(s));
  /** Advance the fake clock, firing due timers in order. */
  const advance = (ms) => {
    const until = now + ms;
    for (;;) {
      const due = [...timers.entries()].filter(([, v]) => v.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      timers.delete(due[0]);
      now = due[1].at;
      due[1].fn();
    }
    now = until;
  };
  const ws = () => MockWebSocket.instances.at(-1);
  const flush = () => new Promise((r) => setImmediate(r));
  return { t, states, lines, advance, ws, flush, timers };
}

beforeEach(() => { MockWebSocket.instances.length = 0; });

describe('RemoteSerialTransport', () => {
  it('is registered as remote-serial', () => {
    expect(createTransport('remote-serial', { log: {}, WebSocketImpl: MockWebSocket })).toBeInstanceOf(RemoteSerialTransport);
  });

  it('connects only when the relay is open AND the sink port is open', async () => {
    const { t, ws } = setup();
    const p = t.connect({ url: URL, baudRate: 115200 });
    ws()._open();
    expect(ws().sent[0]).toBe(JSON.stringify({ type: 'config', baudRate: 115200 }));
    expect(t.connected).toBe(false);
    expect(t.send('L0500\n')).toBe(false);
    ws()._msg(SINK_OPEN);
    expect(await p).toEqual({ success: true });
    expect(t.connected).toBe(true);
    expect(t.send('L0500\n')).toBe(true);
    expect(ws().sent.at(-1)).toBe('L0500\n');
    expect(t.status()).toMatchObject({ transport: 'remote-serial', state: 'connected', url: URL, relay: 'connected', waitingFor: null });
    t.destroy();
  });

  it('waits for the port to reopen at its baud rate before sending', async () => {
    const { t, ws, advance, flush } = setup();
    const p = t.connect({ url: URL, baudRate: 250000 });
    ws()._open();
    // The tab still has the port open at the previous baud until our config reaches it.
    ws()._msg(SINK_OPEN);
    await flush();
    advance(1000);
    expect(await p).toMatchObject({ success: false, retrying: true });
    expect(t.connected).toBe(false);
    expect(t.send('L0500\n')).toBe(false);
    expect(t.status().waitingFor).toBe('port');
    ws()._msg({ ...SINK_OPEN, baudRate: 250000 });
    expect(t.connected).toBe(true);
    expect(t.send('L0500\n')).toBe(true);
    t.destroy();
  });

  it('answers retrying with what it waits for after ~1 s without a port', async () => {
    const { t, ws, advance, flush } = setup();
    const p = t.connect({ url: URL, baudRate: 115200 });
    ws()._open();
    await flush();
    ws()._msg(SINK_ATTACHED);
    const r = await p;
    expect(r).toMatchObject({ success: false, retrying: true });
    expect(r.error).toMatch(/port/);
    expect(t.status()).toMatchObject({ state: 'waiting', waitingFor: 'port' });

    const { t: t2, ws: ws2, advance: adv2, flush: flush2 } = setup();
    const p2 = t2.connect({ url: URL });
    ws2()._open();
    await flush2();
    adv2(1000);
    const r2 = await p2;
    expect(r2.error).toMatch(/browser/);
    expect(t2.status().waitingFor).toBe('browser');
    advance(0);
    t.destroy();
    t2.destroy();
  });

  it('waits for the backend while the relay is down', async () => {
    const { t, ws, flush } = setup();
    const p = t.connect({ url: URL });
    ws()._drop();
    await flush();
    const r = await p;
    expect(r).toMatchObject({ success: false, retrying: true });
    expect(t.status()).toMatchObject({ state: 'waiting', waitingFor: 'backend' });
    t.destroy();
  });

  it('goes waiting → connected → reconnecting → connected, re-sending config on every relay open', async () => {
    const { t, ws, states, advance, flush } = setup();
    const p = t.connect({ url: URL, baudRate: 115200 });
    ws()._open();
    ws()._msg(SINK_NONE);
    await p;
    expect(t.status()).toMatchObject({ state: 'waiting', waitingFor: 'browser' });
    ws()._msg(SINK_OPEN);
    expect(t.status().state).toBe('connected');

    // Sink detaches.
    ws()._msg(SINK_NONE);
    expect(t.status()).toMatchObject({ state: 'reconnecting', waitingFor: 'browser' });
    expect(t.send('L0500\n')).toBe(false);
    ws()._msg(SINK_OPEN);
    expect(t.status().state).toBe('connected');

    // Port closes.
    ws()._msg(SINK_ATTACHED);
    expect(t.status()).toMatchObject({ state: 'reconnecting', waitingFor: 'port' });
    ws()._msg(SINK_OPEN);
    expect(t.status().state).toBe('connected');

    // Relay drops: sink unknown until the backend reports again.
    const first = ws();
    first._drop();
    expect(t.status()).toMatchObject({ state: 'reconnecting', waitingFor: 'backend', sink: null });
    expect(t.connected).toBe(false);
    advance(5000);
    const second = ws();
    expect(second).not.toBe(first);
    second._open();
    expect(second.sent[0]).toBe(JSON.stringify({ type: 'config', baudRate: 115200 }));
    second._msg(SINK_OPEN);
    expect(t.status().state).toBe('connected');
    await flush();
    const seq = states.map((s) => s.state).filter((s, i, a) => s !== a[i - 1]);
    expect(seq).toEqual(['connecting', 'waiting', 'connected', 'reconnecting', 'connected', 'reconnecting', 'connected', 'reconnecting', 'connected']);
    t.destroy();
  });

  it('asks for device info 1500 ms after the port opens, not if it closed first', async () => {
    const { t, ws, advance } = setup();
    const p = t.connect({ url: URL });
    ws()._open();
    ws()._msg(SINK_OPEN);
    await p;
    advance(1499);
    expect(ws().sent).not.toContain('D1\n');
    advance(1);
    expect(ws().sent.slice(-2)).toEqual(['D1\n', 'D2\n']);

    ws()._msg(SINK_ATTACHED);
    ws()._msg(SINK_OPEN);
    advance(1000);
    ws()._msg(SINK_ATTACHED);
    const before = ws().sent.length;
    advance(5000);
    expect(ws().sent.length).toBe(before);
    t.destroy();
  });

  it('logs attach/detach, port open/close and device replies', async () => {
    const { t, ws, lines } = setup();
    const p = t.connect({ url: URL });
    ws()._open();
    ws()._msg(SINK_OPEN);
    await p;
    ws()._msg({ type: 'reply', text: 'TCode v0.3' });
    ws()._msg(SINK_ATTACHED);
    ws()._msg(SINK_NONE);
    expect(lines).toContain('[TCode/remote-serial] Browser attached ip=192.168.2.50');
    expect(lines.some((l) => l.startsWith('[TCode/remote-serial] Device port opened usb=1a86:7523'))).toBe(true);
    expect(lines).toContain('[TCode/remote-serial] Device: TCode v0.3');
    expect(lines).toContain('[TCode/remote-serial] Device port closed usb=1a86:7523');
    expect(lines).toContain('[TCode/remote-serial] Browser detached ip=192.168.2.50');
    t.destroy();
  });

  it('disconnect() stops reconnects and timers', async () => {
    const { t, ws, timers, states } = setup();
    const p = t.connect({ url: URL });
    ws()._open();
    ws()._msg(SINK_OPEN);
    await p;
    await t.disconnect();
    expect(timers.size).toBe(0);
    expect(t.connected).toBe(false);
    expect(t.status().state).toBe('stopped');
    expect(states.at(-1).state).toBe('stopped');
    expect(MockWebSocket.instances.length).toBe(1);
  });
});

describe('TCodeManager — stop on remote-serial', () => {
  let sent;
  beforeEach(() => {
    sent = [];
    global.window = {
      funsync: {
        tcodeConnect: vi.fn(async () => ({ success: true })),
        tcodeDisconnect: vi.fn(async () => ({ success: true })),
        tcodeSend: vi.fn(async (cmd) => { sent.push(cmd); return true; }),
        onTcodeDisconnected: vi.fn(() => () => {}),
      },
    };
  });
  afterEach(() => { delete global.window; });

  it('sends only DSTOP (no neutral frame on real firmware)', async () => {
    const m = new TCodeManager();
    expect(await m.connect('remote-serial', { baudRate: 115200 })).toBe(true);
    expect(window.funsync.tcodeConnect).toHaveBeenCalledWith('remote-serial', { baudRate: 115200 });
    m.stop();
    expect(sent).toEqual(['DSTOP\n']);
  });
});
