// Device dashboard sample stream (renderer/js/device-telemetry.js): runs
// only while the dashboard's lease lasts, and every command the TCode
// manager sends shows up in exactly one sample (the position chart is
// drawn from them).

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { TelemetryStream, buildDeviceTelemetry } from '../../renderer/js/device-telemetry.js';
import { TCodeManager } from '../../renderer/js/tcode-manager.js';

function fakeClock() {
  let now = 1_000_000;
  const timers = new Map();
  let id = 0;
  return {
    now: () => now,
    advance(ms) { now += ms; },
    setInterval: (fn) => { timers.set(++id, fn); return id; },
    clearInterval: (t) => { timers.delete(t); },
    async fire() { for (const fn of [...timers.values()]) await fn(); },
    get active() { return timers.size; },
  };
}

describe('TelemetryStream lease', () => {
  it('samples right away, keeps going while the lease runs, then stops by itself', async () => {
    const clock = fakeClock();
    const sent = [];
    const stream = new TelemetryStream({
      collect: () => ({}), send: (s) => sent.push(s),
      now: clock.now, setInterval: clock.setInterval, clearInterval: clock.clearInterval,
    });
    stream.extend(3000);
    await Promise.resolve();
    expect(sent).toHaveLength(1);

    clock.advance(1000); await clock.fire();
    clock.advance(1000); await clock.fire();
    expect(sent).toHaveLength(3);

    clock.advance(1000); await clock.fire();
    expect(sent).toHaveLength(3);
    expect(stream.running).toBe(false);
    expect(clock.active).toBe(0);
  });

  it('a renewal before expiry keeps one timer and pushes the end out', async () => {
    const clock = fakeClock();
    const sent = [];
    const stream = new TelemetryStream({
      collect: () => ({}), send: (s) => sent.push(s),
      now: clock.now, setInterval: clock.setInterval, clearInterval: clock.clearInterval,
    });
    stream.extend(2000);
    clock.advance(1500);
    stream.extend(2000);
    expect(clock.active).toBe(1);
    clock.advance(1000); await clock.fire();
    expect(stream.running).toBe(true);
    clock.advance(1500); await clock.fire();
    expect(stream.running).toBe(false);
  });
});

describe('commands per sample', () => {
  let realNow;
  let t;

  beforeEach(() => {
    t = 5_000_000;
    realNow = Date.now;
    Date.now = () => t;
    global.window = { funsync: { tcodeSend: vi.fn(async () => true) } };
  });

  afterEach(() => {
    Date.now = realNow;
    delete global.window;
  });

  it('every command lands in exactly one sample, including one sent on a window edge', async () => {
    const tcode = new TCodeManager();
    tcode._connected = true;
    let tick;
    const samples = [];
    const stream = new TelemetryStream({
      collect: (since, now) => buildDeviceTelemetry({ tcodeManager: tcode, commandsSince: since, now }),
      send: (s) => samples.push(s),
      now: () => t,
      setInterval: (fn) => { tick = fn; return 1; },
      clearInterval: () => {},
    });

    stream.extend(10_000);              // first sample at t, before any command
    await Promise.resolve();
    t += 400; tcode.send('L09999\n');
    t += 600; tcode.send('L00000\n');   // exactly at the next sample's time
    await tick();
    t += 1; tcode.send('L05000\n');
    t += 999; await tick();

    const cmds = samples.flatMap((s) => s.tcode.commands.map((c) => c.cmd));
    expect(cmds).toEqual(['L09999', 'L00000', 'L05000']);
    expect(samples[2].tcode.output).toMatchObject({ commands: 3, failed: 0 });
  });
});
