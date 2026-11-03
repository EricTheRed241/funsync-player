// Tests for the renderer-side TCode manager's multi-transport connect()
// signature. The manager always speaks the same protocol (LXXXX RXXXX
// formatter unchanged); only the IPC payload differs by transport.
//
// Existing call sites use the legacy two-arg form
// `connect(portPath, baudRate)` — those tests pin back-compat so the
// connection-panel doesn't regress for users on serial.

import { describe, it, expect, beforeEach, vi, afterEach } from 'vitest';
import { TCodeManager } from '../../renderer/js/tcode-manager.js';

describe('TCodeManager.connect — back-compat legacy serial form', () => {
  let calls;

  beforeEach(() => {
    calls = [];
    global.window = {
      funsync: {
        tcodeConnect: vi.fn(async (kind, opts) => {
          calls.push([kind, opts]);
          return { success: true };
        }),
        onTcodeDisconnected: vi.fn(() => () => {}),
      },
    };
  });

  afterEach(() => { delete global.window; });

  it('two-arg call dispatches as serial', async () => {
    const m = new TCodeManager();
    const ok = await m.connect('COM3', 115200);
    expect(ok).toBe(true);
    expect(calls).toEqual([['serial', { path: 'COM3', baudRate: 115200 }]]);
  });

  it('two-arg call without baud defaults to 115200', async () => {
    const m = new TCodeManager();
    await m.connect('/dev/ttyUSB0');
    expect(calls[0]).toEqual(['serial', { path: '/dev/ttyUSB0', baudRate: 115200 }]);
  });

  it('legacy fields _portPath / _baudRate stay populated for serial', async () => {
    const m = new TCodeManager();
    await m.connect('COM5', 38400);
    expect(m._portPath).toBe('COM5');
    expect(m._baudRate).toBe(38400);
  });
});

describe('TCodeManager.connect — UDP transport', () => {
  let calls;

  beforeEach(() => {
    calls = [];
    global.window = {
      funsync: {
        tcodeConnect: vi.fn(async (kind, opts) => {
          calls.push([kind, opts]);
          return { success: true };
        }),
        onTcodeDisconnected: vi.fn(() => () => {}),
      },
    };
  });
  afterEach(() => { delete global.window; });

  it('passes the host+port options through unchanged', async () => {
    const m = new TCodeManager();
    const ok = await m.connect('udp', { host: '192.168.1.42', port: 8080 });
    expect(ok).toBe(true);
    expect(calls).toEqual([['udp', { host: '192.168.1.42', port: 8080 }]]);
  });

  it('clears legacy serial fields when using UDP', async () => {
    const m = new TCodeManager();
    await m.connect('udp', { host: '1.2.3.4', port: 1000 });
    expect(m._portPath).toBe('');
    expect(m._baudRate).toBe(0);
  });

  it('sets _transportKind to udp', async () => {
    const m = new TCodeManager();
    await m.connect('udp', { host: '1.2.3.4', port: 1000 });
    expect(m._transportKind).toBe('udp');
  });
});

describe('TCodeManager.connect — WebSocket transport', () => {
  let calls;

  beforeEach(() => {
    calls = [];
    global.window = {
      funsync: {
        tcodeConnect: vi.fn(async (kind, opts) => {
          calls.push([kind, opts]);
          return { success: true };
        }),
        onTcodeDisconnected: vi.fn(() => () => {}),
      },
    };
  });
  afterEach(() => { delete global.window; });

  it('accepts kind="websocket" and passes the url', async () => {
    const m = new TCodeManager();
    await m.connect('websocket', { url: 'ws://device.local:81' });
    expect(calls).toEqual([['websocket', { url: 'ws://device.local:81' }]]);
  });

  it('accepts kind="ws" as alias', async () => {
    const m = new TCodeManager();
    await m.connect('ws', { url: 'ws://device.local:81' });
    expect(calls).toEqual([['ws', { url: 'ws://device.local:81' }]]);
  });
});

describe('TCodeManager.connect — failure paths', () => {
  beforeEach(() => {
    global.window = {
      funsync: {
        tcodeConnect: vi.fn(async () => ({ success: false, error: 'EHOSTUNREACH' })),
        onTcodeDisconnected: vi.fn(() => () => {}),
      },
    };
  });
  afterEach(() => { delete global.window; });

  it('returns false when main-process returns success=false', async () => {
    const m = new TCodeManager();
    const ok = await m.connect('udp', { host: '0.0.0.0', port: 80 });
    expect(ok).toBe(false);
    expect(m.connected).toBe(false);
  });

  it('emits error via onError callback', async () => {
    const m = new TCodeManager();
    const errs = [];
    m.onError = (e) => errs.push(e);
    await m.connect('udp', { host: '0.0.0.0', port: 80 });
    expect(errs.length).toBe(1);
    expect(errs[0]).toMatch(/EHOSTUNREACH/);
  });
});

describe('TCodeManager — WebSocket device not answering yet', () => {
  let emitStatus;

  beforeEach(() => {
    emitStatus = null;
    global.window = {
      funsync: {
        tcodeConnect: vi.fn(async () => ({
          success: false, retrying: true, error: 'mirabot.local does not resolve (ENOTFOUND)',
          status: { state: 'waiting', url: 'ws://mirabot.local:8888', attempts: 1 },
        })),
        tcodeDisconnect: vi.fn(async () => ({ success: true })),
        onTcodeDisconnected: vi.fn(() => () => {}),
        onTcodeStatus: vi.fn((cb) => { emitStatus = cb; return () => { emitStatus = null; }; }),
      },
    };
  });
  afterEach(() => { delete global.window; });

  it('waits without an error, then connects when main reports the device', async () => {
    const m = new TCodeManager();
    const errs = [];
    const onConnect = vi.fn();
    m.onError = (e) => errs.push(e);
    m.onConnect = onConnect;

    expect(await m.connect('websocket', { url: 'ws://mirabot.local:8888' })).toBe(false);
    expect(m.waiting).toBe(true);
    expect(m.connected).toBe(false);
    expect(m.status.state).toBe('waiting');
    expect(errs).toEqual([]);

    emitStatus({ state: 'connected', url: 'ws://mirabot.local:8888', ip: '192.168.2.216' });
    expect(m.connected).toBe(true);
    expect(m.waiting).toBe(false);
    expect(onConnect).toHaveBeenCalledTimes(1);
  });

  it('a drop goes back to waiting and fires onDisconnect once', async () => {
    const m = new TCodeManager();
    const onDisconnect = vi.fn();
    m.onDisconnect = onDisconnect;
    await m.connect('websocket', { url: 'ws://mirabot.local:8888' });
    emitStatus({ state: 'connected' });

    emitStatus({ state: 'reconnecting', attempts: 1 });
    emitStatus({ state: 'reconnecting', attempts: 2 });
    expect(m.connected).toBe(false);
    expect(m.waiting).toBe(true);
    expect(m.status.attempts).toBe(2);
    expect(onDisconnect).toHaveBeenCalledTimes(1);
  });

  it('disconnect stops waiting and stops following main', async () => {
    const m = new TCodeManager();
    await m.connect('websocket', { url: 'ws://mirabot.local:8888' });
    await m.disconnect();
    expect(window.funsync.tcodeDisconnect).toHaveBeenCalledTimes(1);
    expect(m.waiting).toBe(false);
    expect(m.status).toBe(null);
    expect(emitStatus).toBe(null);
  });
});

describe('TCodeManager._describeTransport', () => {
  let m;
  beforeEach(() => { m = new TCodeManager(); });

  it('renders serial transport summary', () => {
    expect(m._describeTransport('serial', { path: 'COM3', baudRate: 115200 }))
      .toBe('COM3 @ 115200');
  });

  it('renders UDP transport summary', () => {
    expect(m._describeTransport('udp', { host: '1.2.3.4', port: 8080 }))
      .toBe('1.2.3.4:8080');
  });

  it('renders WebSocket transport summary', () => {
    expect(m._describeTransport('websocket', { url: 'ws://x:81' }))
      .toBe('ws://x:81');
    expect(m._describeTransport('ws', { url: 'ws://x:81' }))
      .toBe('ws://x:81');
  });

  it('falls back to JSON for unknown transports', () => {
    expect(m._describeTransport('zigbee', { mesh: true }))
      .toBe('{"mesh":true}');
  });
});

describe('TCodeManager.connect — disconnect-before-reconnect', () => {
  beforeEach(() => {
    global.window = {
      funsync: {
        tcodeConnect: vi.fn(async () => ({ success: true })),
        tcodeDisconnect: vi.fn(async () => ({ success: true })),
        onTcodeDisconnected: vi.fn(() => () => {}),
      },
    };
  });
  afterEach(() => { delete global.window; });

  it('disconnects before reconnecting if already connected', async () => {
    const m = new TCodeManager();
    await m.connect('serial', { path: 'COM3' });
    expect(m.connected).toBe(true);
    await m.connect('udp', { host: '1.2.3.4', port: 5000 });
    expect(window.funsync.tcodeDisconnect).toHaveBeenCalledTimes(1);
    expect(m._transportKind).toBe('udp');
  });
});
