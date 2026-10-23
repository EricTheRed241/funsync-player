/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run eroscripts:sync — reverse search from library videos to their
// EroScripts posts (scripts/eroscripts-sync-lib.mjs). The forum is faked;
// scenes are real folders in a temp dir.
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  cleanVideoName, queryVariants, normalisedBase, scriptLengthMs,
  classifyScript, createPacer, createForum, loadLocalScene, matchVideo, runSync,
  isStale, planVideo, availableAxes, installAxes, pruneInstalledFolders, installSuffix, renderReport, isRelevant,
  creatorOf, parseInterval, launchdPlist, LAUNCHD_LABEL, SEARCH_INTERVAL_MS,
  RateLimitedError, SessionExpiredError,
} from '../../scripts/eroscripts-sync-lib.mjs';
import { actionsHash } from '../../scripts/handmade-scenes.mjs';

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.parse('2026-10-22T12:00:00Z');

/** A funscript ending at `lengthMs`, shaped by `seed` so different seeds hash differently. */
function script(lengthMs, seed = 1, meta = {}) {
  const actions = [];
  for (let at = 0, i = 0; at <= lengthMs; at += 250, i++) actions.push({ at, pos: (i * 37 * seed) % 101 });
  if (actions[actions.length - 1].at !== lengthMs) actions.push({ at: lengthMs, pos: 50 });
  return JSON.stringify({ version: '1.0', ...meta, actions });
}

let tmp;
beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'es-sync-')); });
afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); vi.useRealTimers(); });

/** `<tmp>/Library/<creator>/<scene>/` with the given files (name → content). */
function scene(creator, sceneName, files) {
  const dir = path.join(tmp, 'Library', creator, sceneName);
  fs.mkdirSync(dir, { recursive: true });
  for (const [name, content] of Object.entries(files)) fs.writeFileSync(path.join(dir, name), content);
  return dir;
}

function video(dir, stem, creator, scriptStem = stem) {
  return { path: path.join(dir, `${stem}.mp4`), funscriptPath: path.join(dir, `${scriptStem}.funscript`), creator };
}

const FORUM = 'https://discuss.eroscripts.com';
const upload = (id) => `${FORUM}/uploads/short-url/${id}.funscript`;
const att = (name, id, extra = {}) => ({ name, url: upload(id), host: 'eroscripts', postNumber: 1, username: 'author', byAuthor: true, ...extra });
const topicResult = (id, title, extra = {}) => ({ id, title, url: `${FORUM}/t/x/${id}`, creator: 'author', tags: ['osr2'], category: 'free-scripts', paid: false, ...extra });

function fakeApi({ search = () => [], topics = {}, files = {} } = {}) {
  return {
    search: vi.fn(async (q) => ({ results: search(q) })),
    getTopicAttachments: vi.fn(async (id) => topics[id] || { attachments: [], linked: [], external: [], paid: false }),
    fetchScript: vi.fn(async (url) => (files[url] != null
      ? { success: true, bytes: Buffer.from(files[url]) }
      : { success: false, error: 'Download failed (404)', status: 404 })),
  };
}

const instant = () => createPacer({ intervalMs: 0 });
const forumFor = (api, cache = {}) => createForum(api, { pacer: instant(), searchPacer: instant(), cache, now: () => NOW });

describe('name cleaning', () => {
  it.each([
    ['Fap Hero - Fantasy Play_hard.mp4', 'Fap Hero', 'Fantasy Play'],
    ['[Drills3D] Judy Alvarez - Braindance Tryout VR.mp4', 'Drills3D', 'Judy Alvarez - Braindance Tryout'],
    ['CM 045 - Date Night Sex With Joni_4k.mp4', 'Cuddle Mocap', '045 - Date Night Sex With Joni'],
    ['Agapilord - Succubitch LoL_120fps.mp4', 'Agapilord', 'Succubitch LoL'],
    ['Scene Title [8K120FPS HEVC].mp4', null, 'Scene Title'],
    ['Scene Title (Soft).funscript', null, 'Scene Title'],
    ['Scene Title_6k.mp4', null, 'Scene Title'],
    ['Cyberpunk 2077 POV - Nikovako.mp4', 'Nikovako', 'Cyberpunk 2077 POV'],
    ['Mime AI by Alberist.mp4', 'Alberist', 'Mime AI'],
    ['Animex 3.0.1.mp4', 'Unknown', 'Animex 3.0.1'],
  ])('%s → %s', (name, creator, expected) => {
    expect(cleanVideoName(name, creator)).toBe(expected);
  });

  it('never strips the creator down to nothing', () => {
    expect(cleanVideoName('Subverse.mp4', 'Subverse')).toBe('Subverse');
  });

  it('builds query variants: title, creator + title, title without a _label', () => {
    expect(queryVariants('CM 048 - Cyber Sex_eve.mp4', 'Cuddle Mocap'))
      .toEqual(['048 Cyber Sex eve', 'Cuddle Mocap 048 Cyber Sex eve', '048 Cyber Sex']);
    expect(queryVariants('Agapilord - Bad.mp4', 'Agapilord')).toEqual(['Agapilord Bad', 'Bad']);
    expect(queryVariants('Like That HMV.mp4', 'Unknown')).toEqual(['Like That HMV']);
  });

  it('normalised base keeps difficulty words, drops format tags, axis suffixes and the creator', () => {
    expect(normalisedBase('Title_hard.funscript')).not.toBe(normalisedBase('Title.funscript'));
    expect(normalisedBase('Title_4k.funscript')).toBe(normalisedBase('Title.funscript'));
    expect(normalisedBase('Title.generated.twist.funscript')).toBe('title');
    expect(normalisedBase('Fap Hero - Title.funscript', 'Fap Hero')).toBe(normalisedBase('Title.funscript', 'Fap Hero'));
    expect(normalisedBase('The Pull - AlpacaLaps.funscript', 'AlpacaLaps')).toBe('the pull');
  });

  it('inspects results sharing half the query, or naming the creator the query names', () => {
    expect(isRelevant('Fantasy Play', 'Fap Hero Fantasy Play (both versions)', 'Fap Hero')).toBe(true);
    expect(isRelevant('2B City', 'Getting Cozy in Park City', 'Shayuki')).toBe(true); // half: weak, but cheap to check
    expect(isRelevant('Chun Li Special', '[Agapilord] HMV/PMV Hexa-Bundle! (6 HMV Scripts)', 'Agapilord')).toBe(false);
    expect(isRelevant('Agapilord Chun Li Special', '[Agapilord] HMV/PMV Hexa-Bundle! (6 HMV Scripts)', 'Agapilord')).toBe(true);
    expect(isRelevant('Unknown Scene Title', 'Unknown artist bundle', 'Unknown')).toBe(false);
  });

  it('reads the creator from <Source>/<Creator>/<Scene>/file', () => {
    expect(creatorOf('/lib/A/Creator X/Scene/v.mp4', ['/lib/A'])).toBe('Creator X');
    expect(creatorOf('/lib/A/v.mp4', ['/lib/A'])).toBe(null);
  });
});

describe('script comparison', () => {
  it('hashes normalised actions only', () => {
    const a = JSON.parse(script(60_000));
    const b = { metadata: { title: 'other' }, actions: [...a.actions].reverse().map((x) => ({ pos: x.pos, at: x.at + 0.2 })) };
    expect(actionsHash(b)).toBe(actionsHash(a));
    expect(actionsHash(JSON.parse(script(60_000, 2)))).not.toBe(actionsHash(a));
    expect(actionsHash({ actions: [{ at: 0, pos: 0 }] })).toBe(null);
    expect(scriptLengthMs(a)).toBe(60_000);
  });

  const local = (name, lengthMs, seed = 1) => {
    const s = JSON.parse(script(lengthMs, seed));
    return { video: { creator: null }, hash: actionsHash(s), lengthMs, base: normalisedBase(name) };
  };

  it('confirms an identical script whatever its name', () => {
    expect(classifyScript(local('Mine.funscript', 600_000), { name: 'Other.funscript', script: JSON.parse(script(600_000)) }))
      .toEqual({ confidence: 'confirmed', basis: 'main-hash' });
  });

  it('calls it probable when the base name matches and the length is within 2 s', () => {
    const r = classifyScript(local('Scene_4k.funscript', 600_000), { name: 'Scene.funscript', script: JSON.parse(script(601_900, 3)) });
    expect(r).toEqual({ confidence: 'probable', basis: 'name+length' });
  });

  it('leaves a name match with a different length, or a length match with another name, unapplied', () => {
    const l = local('Scene.funscript', 600_000);
    const longer = classifyScript(l, { name: 'Scene.funscript', script: JSON.parse(script(603_000, 3)) });
    expect(longer.confidence).toBe(null);
    expect(longer.reason).toMatch(/name matches, length differs/);
    const otherName = classifyScript(l, { name: 'Different.funscript', script: JSON.parse(script(600_000, 3)) });
    expect(otherName.confidence).toBe(null);
  });
});

describe('matching a video', () => {
  it('confirms by main-script content and records the post', async () => {
    const dir = scene('Fap Hero', 'Fap Hero - Fantasy Play', { 'Fap Hero - Fantasy Play_hard.mp4': '', 'Fap Hero - Fantasy Play_hard.funscript': script(900_000) });
    const api = fakeApi({
      search: () => [topicResult(125133, 'Fap Hero Fantasy Play (both versions)', { preview: 'https://eroscripts-discourse.eroscripts.com/original/p.jpg' })],
      topics: { 125133: { attachments: [att('Fantasy Play - HARD MODE.funscript', 'h'), att('Fantasy Play.funscript', 's')], linked: [], external: [], paid: false } },
      files: { [upload('h')]: script(900_000, 1, { metadata: { creator: 'quantoz' } }), [upload('s')]: script(900_000, 5) },
    });
    const r = await matchVideo(forumFor(api), loadLocalScene(video(dir, 'Fap Hero - Fantasy Play_hard', 'Fap Hero')), { now: NOW });
    expect(r.outcome).toBe('confirmed');
    expect(r.link).toMatchObject({
      topicId: 125133, confidence: 'confirmed', basis: 'main-hash', matchedFile: 'Fantasy Play - HARD MODE.funscript',
      tags: ['osr2'], category: 'free-scripts', paid: false, previewUrl: 'https://eroscripts-discourse.eroscripts.com/original/p.jpg',
    });
    expect(r.link.matchedAt).toBe(new Date(NOW).toISOString());
  });

  it('confirms by a hand-made axis file, and keeps reposts that carry it as related', async () => {
    const dir = scene('Fap Hero', 'Scene', { 'Scene.funscript': script(500_000), 'Scene.pitch.funscript': script(500_000, 7) });
    const api = fakeApi({
      search: () => [topicResult(1, 'Fap Hero Scene original'), topicResult(2, '[Multi-Axis Initiative] Fap Hero Scene')],
      topics: {
        1: { attachments: [att('Scene.funscript', 'm')], linked: [], external: [], paid: false },
        2: {
          attachments: [att('Scene ADV.pitch.funscript', 'p'), att('Scene ADV.v0.funscript', 'v', { postNumber: 23, username: 'dimnogro', byAuthor: false })],
          linked: [], external: [], paid: false,
        },
      },
      files: { [upload('m')]: script(500_000), [upload('p')]: script(500_000, 7), [upload('v')]: script(499_000, 9) },
    });
    const local = loadLocalScene(video(dir, 'Scene', 'Fap Hero'));
    const r = await matchVideo(forumFor(api), local, { now: NOW });
    expect(r.link.topicId).toBe(1);
    expect(r.link.related).toHaveLength(1);
    expect(r.link.related[0]).toMatchObject({ topicId: 2, basis: 'axis-hash', confidence: 'confirmed' });

    const plan = availableAxes(r.link, local);
    expect(plan.missing.map((m) => m.axis)).toEqual(['vib']);
    expect(plan.missing[0]).toMatchObject({ installable: true, guard: 'confirmed', file: { topicId: 2, username: 'dimnogro', byAuthor: false } });
  });

  it('puts paid posts and near misses on the review list, never linked', async () => {
    const dir = scene('Studio', 'Scene', { 'Scene.funscript': script(300_000) });
    const api = fakeApi({
      search: () => [topicResult(10, 'Studio Scene', { paid: true, category: 'paid-scripts' }), topicResult(11, 'Scene remake'), topicResult(12, 'Studio Other Thing')],
      topics: {
        10: { attachments: [], linked: [], external: [], paid: true },
        11: { attachments: [att('Scene.funscript', 'x')], linked: [], external: [], paid: false },
      },
      files: { [upload('x')]: script(320_000, 4) },
    });
    const r = await matchVideo(forumFor(api), loadLocalScene(video(dir, 'Scene', 'Studio')), { now: NOW });
    expect(r.outcome).toBe('review');
    expect(r.link).toBeUndefined();
    const reasons = Object.fromEntries(r.review.candidates.map((c) => [c.topicId, c.reasons.join(' ')]));
    expect(reasons[10]).toMatch(/paid/);
    expect(reasons[11]).toMatch(/length differs/);
    // Inspected (half the query) but no near miss: not worth a reviewer's time.
    expect(api.getTopicAttachments).toHaveBeenCalledWith(12);
    expect(reasons[12]).toBeUndefined();
  });

  it('reports none when the search finds nothing related', async () => {
    const dir = scene('Studio', 'Scene', { 'Scene.funscript': script(300_000) });
    const api = fakeApi({ search: () => [topicResult(5, 'Completely unrelated')] });
    const r = await matchVideo(forumFor(api), loadLocalScene(video(dir, 'Scene', 'Studio')), { now: NOW });
    expect(r.outcome).toBe('none');
    expect(api.getTopicAttachments).not.toHaveBeenCalled();
  });

  it('caches searches across runs for the stale window', async () => {
    const api = fakeApi({ search: () => [] });
    const cache = {};
    await forumFor(api, cache).search('scene');
    await forumFor(api, cache).search('scene');
    expect(api.search).toHaveBeenCalledTimes(1);
    const later = createForum(api, { pacer: createPacer({ intervalMs: 0 }), cache, now: () => NOW + 8 * DAY });
    await later.search('scene');
    expect(api.search).toHaveBeenCalledTimes(2);
  });
});

describe('staleness', () => {
  it('flags missing and old dates', () => {
    expect(isStale(null, NOW)).toBe(true);
    expect(isStale(new Date(NOW - 6 * DAY).toISOString(), NOW, 7)).toBe(false);
    expect(isStale(new Date(NOW - 7 * DAY).toISOString(), NOW, 7)).toBe(true);
  });

  it('skips confirmed videos (refreshing their post when stale) and re-checks old unconfirmed ones', () => {
    const recent = new Date(NOW - DAY).toISOString();
    const old = new Date(NOW - 10 * DAY).toISOString();
    const state = {
      links: { a: { confidence: 'confirmed', checkedAt: recent }, b: { confidence: 'confirmed', checkedAt: old }, c: { confidence: 'probable', checkedAt: recent }, d: { confidence: 'probable', checkedAt: old } },
      checked: { e: { checkedAt: recent, outcome: 'review' }, f: { checkedAt: old, outcome: 'none' } },
    };
    const plan = (p, o = {}) => planVideo(state, p, { now: NOW, staleDays: 7, ...o });
    expect(['a', 'b', 'c', 'd', 'e', 'f', 'g'].map((p) => plan(p))).toEqual(['skip', 'refresh-topic', 'skip', 'match', 'skip', 'match', 'match']);
    expect(plan('a', { refresh: true })).toBe('match');
    expect(['a', 'b', 'c', 'd', 'e', 'f'].map((p) => plan(p, { retry: true }))).toEqual(['skip', 'refresh-topic', 'match', 'match', 'match', 'match']);
  });
});

describe('polite pacing', () => {
  it('spaces forum requests at least 700 ms apart', async () => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
    const pacer = createPacer();
    const done = [];
    const run = (async () => {
      for (let i = 0; i < 3; i++) { await pacer.wait(); done.push(Date.now() - NOW); }
    })();
    await vi.advanceTimersByTimeAsync(0);
    expect(done).toEqual([0]);
    await vi.advanceTimersByTimeAsync(699);
    expect(done).toEqual([0]);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toEqual([0, 700]);
    await vi.advanceTimersByTimeAsync(700);
    await run;
    expect(done).toEqual([0, 700, 1400]);
  });

  it('runs every forum call through the pacer', async () => {
    const waits = vi.fn(async () => {});
    const api = fakeApi({ search: () => [topicResult(1, 'Scene')], topics: { 1: { attachments: [att('Scene.funscript', 's')], linked: [], external: [], paid: false } }, files: { [upload('s')]: script(1000) } });
    const searchWaits = vi.fn(async () => {});
    const forum = createForum(api, { pacer: { wait: waits, count: 0 }, searchPacer: { wait: searchWaits }, now: () => NOW });
    await forum.search('scene');
    await forum.topic(1);
    await forum.entries(upload('s'), 'Scene.funscript');
    expect(waits).toHaveBeenCalledTimes(3);
    expect(searchWaits).toHaveBeenCalledTimes(1);
    expect(SEARCH_INTERVAL_MS).toBeGreaterThanOrEqual(3000);
  });
});

describe('runSync', () => {
  function library() {
    const a = scene('S', 'Alpha', { 'Alpha.funscript': script(100_000, 1) });
    const b = scene('S', 'Beta', { 'Beta.funscript': script(100_000, 2) });
    const c = scene('S', 'Gamma', { 'Gamma.funscript': script(100_000, 3) });
    return [video(a, 'Alpha', 'S'), video(b, 'Beta', 'S'), video(c, 'Gamma', 'S'), { path: path.join(a, 'NoScript.mp4'), funscriptPath: null, creator: 'S' }];
  }
  const alphaApi = (over = {}) => fakeApi({
    search: (q) => (/alpha/i.test(q) ? [topicResult(1, 'S Alpha')] : []),
    topics: { 1: { attachments: [att('Alpha.funscript', 'a')], linked: [], external: [], paid: false } },
    files: { [upload('a')]: script(100_000, 1) },
    ...over,
  });

  it('stops on 429 with progress saved and resumes on the next run', async () => {
    const videos = library();
    const api = alphaApi();
    const realSearch = api.search;
    api.search = vi.fn(async (q) => (/beta/i.test(q) ? { results: [], error: 'Rate limited', rateLimited: true } : realSearch(q)));
    const state = { links: {}, review: {}, checked: {} };
    const persist = vi.fn();
    const s1 = await runSync({ videos, forum: forumFor(api), state, persist, now: () => NOW });
    expect(s1.stopped).toBe('rate-limited');
    expect(state.links[videos[0].path].confidence).toBe('confirmed');
    expect(state.checked[videos[1].path]).toBeUndefined();
    expect(persist).toHaveBeenCalled();

    const s2 = await runSync({ videos, forum: forumFor(alphaApi()), state, now: () => NOW });
    expect(s2.stopped).toBe(null);
    expect(s2.skipped).toBe(1); // Alpha, confirmed
    expect(s2.checked).toBe(2); // Beta and Gamma
    expect(s2.noScript).toBe(1);
  });

  it('throws SessionExpiredError on 401/403 after saving progress', async () => {
    const api = alphaApi();
    api.search = vi.fn(async () => ({ results: [], error: 'Access denied', authExpired: true }));
    const persist = vi.fn();
    await expect(runSync({ videos: library(), forum: forumFor(api), state: { links: {} }, persist, now: () => NOW }))
      .rejects.toBeInstanceOf(SessionExpiredError);
    expect(persist).toHaveBeenCalled();
  });

  it('checks at most --limit videos', async () => {
    const s = await runSync({ videos: library(), forum: forumFor(alphaApi()), state: { links: {} }, limit: 2, now: () => NOW });
    expect(s.checked).toBe(2);
  });

  it('exports the error classes the CLI maps to exit codes', () => {
    expect(new RateLimitedError().message).toMatch(/429/);
    expect(new SessionExpiredError().message).toMatch(/expired/);
  });
});

describe('--install-axes guard', () => {
  async function setup({ confidence = 'confirmed', mains = [], axisLength = 99_000, existing = {} } = {}) {
    const dir = scene('S', 'Scene', {
      'Scene.mp4': '',
      'Scene.funscript': script(100_000),
      'Scene.generated.twist.funscript': script(100_000, 4),
      'Scene.generated.roll.funscript': script(100_000, 5),
      ...existing,
    });
    const files = { [upload('tw')]: script(axisLength, 8), [upload('su')]: script(axisLength, 9) };
    const api = fakeApi({ files });
    const local = loadLocalScene(video(dir, 'Scene', 'S'));
    const link = {
      topicId: 1, topicUrl: `${FORUM}/t/x/1`, confidence, matchedFile: confidence === 'confirmed' ? 'Scene.funscript' : null, mains,
      axes: [
        { name: 'Scene.twist.funscript', url: upload('tw'), host: 'eroscripts', axis: 'twist', base: 'Scene', byAuthor: true },
        { name: 'Scene.l1.funscript', url: upload('su'), host: 'eroscripts', axis: 'surge', base: 'Scene', byAuthor: true },
      ],
    };
    return { dir, local, link, forum: forumFor(api), archiveDir: path.join(tmp, 'Archive', '_pruned-2026-10-22') };
  }

  it('installs confirmed axes under the scan spelling, then archives the generated companions', async () => {
    const { dir, local, link, forum, archiveDir } = await setup();
    const plan = availableAxes(link, local);
    expect(plan.missing.every((m) => m.installable)).toBe(true);
    const r = await installAxes({ forum, local, plan, archiveDir, relativeTo: tmp });
    expect(r.installed.map((p) => path.basename(p)).sort()).toEqual(['Scene.surge.funscript', 'Scene.twist.funscript']);
    expect(fs.existsSync(path.join(dir, 'Scene.generated.twist.funscript'))).toBe(false);
    expect(fs.existsSync(path.join(archiveDir, 'Library', 'S', 'Scene', 'Scene.generated.roll.funscript'))).toBe(true);
    const manifest = fs.readFileSync(path.join(archiveDir, 'MANIFEST.txt'), 'utf8');
    expect(manifest).toContain(`${path.join(dir, 'Scene.generated.twist.funscript')}\t`);
    expect(installSuffix('v0')).toBe('vib');
  });

  it('never overwrites an existing file', async () => {
    const { dir, local, link, forum, archiveDir } = await setup({ existing: { 'Scene.twist.funscript': 'MINE' } });
    // The local twist is hand-made now, so only surge is missing; force the clash.
    fs.writeFileSync(path.join(dir, 'Scene.surge.funscript'), 'ALSO MINE');
    const plan = { missing: [{ axis: 'surge', file: link.axes[1], alternatives: [], guard: 'confirmed', installable: true }] };
    const r = await installAxes({ forum, local, plan, archiveDir, relativeTo: tmp });
    expect(r.installed).toEqual([]);
    expect(r.skipped).toEqual([{ axis: 'surge', reason: 'file exists' }]);
    expect(fs.readFileSync(path.join(dir, 'Scene.surge.funscript'), 'utf8')).toBe('ALSO MINE');
    expect(fs.existsSync(path.join(dir, 'Scene.generated.twist.funscript'))).toBe(true);
  });

  it('refuses a probable post whose main script length is unverified', async () => {
    const { local, link, forum, archiveDir } = await setup({ confidence: 'probable', mains: [{ name: 'Scene.funscript', base: 'Scene', lengthMs: 140_000 }] });
    const plan = availableAxes(link, local);
    expect(plan.missing.some((m) => m.installable)).toBe(false);
    const r = await installAxes({ forum, local, plan, archiveDir, relativeTo: tmp });
    expect(r.installed).toEqual([]);
    expect(r.archived).toEqual([]);
  });

  it('accepts a probable post whose main script has the local length', async () => {
    const { local, link } = await setup({ confidence: 'probable', mains: [{ name: 'Scene.funscript', base: 'Scene', lengthMs: 101_500 }] });
    expect(availableAxes(link, local).missing.every((m) => m.installable && m.guard === 'length')).toBe(true);
  });

  it('skips axis files that run past the local script', async () => {
    const { local, link, forum, archiveDir } = await setup({ axisLength: 130_000 });
    const r = await installAxes({ forum, local, plan: availableAxes(link, local), archiveDir, relativeTo: tmp });
    expect(r.installed).toEqual([]);
    expect(r.skipped.map((s) => s.reason)).toEqual(['longer than the local script', 'longer than the local script']);
  });

  it('lists nothing when the post offers several axis sets and none matches', async () => {
    const { local, link } = await setup();
    link.matchedFile = null;
    link.axes = [
      { name: 'Other A.twist.funscript', url: upload('tw'), host: 'eroscripts', axis: 'twist', base: 'Other A' },
      { name: 'Other B.twist.funscript', url: upload('tw'), host: 'eroscripts', axis: 'twist', base: 'Other B' },
    ];
    const plan = availableAxes(link, local);
    expect(plan.missing).toEqual([]);
    expect(plan.ambiguous[0].groups).toEqual(['Other A', 'Other B']);
  });

  it("then copies them to timing-compatible sibling videos and archives the folder's generated files", async () => {
    const { dir, local, link, forum, archiveDir } = await setup({ existing: {
      'Scene_4k.mp4': '',
      'Scene_4k.funscript': script(100_000),
      'Scene_4k.generated.roll.funscript': script(100_000, 5),
      'Scene_VR.mp4': '',
      'Scene_VR.funscript': script(130_000),
      'Scene_VR.generated.twist.funscript': script(130_000, 4),
    } });
    await installAxes({ forum, local, plan: availableAxes(link, local), archiveDir, relativeTo: tmp });
    const siblings = await pruneInstalledFolders([{ dir, archiveDir, relativeTo: tmp }]);
    expect(siblings).toHaveLength(1);
    const [f] = siblings;
    expect(f.copied.map((p) => path.basename(p)).sort()).toEqual(['Scene_4k.surge.funscript', 'Scene_4k.twist.funscript']);
    expect(fs.readFileSync(path.join(dir, 'Scene_4k.twist.funscript'), 'utf8')).toBe(fs.readFileSync(path.join(dir, 'Scene.twist.funscript'), 'utf8'));
    expect(f.refused).toEqual([path.join(dir, 'Scene_VR.mp4')]);
    expect(f.archived.map((x) => path.basename(x.from)).sort()).toEqual(['Scene_4k.generated.roll.funscript', 'Scene_VR.generated.twist.funscript']);
    expect(fs.readdirSync(dir).filter((n) => n.includes('.generated.'))).toEqual([]);
    const md = renderReport({ state: {}, summary: { checked: 0, skipped: 0, noScript: 0, errors: [], stopped: null }, siblings, roots: [tmp], now: NOW });
    expect(md).toContain('`Scene_VR.mp4`: sibling not timing-compatible, nothing copied');
  });
});

describe('report', () => {
  it('renders the summary, links, review list and axes', () => {
    const state = {
      links: { '/L/S/A/A.mp4': { topicId: 1, topicUrl: 'u1', title: 'A post', creator: 'x', confidence: 'confirmed', basis: 'main-hash', matchedFile: 'A.funscript' } },
      review: { '/L/S/B/B.mp4': { queries: ['B'], candidates: [{ topicId: 2, title: 'B?', url: 'u2', paid: true, reasons: ['paid post (no attached scripts)'] }] } },
    };
    const md = renderReport({ state, summary: { checked: 2, skipped: 0, noScript: 0, errors: [], stopped: null }, roots: ['/L'], previews: { available: 1, withUrl: 1, withoutThumb: 1 }, now: NOW });
    expect(md).toContain('Linked: 1 (confirmed 1, probable 0)');
    expect(md).toContain('`L/S/A/A.mp4` → [A post](u1)');
    expect(md).toContain('[B?](u2) (paid): paid post');
  });
});

describe('launchd schedule', () => {
  it('parses intervals', () => {
    expect(parseInterval('3d')).toBe(259200);
    expect(parseInterval('12h')).toBe(43200);
    expect(parseInterval('90m')).toBe(5400);
    expect(parseInterval('3')).toBe(null);
    expect(parseInterval('0d')).toBe(null);
  });

  it('writes an agent that runs the sync with logs in userData', () => {
    const plist = launchdPlist({ nodePath: '/n/node', scriptPath: '/r/scripts/eroscripts-sync.mjs', workingDirectory: '/r', intervalSeconds: 259200, logPath: '/u/logs/eroscripts-sync.log' });
    expect(plist).toContain(`<string>${LAUNCHD_LABEL}</string>`);
    expect(plist).toContain('<integer>259200</integer>');
    expect(plist).toContain('<string>/r/scripts/eroscripts-sync.mjs</string>');
    expect(plist).toContain('<key>StandardOutPath</key>\n  <string>/u/logs/eroscripts-sync.log</string>');
  });
});
