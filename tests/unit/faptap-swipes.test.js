/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run faptap:scout --fetch takes only the videos swiped right in npm run scout:report's
// deck (<userData>/faptap-scout-swipes.json), or the ones --ids names: the swipe reader and the
// pick (scripts/faptap-scout-lib.mjs), and the CLI's --fetch --dry-run against a temp userData.
// No network: fetch throws, and a dry run needs none.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, it, expect } from 'vitest';
import { readSwipes, fetchPick, toCandidate, screenVideo, SWIPES_FILE } from '../../scripts/faptap-scout-lib.mjs';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const cli = path.join(repo, 'scripts', 'faptap-scout.mjs');
// FapTap ids are longer than a JS number holds exactly.
const BIG = '1957860091331874817';

const temps = [];
afterEach(() => { while (temps.length) fs.rmSync(temps.pop(), { recursive: true, force: true }); });

describe('readSwipes (FapTap)', () => {
  it('right swipes are wanted, left ones only hidden; ids stay strings', () => {
    const { wanted, hidden } = readSwipes({ version: 1, swipes: { [BIG]: { dir: 'right', at: 'x' }, 42: { dir: 'left', at: 'x' }, 43: { dir: 'up' } } });
    expect([...wanted]).toEqual([BIG]);
    expect([...hidden]).toEqual(['42']);
    expect(SWIPES_FILE).toBe('faptap-scout-swipes.json');
  });

  it('a missing, foreign or odd file means no swipes', () => {
    for (const json of [null, {}, { swipes: 'x' }, { swipes: { '../x': { dir: 'right' }, '': { dir: 'right' } } }]) {
      const { wanted, hidden } = readSwipes(json);
      expect(wanted.size + hidden.size).toBe(0);
    }
  });
});

describe('fetchPick', () => {
  const c = { id: BIG };
  it('without --ids: only a video swiped right, whatever its score', () => {
    expect(fetchPick({ ...c, score: -5 }, { wanted: new Set([BIG]) })).toBe(true);
    expect(fetchPick({ id: '7', score: 30 }, { wanted: new Set([BIG]) })).toBe(false);
    expect(fetchPick(c, {})).toBe(false);
  });
  it('--ids picks explicitly, swiped or not', () => {
    expect(fetchPick(c, { ids: new Set([BIG]), wanted: new Set() })).toBe(true);
    expect(fetchPick({ id: '7' }, { ids: new Set([BIG]), wanted: new Set(['7']) })).toBe(false);
  });
});

describe('toCandidate thumbnail (the deck shows it)', () => {
  const video = (thumbnail_url) => ({ id: BIG, name: 'PMV', duration: 200, tags: [{ name: 'PMV' }, { name: 'Creampie' }], user: { username: 'u' }, thumbnail_url });
  it("keeps FapTap's thumbnail as an https URL, null without one", () => {
    expect(toCandidate(video('https://vz-1.b-cdn.net/x/thumbnail.jpg'), screenVideo(video())).thumbnail).toBe('https://vz-1.b-cdn.net/x/thumbnail.jpg');
    expect(toCandidate(video('thumbnails/1.webp'), screenVideo(video())).thumbnail).toBe('https://faptap.net/api/assets/thumbnails/1.webp');
    expect(toCandidate(video(''), screenVideo(video())).thumbnail).toBeNull();
  });
});

describe('faptap:scout --fetch --dry-run', () => {
  function userData({ swipes } = {}) {
    const ud = fs.mkdtempSync(path.join(os.tmpdir(), 'faptap-swipes-'));
    temps.push(ud);
    const json = (name, v) => fs.writeFileSync(path.join(ud, name), JSON.stringify(v));
    json('config.json', { settings: { library: { sources: [] } } });
    const cand = (id, title, score) => ({
      id, title, url: `https://faptap.net/v/${id}`, author: 'u', tags: ['pmv', 'creampie'], likes: 0, duration: 200, vr: false,
      projection: 'MONO_FLAT', music: true, focus: ['pmv', 'creampie'], picked: true, favourite: null, score, quality: 1080,
    });
    json('faptap-scout.json', { generatedAt: 'x', candidates: [cand(BIG, 'Swiped right', 1), cand('2', 'Swiped left', 9), cand('3', 'Not swiped', 20)] });
    if (swipes) json(SWIPES_FILE, { version: 1, swipes });
    fs.mkdirSync(path.join(ud, 'FapTap'));
    fs.writeFileSync(path.join(ud, 'no-network.mjs'), "globalThis.fetch = async () => { throw new Error('no network in tests'); };");
    return ud;
  }
  const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('FUNSYNC_')));
  const run = (ud, ...args) => {
    const r = spawnSync(process.execPath, [
      '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON', '--import', path.join(ud, 'no-network.mjs'),
      cli, '--fetch', '--dry-run', ...args, '--user-data', ud, '--incoming', path.join(ud, 'FapTap'),
    ], { encoding: 'utf8', cwd: repo, env });
    return { code: r.status, out: r.stdout, err: r.stderr };
  };

  it('nothing swiped right: says so and downloads nothing', () => {
    const r = run(userData({ swipes: { 2: { dir: 'left', at: 'x' } } }));
    expect(r.err).toBe('');
    expect(r.code).toBe(0);
    expect(r.out).toContain('Swiped right in npm run scout:report: 0 FapTap videos');
    expect(r.out).toContain('Nothing to download');
    expect(r.out).not.toContain('Queue:');
    expect(run(userData()).out).toContain('Nothing to download');
  });

  it('takes only the right swipes, not the best score', () => {
    const r = run(userData({ swipes: { [BIG]: { dir: 'right', at: 'x' }, 2: { dir: 'left', at: 'x' } } }));
    expect(r.code).toBe(0);
    expect(r.out).toContain('Swiped right in npm run scout:report: 1 FapTap videos, 1 of them on the list');
    expect(r.out).toContain('Queue: 1 videos');
    expect(r.out).toContain(`${BIG} 1080p Swiped right`);
    expect(r.out).not.toContain('Not swiped');
    expect(r.out).not.toContain('Swiped left');
  });

  it('--ids picks without looking at the swipes', () => {
    const r = run(userData({ swipes: { [BIG]: { dir: 'right', at: 'x' } } }), '--ids', '3');
    expect(r.code).toBe(0);
    expect(r.out).not.toContain('Swiped right in');
    expect(r.out).toContain('Queue: 1 videos');
    expect(r.out).toContain('3 1080p Not swiped');
  });
});
