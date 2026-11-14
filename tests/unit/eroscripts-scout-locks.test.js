/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run eroscripts:scout's run locks, one per kind of work (lib.SCOUT_LOCKS): which lock each
// run takes, and the CLI itself against a temp userData whose other locks are held by a live pid
// (this test process): --frames goes on beside a scan and a download and takes in the old frame
// records, two frames runs exclude each other, --daily skips a busy step and records it, and the
// download lock covers --retitle. No network: fetch throws, and nothing here needs it.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, it, expect } from 'vitest';
import { SCOUT_LOCKS, scoutLockFor, lockBusyMessage } from '../../scripts/eroscripts-scout-lib.mjs';
import { FRAMES_FILE } from '../../scripts/scout-frames.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const cli = path.join(repo, 'scripts', 'eroscripts-scout.mjs');
const live = `${process.pid}@${os.hostname()}`;

const temps = [];
afterEach(() => { while (temps.length) fs.rmSync(temps.pop(), { recursive: true, force: true }); });

describe('which lock a run takes', () => {
  it('scan work, downloads and frames each have their own lock file; --status and --daily take none for the run', () => {
    expect(Object.values(SCOUT_LOCKS).map((l) => l.file)).toEqual(['eroscripts-scout.lock', 'eroscripts-scout-fetch.lock', 'eroscripts-scout-frames.lock']);
    expect(scoutLockFor({})).toBe('scan');
    expect(scoutLockFor({ add: [1] })).toBe('scan');
    expect(scoutLockFor({ rejudge: ['no-video'] })).toBe('scan');
    expect(scoutLockFor({ titles: true })).toBe('scan');
    expect(scoutLockFor({ fetch: true, auto: true })).toBe('fetch');
    expect(scoutLockFor({ fetch: true, dryRun: true })).toBe('fetch');
    expect(scoutLockFor({ manualList: true })).toBe('fetch');
    expect(scoutLockFor({ retitle: true, dryRun: true })).toBe('fetch');
    expect(scoutLockFor({ frames: 0 })).toBe('frames');
    expect(scoutLockFor({ frames: 3, ids: new Set([1]) })).toBe('frames');
    expect(scoutLockFor({ status: true })).toBe(null);
    expect(scoutLockFor({ daily: true })).toBe(null);
    // Flags that pick another run first take that run's lock.
    expect(scoutLockFor({ add: [1], fetch: true })).toBe('scan');
    expect(scoutLockFor({ daily: true, retitle: true })).toBe('fetch');
    expect(scoutLockFor({ status: true, fetch: true })).toBe(null);
  });

  it('the message names the work, the lock file and the holder', () => {
    expect(lockBusyMessage('fetch', 'pid 12 on mac')).toBe('Another eroscripts:scout run is doing downloads (--fetch, --manual-list, --retitle): eroscripts-scout-fetch.lock is held by pid 12 on mac');
  });
});

describe('eroscripts:scout with other runs going', () => {
  // A userData with the forum session, a waiting list of three posts (one swiped right, one left,
  // one not swiped; their only scene is an archive, so no host is read), and frames made before the
  // frames file: two records on the scan state and a folder on disk without one.
  function userData() {
    const ud = fs.mkdtempSync(path.join(os.tmpdir(), 'scout-locks-'));
    temps.push(ud);
    const json = (name, v) => fs.writeFileSync(path.join(ud, name), JSON.stringify(v));
    json('config.json', { settings: { eroscripts: { session: { cookie: 'test-cookie', username: 'tester' } }, library: { sources: [] } } });
    // Every record has its post info, so --titles has nothing to read and only saves the state.
    const rec = (extra = {}) => ({ verdict: 'candidate', rules: 14, at: '2026-11-01T00:00:00.000Z', title: 'T', images: [], imagesVersion: 2, ...extra });
    json('eroscripts-scout-state.json', {
      version: 1, lastScanAt: null, feeds: {}, pending: {},
      topics: {
        101: rec(), 102: rec(), 103: rec(),
        201: rec({ frames: { at: '2026-11-13T00:00:00.000Z', source: 'mega: a.mp4', files: ['25.jpg', '50.jpg'] } }),
        202: rec({ framesFailed: { at: '2026-11-13T00:00:00.000Z', reason: 'rule34video: HTTP 403' } }),
      },
    });
    const packed = (id) => ({ id, title: `Post ${id}`, author: 'a', tags: [], score: id, axisType: 'multi', scenes: [{ base: `p${id}`, packed: true, video: { name: `p${id}.zip`, host: 'pixeldrain' }, scripts: [], axes: [] }] });
    json('eroscripts-scout.json', { candidates: [packed(101), packed(102), packed(103)] });
    json('eroscripts-scout-swipes.json', { version: 1, swipes: { 102: { dir: 'right', at: 'x' }, 103: { dir: 'left', at: 'x' } } });
    const disk = path.join(ud, 'scout-frames', 'eroscripts', '203');
    fs.mkdirSync(disk, { recursive: true });
    fs.writeFileSync(path.join(disk, '75.jpg'), 'jpeg');
    fs.mkdirSync(path.join(ud, 'EroScripts'));
    const stub = path.join(ud, 'no-network.mjs');
    fs.writeFileSync(stub, "globalThis.fetch = async () => { throw new Error('no network in tests'); };");
    return ud;
  }
  const hold = (ud, kind) => fs.writeFileSync(path.join(ud, SCOUT_LOCKS[kind].file), live);
  const read = (ud, name) => JSON.parse(fs.readFileSync(path.join(ud, name), 'utf8'));
  const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('FUNSYNC_')));
  const run = (ud, ...args) => {
    const r = spawnSync(process.execPath, [
      '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON', '--import', path.join(ud, 'no-network.mjs'),
      cli, ...args, '--user-data', ud, '--incoming', path.join(ud, 'EroScripts'),
    ], { encoding: 'utf8', cwd: repo, env });
    return { code: r.status, out: r.stdout, err: r.stderr };
  };

  it('--frames runs while a scan and a download hold their locks: only unswiped posts, old records taken in, the state and list only read', () => {
    const ud = userData();
    hold(ud, 'scan');
    hold(ud, 'fetch');
    const state = fs.readFileSync(path.join(ud, 'eroscripts-scout-state.json'), 'utf8');
    const list = fs.readFileSync(path.join(ud, 'eroscripts-scout.json'), 'utf8');
    const r = run(ud, '--frames', '3');
    expect(r.err).toBe('');
    expect(r.code).toBe(0);
    expect(r.out).toMatch(/took in 2 records from the scan state and 1 frames folders without a record/);
    // 102 (swiped right) and 103 (swiped left) aren't taken; 101 is, and has no free source (an archive).
    expect(r.out).toMatch(/Frames: 1 unswiped waiting posts show fewer than 3 images/);
    expect(r.out).toMatch(/skipped, no free source: archive 1/);
    const made = read(ud, FRAMES_FILE);
    expect(made.version).toBe(1);
    expect(made.topics).toEqual({
      201: { frames: { at: '2026-11-13T00:00:00.000Z', source: 'mega: a.mp4', files: ['25.jpg', '50.jpg'] } },
      202: { framesFailed: { at: '2026-11-13T00:00:00.000Z', reason: 'rule34video: HTTP 403' } },
      203: { frames: { at: made.topics[203].frames.at, source: '', files: ['75.jpg'] } },
    });
    expect(fs.readFileSync(path.join(ud, 'eroscripts-scout-state.json'), 'utf8')).toBe(state);
    expect(fs.readFileSync(path.join(ud, 'eroscripts-scout.json'), 'utf8')).toBe(list);
    // The other runs' locks are untouched, and this run's own is gone.
    expect(fs.readFileSync(path.join(ud, SCOUT_LOCKS.scan.file), 'utf8')).toBe(live);
    expect(fs.readFileSync(path.join(ud, SCOUT_LOCKS.fetch.file), 'utf8')).toBe(live);
    expect(fs.existsSync(path.join(ud, SCOUT_LOCKS.frames.file))).toBe(false);
  }, 60_000);

  it('a second --frames run exits 2 and says a frames run holds eroscripts-scout-frames.lock', () => {
    const ud = userData();
    hold(ud, 'frames');
    const r = run(ud, '--frames', '3');
    expect(r.code).toBe(2);
    expect(r.err).toContain(`Another eroscripts:scout run is doing frames (--frames): eroscripts-scout-frames.lock is held by pid ${process.pid} on ${os.hostname()}`);
    expect(fs.existsSync(path.join(ud, FRAMES_FILE))).toBe(false);
    expect(fs.readFileSync(path.join(ud, SCOUT_LOCKS.frames.file), 'utf8')).toBe(live);
  }, 60_000);

  it('--daily skips the steps whose lock another run holds, records them (exit 2), and still makes frames', () => {
    const ud = userData();
    hold(ud, 'scan');
    hold(ud, 'fetch');
    const r = run(ud, '--daily');
    expect(r.code).toBe(2);
    expect(r.out).toContain('Daily: Another eroscripts:scout run is doing a scan (scan, --add, --rejudge, --titles): eroscripts-scout.lock is held by');
    expect(r.out).toContain('Daily: Another eroscripts:scout run is doing downloads (--fetch, --manual-list, --retitle): eroscripts-scout-fetch.lock is held by');
    expect(r.out).toMatch(/Frames: 1 unswiped waiting posts/);
    const daily = read(ud, 'eroscripts-scout-daily.json');
    expect(daily).toMatchObject({
      scanExit: 2, scanSkipped: `eroscripts-scout.lock held by pid ${process.pid} on ${os.hostname()}`,
      framesExit: 0,
      fetchExit: 2, fetchSkipped: `eroscripts-scout-fetch.lock held by pid ${process.pid} on ${os.hostname()}`,
    });
    for (const k of ['scanAt', 'framesAt', 'fetchAt']) expect(Number.isFinite(Date.parse(daily[k]))).toBe(true);
    expect(daily).not.toHaveProperty('framesSkipped');
    expect(Object.keys(read(ud, FRAMES_FILE).topics).sort()).toEqual(['201', '202', '203']);
    expect(fs.existsSync(path.join(ud, SCOUT_LOCKS.frames.file))).toBe(false);
    // --status shows the skipped steps.
    const s = run(ud, '--status');
    expect(s.code).toBe(0);
    expect(s.out).toMatch(/Daily run: scan .*, exit 2 \(skipped: eroscripts-scout\.lock held by pid \d+ on .*\); frames .*, exit 0; auto-fetch .*, exit 2 \(skipped: eroscripts-scout-fetch\.lock/);
  }, 60_000);

  it('--retitle waits for a download run (the library lock), not for a frames run', () => {
    const ud = userData();
    hold(ud, 'fetch');
    const busy = run(ud, '--retitle', '--dry-run');
    expect(busy.code).toBe(2);
    expect(busy.err).toContain('eroscripts-scout-fetch.lock is held by');
    fs.rmSync(path.join(ud, SCOUT_LOCKS.fetch.file));
    hold(ud, 'frames');
    hold(ud, 'scan');
    const free = run(ud, '--retitle', '--dry-run');
    expect(free.code).toBe(0);
    expect(free.out).toContain('0 of 0 scenes would be renamed');
  }, 60_000);

  it('a scan-lock run (--titles) goes on beside frames and downloads, and drops the state\'s old frame records only once the frames file has them', () => {
    const ud = userData();
    const topics = () => read(ud, 'eroscripts-scout-state.json').topics;
    hold(ud, 'frames');
    hold(ud, 'fetch');
    const before = run(ud, '--titles');
    expect(before.code).toBe(0);
    expect(before.out).toContain('0 judged posts without a title or images, 0 waiting posts without a script preview');
    // No frames file yet: the records stay where they are.
    expect(topics()[201].frames).toBeTruthy();
    expect(topics()[202].framesFailed).toBeTruthy();
    fs.rmSync(path.join(ud, SCOUT_LOCKS.frames.file));
    expect(run(ud, '--frames', '0').code).toBe(0);
    expect(Object.keys(read(ud, FRAMES_FILE).topics).sort()).toEqual(['201', '202', '203']);
    expect(run(ud, '--titles').code).toBe(0);
    expect(topics()[201]).toEqual({ verdict: 'candidate', rules: 14, at: '2026-11-01T00:00:00.000Z', title: 'T', images: [], imagesVersion: 2 });
    expect(topics()[202]).not.toHaveProperty('framesFailed');
    expect(read(ud, FRAMES_FILE).topics[201].frames.source).toBe('mega: a.mp4');
  }, 60_000);
});
