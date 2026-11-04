// Tests for MFP-protocol compatibility upgrades on the TCode WS transport
// and tcode-manager formatter:
//   1. Precision configurability — 3-digit (TCode-0.2) vs 4-digit (TCode-0.3).
//      Restim's parser is lenient (accepts either), but TCode-0.3-only
//      firmware needs 4-digit; some consumers infer protocol version from
//      digit count.
//   2. Auto-reconnect on the WebSocket transport: every 5 s for the first
//      30 minutes after a drop, then every 60 s. Restim users expect
//      FunSync to follow them through a restart, and a MiraBot that
//      reboots or leaves Wi-Fi should come back by itself. The reconnect is
//      gated on having had an initial successful open, and is cancelled
//      when the user calls disconnect().
//   3. Stop now emits a portable neutral-position frame in addition to the
//      non-standard DSTOP — MFP-protocol consumers ignore DSTOP and need
//      explicit axis values.
//
// Sources (see notes/UPDATES.md):
//   - MFP wire format research → 3- vs 4-digit precision is per-device,
//     restim parser is lenient, no auto-reconnect in MFP itself.
//   - The community ask from the comment thread is "drop-in MFP".

import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { TCodeManager } from '../../renderer/js/tcode-manager.js';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { WebsocketTransport } = require('../../electron/tcode-transports.js');

describe('TCodeManager — precision configurability', () => {
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

  it('defaults to 3-digit precision (TCode-0.2)', async () => {
    const m = new TCodeManager();
    expect(m.precision).toBe(3);
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 50 });
    expect(sent).toEqual(['L0500\n']);
  });

  it('emits 4-digit values after setPrecision(4)', async () => {
    const m = new TCodeManager();
    m.setPrecision(4);
    expect(m.precision).toBe(4);
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 50 });
    expect(sent).toEqual(['L05000\n']);
  });

  it('emits maximum-value at 100% (3-digit)', async () => {
    const m = new TCodeManager();
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 100 });
    expect(sent).toEqual(['L0999\n']);
  });

  it('emits maximum-value at 100% (4-digit)', async () => {
    const m = new TCodeManager();
    m.setPrecision(4);
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 100 });
    expect(sent).toEqual(['L09999\n']);
  });

  it('emits zero-padded minimum at 0% (3-digit)', async () => {
    const m = new TCodeManager();
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 0 });
    expect(sent).toEqual(['L0000\n']);
  });

  it('emits zero-padded minimum at 0% (4-digit)', async () => {
    const m = new TCodeManager();
    m.setPrecision(4);
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 0 });
    expect(sent).toEqual(['L00000\n']);
  });

  it('combines multi-axis with consistent precision', async () => {
    const m = new TCodeManager();
    m.setPrecision(4);
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 50, R0: 75, V0: 30 });
    // L0 = 0.50 * 9999 = 4999.5 → 5000 (rounded)
    // R0 = 0.75 * 9999 = 7499.25 → 7499 (rounded)
    // V0 = 0.30 * 9999 = 2999.7 → 3000 (rounded)
    expect(sent).toEqual(['L05000 R07499 V03000\n']);
  });

  it('appends I-suffix duration regardless of precision', async () => {
    const m = new TCodeManager();
    m.setPrecision(4);
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 50 }, 100);
    expect(sent).toEqual(['L05000I100\n']);
  });

  it('setPrecision(unknown) is a no-op', () => {
    const m = new TCodeManager();
    m.setPrecision(99);
    expect(m.precision).toBe(3);
    m.setPrecision('abc');
    expect(m.precision).toBe(3);
  });

  it('setPrecision(3) reverts after a 4-digit upgrade', async () => {
    const m = new TCodeManager();
    m.setPrecision(4);
    m.setPrecision(3);
    await m.connect('serial', { path: 'COM3' });
    m.sendAxes({ L0: 50 });
    expect(sent).toEqual(['L0500\n']);
  });
});

describe('TCodeManager — stop emits portable neutral frame', () => {
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

  it('sends DSTOP for legacy firmware AND a rest frame for MFP consumers', async () => {
    // The rest frame is for MFP-protocol consumers (restim/Howl) over ws/udp
    // that don't understand DSTOP. Serial firmware halts in place on DSTOP and
    // must NOT get the (violent) recenter frame — see the serial-only test in
    // tcode.test.js.
    const m = new TCodeManager();
    await m.connect('websocket', { url: 'ws://restim.local:81' });
    m.stop();
    expect(sent[0]).toBe('DSTOP\n');
    expect(sent[1]).toMatch(/^L0500 L1500 L2500 R0500 R1500 R2500 V0000 V1000 A0000 A1000 A2000\n$/);
  });

  // A stop must actually stop. Centring a position axis is right; "centring" a
  // vibration axis parks it at half power, which is not a stop at all.
  it('centres position axes but zeroes intensity axes on stop', async () => {
    const m = new TCodeManager();
    await m.connect('websocket', { url: 'ws://restim.local:81' });
    m.stop();
    const frame = sent[1].trim().split(' ');
    const value = (axis) => frame.find((f) => f.startsWith(axis)).slice(axis.length);
    for (const axis of ['L0', 'L1', 'L2', 'R0', 'R1', 'R2']) {
      expect(value(axis), `${axis} should centre`).toBe('500');
    }
    for (const axis of ['V0', 'V1', 'A0', 'A1', 'A2']) {
      expect(value(axis), `${axis} should be off, not half power`).toBe('000');
    }
  });

  // Regression: the frame used to zero A1/A2 (which nothing drove) while
  // skipping V1/V2 (which were driven), so those axes held their last value.
  it('covers every channel the axis table can drive', async () => {
    const { AXIS_DEFINITIONS } = await import('../../renderer/js/multi-axis.js');
    const m = new TCodeManager();
    await m.connect('websocket', { url: 'ws://restim.local:81' });
    m.stop();
    const frame = sent[1];
    for (const { tcode, suffix } of AXIS_DEFINITIONS) {
      expect(frame, `${suffix} (${tcode}) must be reset on stop`).toContain(tcode);
    }
  });

  it('scales the rest frame to 4-digit precision when configured', async () => {
    const m = new TCodeManager();
    m.setPrecision(4);
    await m.connect('websocket', { url: 'ws://restim.local:81' });
    m.stop();
    expect(sent[1]).toMatch(/^L05000 .* A20000\n$/);
  });

  it('stop is a no-op when not connected', () => {
    const m = new TCodeManager();
    m.stop();
    expect(sent).toEqual([]);
  });
});

// --- WebsocketTransport: auto-reconnect -----------------------------------

class MockWebSocket {
  constructor(url) {
    this.url = url;
    this.readyState = 0;
    this.sent = [];
    this.onopen = null;
    this.onclose = null;
    this.onerror = null;
    MockWebSocket.instances.push(this);
  }
  send(d) { this.sent.push(d); }
  close() {
    this.readyState = 3;
    if (this.onclose) this.onclose({ code: 1000 });
  }
  _open() { this.readyState = 1; if (this.onopen) this.onopen({}); }
  _serverDrop() {
    // Remote closed the connection (restim restart, MiraBot off Wi-Fi), or
    // a reconnect attempt that never opened.
    this.readyState = 3;
    if (this.onclose) this.onclose({ code: 1006, reason: '', wasClean: false });
  }
}
MockWebSocket.instances = [];

beforeEach(() => { MockWebSocket.instances.length = 0; });

const MIN = 60 * 1000;
const flush = () => new Promise((r) => setTimeout(r, 0));

describe('WebsocketTransport — auto-reconnect', () => {
  // Fake timers, clock and network so each test drives the transport by hand.
  function setup(extra = {}) {
    const timers = [];
    let nextId = 1;
    const setT = (fn, ms) => { const id = nextId++; timers.push({ id, fn, ms }); return id; };
    const clearT = (id) => { const idx = timers.findIndex((x) => x.id === id); if (idx >= 0) timers.splice(idx, 1); };
    const clock = { now: 1_000_000 };
    const lines = [];
    const net = { probe: { ok: false, code: 'ECONNREFUSED', ms: 2 } };
    const t = new WebsocketTransport({
      log: {
        info: (m) => lines.push(['info', m]),
        warn: (m) => lines.push(['warn', m]),
      },
      WebSocketImpl: MockWebSocket,
      setTimeoutImpl: setT,
      clearTimeoutImpl: clearT,
      nowImpl: () => clock.now,
      lookupImpl: async () => ({ address: '10.0.0.5', family: 4 }),
      probeImpl: async () => net.probe,
      ...extra,
    });
    const latest = () => MockWebSocket.instances[MockWebSocket.instances.length - 1];
    // Fire the pending reconnect timer; the attempt it opens fails pre-open.
    const fireAndFail = () => {
      const tm = timers.shift();
      tm.fn();
      latest()._serverDrop();
    };
    const connectAndOpen = async () => {
      const p = t.connect({ url: 'ws://mirabot.local:8888' });
      latest()._open();
      await p;
    };
    return { t, timers, clock, lines, net, latest, fireAndFail, connectAndOpen };
  }

  it('schedules a 5 s reconnect after a server-side close', async () => {
    const { t, timers, latest, connectAndOpen } = setup();
    await connectAndOpen();
    expect(t.connected).toBe(true);

    latest()._serverDrop();
    expect(t.connected).toBe(false);
    expect(t.reconnecting).toBe(true);
    expect(timers.map((x) => x.ms)).toEqual([5000]);
  });

  it('keeps retrying every 5 s for the first 30 minutes down, then every 60 s', async () => {
    const { timers, clock, latest, fireAndFail, connectAndOpen } = setup();
    await connectAndOpen();
    const droppedAt = clock.now;
    latest()._serverDrop();

    clock.now = droppedAt + 5000;
    fireAndFail();
    expect(timers[0].ms).toBe(5000);

    clock.now = droppedAt + 30 * MIN - 1;
    fireAndFail();
    expect(timers[0].ms).toBe(5000);

    clock.now = droppedAt + 30 * MIN;
    fireAndFail();
    expect(timers[0].ms).toBe(60000);

    clock.now = droppedAt + 5 * 60 * MIN;
    fireAndFail();
    expect(timers[0].ms).toBe(60000);
  });

  it('a successful reconnect restarts the 30-minute fast window', async () => {
    const { timers, clock, latest, fireAndFail, connectAndOpen } = setup();
    await connectAndOpen();
    latest()._serverDrop();
    clock.now += 40 * MIN;
    fireAndFail();
    expect(timers[0].ms).toBe(60000);

    // The next attempt opens, then the device drops again.
    timers.shift().fn();
    latest()._open();
    clock.now += 10 * MIN;
    latest()._serverDrop();
    expect(timers.map((x) => x.ms)).toEqual([5000]);
  });

  it('user-initiated disconnect cancels the pending reconnect', async () => {
    const { t, timers, latest, connectAndOpen } = setup();
    await connectAndOpen();
    latest()._serverDrop();
    expect(t.reconnecting).toBe(true);

    await t.disconnect();
    expect(t.reconnecting).toBe(false);
    expect(timers.length).toBe(0);
  });

  it('reconnect disabled (option = false) leaves the socket closed without rescheduling', async () => {
    const { t, timers, latest, connectAndOpen } = setup({ reconnect: false });
    const onDisc = vi.fn();
    t.onDisconnect(onDisc);
    await connectAndOpen();
    latest()._serverDrop();
    expect(t.reconnecting).toBe(false);
    expect(timers.length).toBe(0);
    expect(onDisc).toHaveBeenCalledTimes(1);
  });

  it('a brand-new connect() call resets the stopped flag', async () => {
    const { t, latest, connectAndOpen } = setup();
    await connectAndOpen();
    await t.disconnect();
    expect(t.reconnecting).toBe(false);

    await connectAndOpen();
    latest()._serverDrop();
    expect(t.reconnecting).toBe(true);
  });

  it('logs a repeated failure once a minute with a count, and a new cause at once', async () => {
    const { clock, lines, net, latest, fireAndFail, connectAndOpen } = setup();
    await connectAndOpen();
    latest()._serverDrop();
    const failures = () => lines.filter(([, m]) => m.includes('Reconnect attempt'));

    // Attempts 1–13, 5 s apart: 1 is logged, 2–12 fall inside its minute.
    for (let i = 1; i <= 13; i++) {
      clock.now += 5000;
      fireAndFail();
      await flush();
    }
    expect(failures()).toHaveLength(2);
    expect(failures()[0][1]).toMatch(/attempt 1 failed: 10\.0\.0\.5:8888 refused the connection \(ECONNREFUSED\)/);
    expect(failures()[1][1]).toMatch(/attempt 13 failed: .*\(\+11 similar not logged\)/);

    net.probe = { ok: false, code: 'ETIMEDOUT', ms: 3000 };
    clock.now += 5000;
    fireAndFail();
    await flush();
    expect(failures()).toHaveLength(3);
    expect(failures()[2][1]).toMatch(/attempt 14 failed: no answer from 10\.0\.0\.5:8888 .*\(ETIMEDOUT\)/);
  });

  it('explains a host that stops resolving', async () => {
    let resolves = true;
    const { clock, lines, latest, fireAndFail, connectAndOpen } = setup({
      lookupImpl: async () => {
        if (resolves) return { address: '10.0.0.5', family: 4 };
        throw Object.assign(new Error('getaddrinfo ENOTFOUND mirabot.local'), { code: 'ENOTFOUND' });
      },
    });
    await connectAndOpen();
    latest()._serverDrop();
    resolves = false;
    clock.now += 5000;
    fireAndFail();
    await flush();
    const line = lines.find(([, m]) => m.includes('Reconnect attempt 1'))[1];
    expect(line).toMatch(/mirabot\.local does not resolve \(ENOTFOUND\)/);
  });

  it('logs the dropped connection and the recovery with their numbers', async () => {
    const { t, timers, clock, lines, latest, fireAndFail, connectAndOpen } = setup();
    await connectAndOpen();
    await flush();
    t.send('L05000\n');
    t.send('R05000\n');
    clock.now += 90 * 1000;
    latest()._serverDrop();
    const lost = lines.find(([, m]) => m.includes('Connection lost'));
    expect(lost[0]).toBe('warn');
    expect(lost[1]).toMatch(/close=1006 .*ip=10\.0\.0\.5 up=1m30s sent=2 cmds\/14B/);

    clock.now += 5000;
    fireAndFail();
    await flush();
    clock.now += 5000;
    timers.shift().fn();
    latest()._open();
    await flush();
    const back = lines.find(([, m]) => m.includes('Reconnected'));
    expect(back[1]).toMatch(/ip=10\.0\.0\.5 down=10\.0s attempts=2/);
  });

  it('reports each link change through onStatus', async () => {
    const { t, clock, latest, fireAndFail, connectAndOpen } = setup();
    const states = [];
    t.onStatus((s) => states.push(s));
    await connectAndOpen();
    await flush();
    expect(t.status()).toMatchObject({ state: 'connected', ip: '10.0.0.5', downSince: null });

    const droppedAt = clock.now;
    latest()._serverDrop();
    expect(t.status()).toMatchObject({
      state: 'reconnecting',
      downSince: new Date(droppedAt).toISOString(),
      attempts: 1,
      retryEveryMs: 5000,
      nextRetryAt: new Date(droppedAt + 5000).toISOString(),
    });
    expect(t.status().lastDisconnect.close).toMatch(/^close=1006/);

    clock.now += 5000;
    fireAndFail();
    await flush();
    expect(states.at(-1)).toMatchObject({ state: 'reconnecting', attempts: 2 });
    expect(states.at(-1).lastError.text).toMatch(/ECONNREFUSED/);

    await t.disconnect();
    expect(states.at(-1).state).toBe('stopped');
  });
});
