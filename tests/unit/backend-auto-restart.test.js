/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The app restarts a backend that went down by itself (electron/python-bridge.js
// createAutoRestart): after 5 s, 30 s, 2 min, then at most once per 10 min;
// never once the app quits. Fake timers and a fake restart: no processes.
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const log = require('../../electron/logger.js');
log.transports.file.level = false;
log.transports.console.level = false;
const { autoRestartDelay, createAutoRestart } = require('../../electron/python-bridge.js');

const S = 1000;
const MIN = 60 * S;

/** A schedule whose restarts leave the backend down (or up, with `heals`). */
function setup({ heals = () => false } = {}) {
  const lines = [];
  const logger = { info: (m) => lines.push(m), warn: (m) => lines.push(m) };
  let state = 'down';
  let stopped = false;
  const restarts = [];
  const auto = createAutoRestart({
    restart: async () => {
      restarts.push(Date.now());
      state = heals(restarts.length) ? 'running' : 'down';
      if (state === 'running') auto.onState('running');
    },
    isStopped: () => stopped,
    getState: () => state,
    logger,
  });
  return { auto, restarts, lines, stop: () => { stopped = true; }, setState: (s) => { state = s; auto.onState(s); } };
}

beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(0); });
afterEach(() => { vi.useRealTimers(); });

describe('autoRestartDelay', () => {
  it('is 5 s, 30 s, 2 min, then 10 min', () => {
    expect([0, 1, 2, 3, 4, 20].map(autoRestartDelay)).toEqual([5 * S, 30 * S, 2 * MIN, 10 * MIN, 10 * MIN, 10 * MIN]);
  });
});

describe('createAutoRestart', () => {
  it('backs off while the backend stays down', async () => {
    const t = setup();
    t.auto.onState('down');
    await vi.advanceTimersByTimeAsync(5 * S - 1);
    expect(t.restarts).toEqual([]);
    await vi.advanceTimersByTimeAsync(1);
    expect(t.restarts).toEqual([5 * S]);
    // Each failed restart schedules the next, further out.
    await vi.advanceTimersByTimeAsync(30 * S);
    await vi.advanceTimersByTimeAsync(2 * MIN);
    await vi.advanceTimersByTimeAsync(10 * MIN);
    await vi.advanceTimersByTimeAsync(10 * MIN);
    expect(t.restarts).toEqual([5 * S, 35 * S, 155 * S, 155 * S + 10 * MIN, 155 * S + 20 * MIN]);
    expect(t.auto.attempts).toBe(5);
    expect(t.lines).toContain('[Backend] auto-restart in 5 s (attempt 1)');
    expect(t.lines).toContain('[Backend] auto-restart attempt 1');
    expect(t.lines).toContain('[Backend] auto-restart in 600 s (attempt 4)');
  });

  it('stops once /health answers, and repeated down events schedule one restart', async () => {
    const t = setup({ heals: (n) => n === 2 });
    t.auto.onState('down');
    t.auto.onState('down');
    await vi.advanceTimersByTimeAsync(5 * S);
    expect(t.restarts).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(30 * S);
    expect(t.restarts).toHaveLength(2);
    expect(t.auto.pending).toBe(false);
    await vi.advanceTimersByTimeAsync(60 * MIN);
    expect(t.restarts).toHaveLength(2);
  });

  it('keeps backing off through a crash loop, and starts over after 10 min up', async () => {
    const t = setup({ heals: () => true });
    t.auto.onState('down');
    await vi.advanceTimersByTimeAsync(5 * S);
    // Up for a minute, down again: the next wait is the second one.
    await vi.advanceTimersByTimeAsync(1 * MIN);
    t.setState('down');
    await vi.advanceTimersByTimeAsync(30 * S - 1);
    expect(t.restarts).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(t.restarts).toHaveLength(2);
    // Up for 10 min: back to 5 s.
    await vi.advanceTimersByTimeAsync(10 * MIN);
    t.setState('down');
    await vi.advanceTimersByTimeAsync(5 * S);
    expect(t.restarts).toHaveLength(3);
  });

  it('never restarts once the app quits', async () => {
    const t = setup();
    t.auto.onState('down');
    t.stop();
    await vi.advanceTimersByTimeAsync(60 * MIN);
    expect(t.restarts).toEqual([]);
    t.auto.onState('down');
    expect(t.auto.pending).toBe(false);
  });

  it('cancel drops a pending restart (a user restart, or quitting)', async () => {
    const t = setup();
    t.auto.onState('down');
    expect(t.auto.pending).toBe(true);
    t.auto.cancel();
    await vi.advanceTimersByTimeAsync(60 * MIN);
    expect(t.restarts).toEqual([]);
    // The next down event schedules again.
    t.auto.onState('down');
    await vi.advanceTimersByTimeAsync(5 * S);
    expect(t.restarts).toHaveLength(1);
  });

  it('a failing restart is logged and the schedule goes on', async () => {
    const lines = [];
    const logger = { info: (m) => lines.push(m), warn: (m) => lines.push(m) };
    let calls = 0;
    const auto = createAutoRestart({
      restart: async () => { calls++; throw new Error('spawn python3 ENOENT'); },
      isStopped: () => false,
      getState: () => 'down',
      logger,
    });
    auto.onState('down');
    await vi.advanceTimersByTimeAsync(5 * S + 30 * S);
    expect(calls).toBe(2);
    expect(lines).toContain('[Backend] auto-restart attempt 1 failed: spawn python3 ENOENT');
  });
});
