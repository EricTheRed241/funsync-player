/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// vrbangers.com scenes as videos for npm run eroscripts:scout / vrbangers:scout
// (scripts/vrbangers-host.mjs), their naming and the VRBangers search feed
// (scripts/eroscripts-scout-lib.mjs). No network: the site, Chrome, the forum,
// the clock and ffprobe are fakes.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, it, expect } from 'vitest';
import {
  vrbangersLinks, vrbangersFolders, vrbangersTitleSlugs, vrbangersLeads, vrbangersFile, createVrbangers, VRBANGERS_HOST, VRBANGERS_INTERVAL_MS,
} from '../../scripts/vrbangers-host.mjs';
import { parseScenePage } from '../../scripts/vrbangers-lib.mjs';
import {
  planWithStreams, vrbangersSceneName, sceneNameOf, retitlePlan, feedPage, listFeed, emptyScoutState, ALL_FEEDS,
  isVrbangersCandidate, videoPreference, RULES_VERSION,
} from '../../scripts/eroscripts-scout-lib.mjs';
import { RateLimitedError, SessionExpiredError, SEARCH_INTERVAL_MS } from '../../scripts/eroscripts-sync-lib.mjs';
import { FEED_TAGS, SEARCH_FEEDS } from '../../renderer/js/content-preferences.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SCENE_HTML = fs.readFileSync(path.join(HERE, 'fixtures/vrbangers-scene.html'), 'utf8');
const PAGE = 'https://vrbangers.com/video/anal-virginity/';
const EXP = 1794267292;
const mov = (folder, q, proj = '180x180_3dh') => `https://mov.vrbangers.com/vrb_${folder}/VRBANGERS_${folder}_${q}_${proj}.mp4?secure=TOKEN==,${EXP}`;
const link = (folder, k, extra = {}) => ({
  quality: `${k}K`, k, width: k * 960, file: `VRBANGERS_${folder}_${k}K_180x180_3dh.mp4`, folder, projection: '180x180_3dh',
  size: null, bytes: null, expires: EXP, url: mov(folder, `${k}K`), ...extra,
});
const facts = (over = {}) => ({
  slug: 'one-last-bang', title: 'One Last Bang', performers: ['Mia Malkova', 'Riley Reid'], duration: 2123,
  resolutions: ['2K', '3K', '4K'], fps: 60, degree: '180', format: '3D', folder: 'one_last_bang',
  qualities: ['2K_180x180_3dh', '3K_180x180_3dh', '4K_180x180_3dh'],
  fileSizes: { '2K_180x180_3dh': 1e9, '3K_180x180_3dh': 2e9, '4K_180x180_3dh': 3514643215 },
  ...over,
});

/** A fake clock: sleep moves it on and records the waits. */
function fakeClock() {
  const c = { t: 0, sleeps: [] };
  c.now = () => c.t;
  c.sleep = async (ms) => { c.sleeps.push(ms); c.t += ms; };
  return c;
}

describe('vrbangersLinks', () => {
  it('finds scene pages, one per slug, and skips other site pages', () => {
    expect(vrbangersLinks([
      'https://vrbangers.com/video/anal-virginity/',
      'https://www.vrbangers.com/video/Anal-Virginity/?utm_source=x',
      'http://vrbangers.com/video/one-last-bang',
      'https://vrbangers.com/model/riley-reid/',
      'https://vrbangers.com/videos/',
      'https://notvrbangers.com/video/x/',
      'https://vrbangers.com/video/one-last-bang/#t=1',
    ])).toEqual([
      { slug: 'anal-virginity', url: 'https://vrbangers.com/video/anal-virginity/' },
      { slug: 'one-last-bang', url: 'https://vrbangers.com/video/one-last-bang/' },
    ]);
  });
});

describe('vrbangersLeads', () => {
  it('reads CDN folders from the site-style file names', () => {
    expect(vrbangersFolders([
      'VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.funscript',
      'VRBANGERS_wet_and_wild_in_tulum.twist.funscript',
      'VRBANGERS_wet_and_wild_in_tulum.funscript',
      'VRBANGERS_one_last_bang_HD_180x180_3dh.funscript',
      'vrbangers_lower_case_180x180_3dh.mp4',
      'VR bangers Lexi luna inside the AR conk.funscript',
      'Scene.funscript',
    ])).toEqual(['anal_virginity_remastered', 'wet_and_wild_in_tulum', 'one_last_bang', 'lower_case']);
  });

  it('takes the parts of a title naming VRBangers as slugs, people lists last', () => {
    expect(vrbangersTitleSlugs('VRBangers - Harmony Wonder - Boned in Ukraine')).toEqual(['harmony-wonder', 'boned-in-ukraine']);
    expect(vrbangersTitleSlugs("Vrbangers : A Gentleman's Wish : Katie Kush")).toEqual(['a-gentlemans-wish', 'katie-kush']);
    expect(vrbangersTitleSlugs('[VRBangers] Adira Allure, Julia Robbie - My Milfshake [Multi-Axis] :free:')).toEqual(['my-milfshake', 'adira-allure-julia-robbie']);
    expect(vrbangersTitleSlugs('Riley Reid - Anal Virginity')).toEqual([]);
  });

  it('orders the leads: links, file folders, then the title as a fallback, each slug once', () => {
    expect(vrbangersLeads({
      hrefs: ['https://vrbangers.com/video/one-last-bang/', 'https://vrbangers.com/model/riley-reid/'],
      names: ['VRBANGERS_one_last_bang_HD_180x180_3dh.funscript', 'VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.funscript'],
      title: 'VRBangers - Riley Reid - Anal Virginity',
    })).toEqual([
      { from: 'link', slugs: ['one-last-bang'] },
      { from: 'file', folder: 'anal_virginity_remastered', slugs: ['anal-virginity-remastered', 'anal-virginity'] },
      { from: 'title', slugs: ['riley-reid'], fallback: true },
    ]);
    expect(vrbangersLeads({ title: 'Some animation' })).toEqual([]);
  });
});

describe('vrbangersFile', () => {
  it('is the best quality within the VR cap, named like the CDN file, with size and length from the public page', () => {
    const f = vrbangersFile(PAGE, parseScenePage(SCENE_HTML, 'anal-virginity'));
    expect(f).toEqual({
      name: 'VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.mp4', size: 19301406842, host: VRBANGERS_HOST, url: PAGE,
      slug: 'anal-virginity', duration: 3141, quality: '8K', projection: '180x180_3dh', title: 'Anal Virginity (Remastered)', performers: ['Riley Reid'],
    });
  });

  it('leaves out qualities over 8K, prefers stereo 180 to mono 360, and falls back to the slug folder', () => {
    const f = vrbangersFile('u', facts({
      folder: null, qualities: ['4K_mono360', '6K_180x180_3dh', '8K_180x180_3dh', '12K_180x180_3dh'], fileSizes: { '8K_180x180_3dh': 42 },
    }));
    expect(f).toMatchObject({ name: 'VRBANGERS_one_last_bang_8K_180x180_3dh.mp4', size: 42, quality: '8K' });
    expect(vrbangersFile('u', facts({ qualities: ['2K_mono360', '4K_mono360'] })).name).toBe('VRBANGERS_one_last_bang_4K_mono360.mp4');
  });

  it('uses the Resolution list when the page has no data block, size unknown', () => {
    const f = vrbangersFile('u', facts({ qualities: [], fileSizes: {}, resolutions: ['2K', '4K'] }));
    expect(f).toMatchObject({ name: 'VRBANGERS_one_last_bang_4K_180x180_3dh.mp4', size: 0, projection: '180x180_3dh' });
  });

  it('is null when nothing fits the caps (only 12K, or over 60 fps)', () => {
    expect(vrbangersFile('u', facts({ qualities: ['12K_180x180_3dh'] }))).toBeNull();
    expect(vrbangersFile('u', facts({ fps: 120 }))).toBeNull();
  });

  it('counts as a VR video when the post is VR (projection suffix)', () => {
    expect(videoPreference('VRBANGERS_x_8K_180x180_3dh.mp4', 0, true)).toBeGreaterThan(videoPreference('Some Scene 4K.mp4', 0, true));
  });
});

describe('VRBangers videos and the post scripts (planWithStreams)', () => {
  const scripts = [
    { name: 'Wet and Wild In Tulum.funscript', host: 'forum', url: 'main' },
    { name: 'Wet and Wild In Tulum.twist.funscript', host: 'forum', url: 'twist' },
  ];
  const scene = (duration) => vrbangersFile('https://vrbangers.com/video/wet-and-wild-in-tulum/', facts({
    slug: 'wet-and-wild-in-tulum', folder: 'wet_and_wild_in_tulum', duration, qualities: ['8K_180x180_3dh'], fileSizes: {},
  }));

  it('pairs a script that stops before the credits (≥ 80 % of the scene)', async () => {
    const plan = await planWithStreams({ scripts, videos: [scene(3000)], vr: true, scriptMs: async () => 2_500_000 });
    expect(plan.scenes).toHaveLength(1);
    expect(plan.scenes[0]).toMatchObject({ axes: ['twist'], video: { host: VRBANGERS_HOST, name: 'VRBANGERS_wet_and_wild_in_tulum_8K_180x180_3dh.mp4' } });
  });

  it('drops a scene whose length the script does not fit', async () => {
    const plan = await planWithStreams({ scripts, videos: [scene(3000)], vr: true, scriptMs: async () => 1_200_000 });
    expect(plan.scenes).toEqual([]);
    expect(plan.dropped).toEqual([{ url: 'https://vrbangers.com/video/wet-and-wild-in-tulum/', durationSec: 3000, scriptMs: 1_200_000 }]);
  });
});

describe('VRBangers scene names', () => {
  it('names the folder by the first performer and the scene by performers, title and projection', () => {
    expect(vrbangersSceneName({ performers: ['Riley Reid'], title: 'Anal Virginity (Remastered)', projection: '180x180_3dh' }))
      .toEqual({ creator: 'Riley Reid', stem: 'Riley Reid - Anal Virginity (VRBangers)_180x180_3dh' });
    expect(vrbangersSceneName({ performers: ['Mia Malkova', 'Riley Reid'], title: 'One Last Bang', projection: '180x180_3dh' }))
      .toEqual({ creator: 'Mia Malkova', stem: 'Mia Malkova, Riley Reid - One Last Bang (VRBangers)_180x180_3dh' });
  });

  it('keeps bracket parts that are not encode words, and falls back to the slug and the studio', () => {
    expect(vrbangersSceneName({ performers: ['Lena Paul'], title: 'Entertainment Tax (Part 2) [8K]', projection: '180x180_3dh' }).stem)
      .toBe('Lena Paul - Entertainment Tax (Part 2) (VRBangers)_180x180_3dh');
    expect(vrbangersSceneName({ performers: [], title: null, slug: 'super-hole-lvi', projection: 'mono360' }))
      .toEqual({ creator: 'VRBangers', stem: 'VRBangers - super hole lvi (VRBangers)_mono360' });
  });

  it('drops performers from the end when the name gets too long', () => {
    const many = Array.from({ length: 12 }, (_, i) => `Performer Number ${i + 1}`);
    const { creator, stem } = vrbangersSceneName({ performers: many, title: 'World Cup 2026', projection: '180x180_3dh' });
    expect(creator).toBe('Performer Number 1');
    expect(Buffer.byteLength(stem)).toBeLessThanOrEqual(180 + '_180x180_3dh'.length);
    expect(stem).toMatch(/^Performer Number 1, .* - World Cup 2026 \(VRBangers\)_180x180_3dh$/);
  });

  it('uses the site name for a VRBangers scene and the post name otherwise', () => {
    const c = { title: 'VRBangers - Riley Reid - Anal Virginity (Remastered)', author: 'crwxaj', scenes: [{}] };
    const video = vrbangersFile(PAGE, parseScenePage(SCENE_HTML, 'anal-virginity'));
    expect(sceneNameOf(c, { video })).toEqual({ creator: 'Riley Reid', stem: 'Riley Reid - Anal Virginity (VRBangers)_180x180_3dh' });
    expect(sceneNameOf(c, { video: { name: 'anal.mp4', host: 'mega' } }).creator).not.toBe('Riley Reid');
    expect(isVrbangersCandidate({ scenes: [{ video: { host: 'mega' } }, { video }] })).toBe(true);
    expect(isVrbangersCandidate({ scenes: [{ video: { host: 'mega' } }] })).toBe(false);
  });

  it('--retitle leaves a fetched VRBangers scene under its site name', () => {
    const stem = 'Riley Reid - Anal Virginity (VRBangers)_180x180_3dh';
    const dir = path.join('/lib/EroScripts', 'Riley Reid', stem);
    const rec = {
      topicId: 161227, title: 'VRBangers - Riley Reid - Anal Virginity (Remastered)', author: 'crwxaj', scenes: 1,
      video: 'VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.mp4', file: `${stem}.mp4`,
      vrbangers: { slug: 'anal-virginity', title: 'Anal Virginity (Remastered)', performers: ['Riley Reid'], projection: '180x180_3dh' },
    };
    expect(retitlePlan([{ dir, rec, files: [`${stem}.mp4`, `${stem}.funscript`, 'eroscripts.json'] }], '/lib/EroScripts')).toEqual([]);
  });
});

describe('the vrbangers search feed', () => {
  function fakeForum(pages) {
    const calls = [];
    const api = {
      async listFreeSearchTopics(q, page) { calls.push(['search', q, page]); return pages[page] || { topics: [], more: false }; },
      async listFreeTagTopics(tag, page) { calls.push(['tag', tag, page]); return { topics: [], more: false }; },
    };
    return { api, calls };
  }
  const topic = (id, bumped = '2026-09-01T00:00:00Z') => ({ id, title: `VRBangers - Scene ${id}`, tags: ['vr', 'real'], likes: 1, bumped });

  it('is a feed --feeds takes, listed by forum search, with its own backfill cursor', async () => {
    expect(ALL_FEEDS).toEqual([...FEED_TAGS, 'vrbangers', 'slr']);
    expect(SEARCH_FEEDS.vrbangers).toBe('vrbangers');
    const { api, calls } = fakeForum({
      0: { topics: [topic(338156), topic(336077)], more: true },
      1: { topics: [topic(321639)], more: true },
      2: { topics: [topic(170767)], more: false },
    });
    const waits = { forum: 0, search: 0 };
    const listPage = feedPage(api, { pacer: { wait: async () => { waits.forum++; } }, searchPacer: { wait: async () => { waits.search++; } } });
    const state = emptyScoutState();
    state.feeds['multi-axis'] = { backfillPage: 7, exhausted: false };
    const r = await listFeed(state, 'vrbangers', listPage, { maxNew: 1, backfill: 1, keepPending: true });
    expect(r.topics.map((t) => t.id)).toEqual([338156, 336077, 321639]);
    expect(calls).toEqual([['search', 'vrbangers', 0], ['search', 'vrbangers', 1]]);
    expect(state.feeds.vrbangers).toMatchObject({ backfillPage: 2, exhausted: false });
    expect(state.feeds['multi-axis'].backfillPage).toBe(7);
    expect(Object.keys(state.pending).map(Number).sort()).toEqual([321639, 336077, 338156]);
    expect(waits).toEqual({ forum: 2, search: 2 });

    const next = await listFeed(state, 'vrbangers', listPage, { maxNew: 0, backfill: 2 });
    expect(next.topics.map((t) => t.id)).toEqual([170767]);
    expect(state.feeds.vrbangers.exhausted).toBe(true);
  });

  it('tag feeds still read their tag, and searches are paced at least 750 ms apart', async () => {
    const { api, calls } = fakeForum({});
    const searched = [];
    await feedPage(api, { pacer: { wait: async () => {} }, searchPacer: { wait: async () => { searched.push(1); } } })('multi-axis', 3);
    expect(calls).toEqual([['tag', 'multi-axis', 3]]);
    expect(searched).toEqual([]);
    expect(SEARCH_INTERVAL_MS).toBeGreaterThanOrEqual(750);
  });

  it('turns forum errors into the scout errors', async () => {
    const pacer = { wait: async () => {} };
    const answer = (r) => feedPage({ listFreeSearchTopics: async () => r }, { pacer, searchPacer: pacer });
    await expect(answer({ topics: [], rateLimited: true })('vrbangers', 0)).rejects.toBeInstanceOf(RateLimitedError);
    await expect(answer({ topics: [], authExpired: true })('vrbangers', 0)).rejects.toBeInstanceOf(SessionExpiredError);
    await expect(answer({ topics: [], error: 'boom' })('vrbangers', 0)).rejects.toThrow('boom');
  });

  it('judges every post again under the new rules (no-video verdicts change)', () => {
    expect(RULES_VERSION).toBeGreaterThanOrEqual(12);
  });
});

describe('createVrbangers', () => {
  const tmpDirs = [];
  afterEach(() => { for (const d of tmpDirs.splice(0)) fs.rmSync(d, { recursive: true, force: true }); });
  const tmpFile = () => {
    const d = fs.mkdtempSync(path.join(os.tmpdir(), 'vrb-host-'));
    tmpDirs.push(d);
    return path.join(d, 'scene.mp4.part');
  };

  /** A fake Chrome session and CDN; `events` records the order of link reads and downloads. */
  function fakeSite({ links = () => [link('one_last_bang', 4, { bytes: 11 }), link('one_last_bang', 2)], login = async () => {}, body = 'video bytes', status = 200 } = {}) {
    const events = [];
    const browsers = [];
    const open = async (o) => {
      const b = {
        options: o, closed: 0, userAgent: 'Mozilla/5.0 Chrome/142',
        async ensureLoggedIn() { events.push('login'); await login(); },
        async sceneLinks(slug) { events.push(`links ${slug}`); return links(); },
        async close() { b.closed++; events.push('close'); },
      };
      browsers.push(b);
      events.push('open');
      return b;
    };
    const fetch = async (url, init = {}) => {
      events.push(init.headers?.Range ? `probe ${url.split('?')[0].split('/').pop()}` : `get ${url.split('?')[0].split('/').pop()}`);
      if (init.headers?.Range) return new Response('x', { status: 206, headers: { 'content-range': 'bytes 0-0/3514643215' } });
      return new Response(body, { status, headers: { 'content-length': String(Buffer.byteLength(body)) } });
    };
    const clock = fakeClock();
    const host = createVrbangers({ open, fetch, now: clock.now, sleep: clock.sleep, userData: '/ud', credentialsFile: '/creds.txt' });
    return { host, events, browsers, clock };
  }
  const planned = { name: 'VRBANGERS_one_last_bang_4K_180x180_3dh.mp4', size: 11, host: VRBANGERS_HOST, slug: 'one-last-bang', duration: 2123 };

  /** The public site: the anal-virginity page at its slug (or at every slug: a soft "not found"), 404 otherwise. */
  function publicSite({ everywhere = false } = {}) {
    const at = [];
    const clock = fakeClock();
    const host = createVrbangers({
      open: async () => { throw new Error('no browser at scan time'); },
      fetch: async (url) => {
        at.push([clock.t, url]);
        return everywhere || url === PAGE ? new Response(SCENE_HTML, { status: 200 }) : new Response('', { status: 404 });
      },
      now: clock.now, sleep: clock.sleep,
    });
    return { host, at };
  }

  it('finds a scene from its file names through the paced public page (≥ 1 s apart, no browser)', async () => {
    const { host, at } = publicSite();
    const leads = vrbangersLeads({ names: ['VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.funscript'], title: 'VRBangers - Riley Reid - Anal Virginity (Remastered)' });
    const { files, missed } = await host.filesFor(leads);
    expect(files.map((f) => [f.name, f.url])).toEqual([['VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.mp4', PAGE]]);
    expect(missed).toEqual([]);
    // The remaster's own slug first, then the plain one; the title fallback isn't needed.
    expect(at).toEqual([[0, 'https://vrbangers.com/video/anal-virginity-remastered/'], [VRBANGERS_INTERVAL_MS, PAGE]]);
    expect(VRBANGERS_INTERVAL_MS).toBeGreaterThanOrEqual(1000);
  });

  it('finds a scene from the title when the post names no page or file', async () => {
    const { host } = publicSite();
    const { files } = await host.filesFor(vrbangersLeads({ title: 'VRBangers - Riley Reid - Anal Virginity' }));
    expect(files.map((f) => f.slug)).toEqual(['anal-virginity']);
  });

  it('rejects a page for another folder or another title, and reports missing linked scenes', async () => {
    const soft = publicSite({ everywhere: true });
    const other = await soft.host.filesFor([{ from: 'file', folder: 'one_last_bang', slugs: ['anal-virginity'] }]);
    expect(other.files).toEqual([]);
    expect(other.missed[0].why).toMatch(/folder anal_virginity_remastered/);
    const guessed = await soft.host.filesFor(vrbangersLeads({ title: 'VRBangers - Riley Reid - Bad Date' }));
    expect(guessed.files).toEqual([]);
    expect(guessed.missed[0].why).toMatch(/is "Anal Virginity/);

    const { host } = publicSite();
    const gone = await host.filesFor(vrbangersLeads({ hrefs: ['https://vrbangers.com/video/new-star-is-cuming/'] }));
    expect(gone).toEqual({ files: [], missed: [{ lead: { from: 'link', slugs: ['new-star-is-cuming'] }, why: 'no such scene' }] });
  });

  it('opens Chrome once, logs in once, and reads a fresh link right before each download', async () => {
    const { host, events, browsers } = fakeSite();
    const seconds = [];
    const videoSeconds = (file) => { seconds.push(file); return 2120; };
    const tmp1 = tmpFile();
    const tmp2 = tmpFile();
    const got = await host.download(planned, tmp1, { scriptMs: 2_000_000, videoSeconds });
    await host.download({ ...planned, slug: 'other-scene' }, tmp2, { videoSeconds });
    expect(got.file).toBe(planned.name);
    expect(fs.readFileSync(tmp1, 'utf8')).toBe('video bytes');
    expect(seconds).toEqual([tmp1, tmp2]);
    expect(events).toEqual([
      'open', 'login', 'links one-last-bang', `get ${planned.name}`,
      'links other-scene', `get ${planned.name}`,
    ]);
    expect(browsers).toHaveLength(1);
    expect(browsers[0].options).toMatchObject({ userData: '/ud', credentialsFile: '/creds.txt' });
    await host.close();
    expect(browsers[0].closed).toBe(1);
  });

  it('streams with the browser user agent and no cookie', async () => {
    const seen = [];
    const host = createVrbangers({
      open: async () => ({ userAgent: 'Mozilla/5.0 Chrome/142', ensureLoggedIn: async () => {}, sceneLinks: async () => [link('one_last_bang', 4)], close: async () => {} }),
      fetch: async (url, init) => { seen.push(init.headers); return new Response('video bytes', { status: 200 }); },
      now: () => 0, sleep: async () => {},
    });
    await host.download({ ...planned, size: 0 }, tmpFile(), { videoSeconds: () => 2123 });
    expect(seen).toEqual([{ 'User-Agent': 'Mozilla/5.0 Chrome/142' }]);
  });

  it('falls back to the best file within the caps when the planned one is gone', async () => {
    const { host, events } = fakeSite({ links: () => [link('one_last_bang', 12), link('one_last_bang', 6, { bytes: 11 }), link('one_last_bang', 2)] });
    const got = await host.download(planned, tmpFile(), { videoSeconds: () => 2123 });
    expect(got).toMatchObject({ quality: '6K', file: 'VRBANGERS_one_last_bang_6K_180x180_3dh.mp4' });
    expect(events.at(-1)).toBe('get VRBANGERS_one_last_bang_6K_180x180_3dh.mp4');
  });

  it('checks the size and the length of the download', async () => {
    const short = fakeSite({ body: 'video' });
    await expect(short.host.download(planned, tmpFile(), { videoSeconds: () => 2123 })).rejects.toThrow(/size 5 ≠ 11/);
    const teaser = fakeSite();
    await expect(teaser.host.download(planned, tmpFile(), { scriptMs: 2_100_000, videoSeconds: () => 60 })).rejects.toThrow(/not this scene's video/);
    const unreadable = fakeSite();
    await expect(unreadable.host.download(planned, tmpFile(), { videoSeconds: () => 0 })).rejects.toThrow(/readable video/);
    const failed = fakeSite({ status: 403 });
    await expect(failed.host.download(planned, tmpFile(), { videoSeconds: () => 2123 })).rejects.toThrow(/HTTP 403/);
    const wrongScene = fakeSite();
    await expect(wrongScene.host.download(planned, tmpFile(), { videoSeconds: () => 600 })).rejects.toThrow(/page 35:23; not this scene's video/);
  });

  it('refuses a planned scene the script does not fit before reading any link', async () => {
    const { host, events } = fakeSite();
    await expect(host.download(planned, tmpFile(), { scriptMs: 600_000, videoSeconds: () => 2123 })).rejects.toThrow(/not this scene's video/);
    expect(events).toEqual([]);
  });

  it('probes the signed link for one byte without downloading', async () => {
    const { host, events } = fakeSite();
    const r = await host.probe(planned);
    expect(r).toMatchObject({ status: 206, total: 3514643215, link: { file: planned.name } });
    expect(events).toEqual(['open', 'login', 'links one-last-bang', `probe ${planned.name}`]);
  });

  it('closes Chrome when the pass fails, and when the login fails', async () => {
    const { host, browsers } = fakeSite();
    await expect(host.run(async () => {
      await host.probe(planned);
      throw new Error('disk full');
    })).rejects.toThrow('disk full');
    expect(browsers[0].closed).toBe(1);

    class LoginError extends Error { exitCode = 3; }
    const refused = fakeSite({ login: async () => { throw new LoginError('VRBangers login failed'); } });
    await expect(refused.host.run(() => refused.host.download(planned, tmpFile(), { videoSeconds: () => 2123 }))).rejects.toBeInstanceOf(LoginError);
    expect(refused.browsers[0].closed).toBe(1);
    expect(refused.events.filter((e) => e.startsWith('get'))).toEqual([]);
  });

  it('starts no Chrome for a pass without VRBangers scenes', async () => {
    const { host, browsers } = fakeSite();
    expect(await host.run(async () => 'nothing to do')).toBe('nothing to do');
    expect(browsers).toEqual([]);
  });
});
