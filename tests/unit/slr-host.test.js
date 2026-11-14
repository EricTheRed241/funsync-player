/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// SexLikeReal scenes as videos for npm run eroscripts:scout
// (scripts/slr-host.mjs): leads, the title search's length check, the VR
// suffix and scene names, the download choice, the session and quota, and
// how a post with only an SLR link becomes a candidate
// (scripts/eroscripts-scout-lib.mjs). No network: the site, the clock and
// ffprobe are fakes.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, it, expect } from 'vitest';
import {
  slrLinks, namesSlr, slrSearchQueries, slrLeads, pickSearchHit, slrVrSuffix, slrSceneName, slrFile, slrDownloadOptions,
  chooseSlrDownload, rotatedRefreshJwt, createSlr, SlrLoginError, SlrQuotaError, SLR_HOST, SLR_INTERVAL_MS, SLR_SESSION_FILE,
  SLR_ACCOUNT_DOWNLOAD,
} from '../../scripts/slr-host.mjs';
import {
  planWithStreams, sceneNameOf, sceneSidecar, retitlePlan, manualDownloads, manualListText, RULES_VERSION, videoPreference,
} from '../../scripts/eroscripts-scout-lib.mjs';
import { parseManualList, postsToOpen } from '../../scripts/manual-open-lib.mjs';

// A scene's details as `/v3/scenes/<id>` answers them (trimmed).
const detail = (over = {}) => ({
  id: 49489, label: 'sharing-is-caring-a-wifes-surprise-49489', title: 'Sharing is caring : a wife’s surprise',
  studio: { name: 'Miss Lexa VR' }, actors: [{ name: 'Miss Lexa' }, { name: 'Chloe Wild' }], fullVideoLength: 2328, fps: 60,
  isFlatScreenType: false, projection: 0, stereomode: 'sbs2l', screenType: 0, viewAngle: 180,
  projectionParams: { format: 2, viewAngle: 180, projection: 0 },
  ...over,
});
const STEM = 'Miss Lexa, Chloe Wild - Sharing is caring a wife’s surprise (Miss Lexa VR)_180_LR';
// download-options as /content/v1 answers them.
const OPTIONS = [
  { type: 'maxResolution', codec: 'h265', resolution: 3840, height: 3840, width: 7680, size: 6505289695, isAvailableThroughPurchase: false, isDownloadable: true },
  { type: 'trailer', codec: 'h265', resolution: 3840, height: 3840, width: 7680, size: 342169540, isAvailableThroughPurchase: false, isDownloadable: true },
  { type: 'original', codec: 'h265', resolution: 3840, height: 3840, width: 7680, size: 11332146596, isAvailableThroughPurchase: true, isDownloadable: true },
  { type: 'optimalResolution', codec: 'h265', resolution: 2160, height: 2160, width: 4320, size: 2938799465, isAvailableThroughPurchase: false, isDownloadable: true },
];

describe('SLR links and leads', () => {
  it('takes scene pages by label or id, with or without www and query strings, one per scene', () => {
    expect(slrLinks([
      'https://www.sexlikereal.com/scenes/sharing-is-caring-a-wifes-surprise-49489?utm_source=x',
      'https://sexlikereal.com/scenes/49489',
      'http://sexlikereal.com/scenes/77001/#comments',
      'https://www.sexlikereal.com/pornstars/miss-lexa-7954',
      'https://www.sexlikereal.com/scenes',
      'https://www.sexlikereal.com/scenes/no-id-here',
      'https://vrbangers.com/video/x/',
    ])).toEqual([
      { id: 49489, label: 'sharing-is-caring-a-wifes-surprise-49489', url: 'https://www.sexlikereal.com/scenes/sharing-is-caring-a-wifes-surprise-49489' },
      { id: 77001, label: null, url: 'https://www.sexlikereal.com/scenes/77001' },
    ]);
  });

  it('knows a post that names SLR in its title or tags, or sexlikereal in its text', () => {
    expect(namesSlr({ title: '[SLR] Sharing is caring' })).toBe(true);
    expect(namesSlr({ title: 'SexLikeReal - Hot scene' })).toBe(true);
    expect(namesSlr({ title: 'Slurp', tags: ['sex-like-real'] })).toBe(true);
    expect(namesSlr({ title: 'Plain', text: 'video at sexlikereal.com' })).toBe(true);
    expect(namesSlr({ title: 'Slurpy SLRS', tags: ['vr'], text: 'no site' })).toBe(false);
  });

  it('searches only for a post with no SLR link and no video of its own that names SLR', () => {
    const link = 'https://www.sexlikereal.com/scenes/sharing-is-caring-a-wifes-surprise-49489';
    expect(slrLeads({ hrefs: [link], title: 'Whatever' })).toEqual([
      { from: 'link', id: 49489, label: 'sharing-is-caring-a-wifes-surprise-49489', url: link },
    ]);
    expect(slrLeads({ title: '[SLR] Miss Lexa - Sharing is caring' })).toEqual([
      { from: 'search', queries: ['Miss Lexa Sharing is caring', 'Sharing is caring', 'Miss Lexa'] },
    ]);
    expect(slrLeads({ title: '[SLR] Miss Lexa - Sharing is caring', hasVideo: true })).toEqual([]);
    expect(slrLeads({ title: 'Miss Lexa - Sharing is caring' })).toEqual([]);
  });

  it('leaves the post labels out of the search words', () => {
    expect(slrSearchQueries('SexLikeReal | VR | Funscript - Chloe Wild: Morning Glory [8K] (Multi-Axis)'))
      .toEqual(['Chloe Wild Morning Glory', 'Morning Glory', 'Chloe Wild']);
    expect(slrSearchQueries('[SLR] :fire: SLR Originals - Hot Tub')).toEqual(['Hot Tub']);
    expect(slrSearchQueries('[SLR] VR')).toEqual([]);
  });
});

describe('the title search (pickSearchHit)', () => {
  const hit = (id, title, len, actors = ['Chloe Wild']) => ({ id, label: `x-${id}`, title, fullVideoLength: len, actors: actors.map((name) => ({ name })), studio: { name: 'Studio' } });

  it('takes a hit only when the main script fits its length (≤ video + 2 s, ≥ 80 %)', () => {
    const hits = [hit(1, 'Morning Glory', 3000), hit(2, 'Morning Glory', 2000)];
    expect(pickSearchHit(hits, { scriptLengths: [1_900_000], title: 'Morning Glory' })?.id).toBe(2);
    expect(pickSearchHit(hits, { scriptLengths: [2_002_000], title: 'Morning Glory' })?.id).toBe(2);
    expect(pickSearchHit(hits, { scriptLengths: [2_450_000], title: 'Morning Glory' })?.id).toBe(1);
    expect(pickSearchHit(hits, { scriptLengths: [2_003_000], title: 'Morning Glory' })).toBeNull();
    expect(pickSearchHit(hits, { scriptLengths: [1_500_000], title: 'Morning Glory' })).toBeNull();
    expect(pickSearchHit(hits, { scriptLengths: [], title: 'Morning Glory' })).toBeNull();
  });

  it('wants a word shared with the post title, and the hit sharing most', () => {
    const hits = [hit(1, 'Evening Walk', 2000, ['Someone Else']), hit(2, 'Morning Coffee', 2000, ['Someone Else']), hit(3, 'Morning Glory', 2000)];
    expect(pickSearchHit(hits, { scriptLengths: [1_900_000], title: '[SLR] Chloe Wild - Morning Glory' })?.id).toBe(3);
    expect(pickSearchHit(hits.slice(0, 1), { scriptLengths: [1_900_000], title: 'Morning Glory' })).toBeNull();
  });
});

describe('SLR scene names and the pseudo file', () => {
  it('reads the VR suffix the headset understands from the projection', () => {
    expect(slrVrSuffix(detail())).toBe('_180_LR');
    expect(slrVrSuffix(detail({ projection: 4, screenType: 11, viewAngle: 190 }))).toBe('_FISHEYE190');
    expect(slrVrSuffix(detail({ projection: 4, screenType: 6, viewAngle: 200, projectionParams: { cameraLens: 'mkx200' } }))).toBe('_MKX200');
    expect(slrVrSuffix(detail({ screenType: 0, projectionParams: { cameraLens: 'rf52' } }))).toBe('_RF52');
    expect(slrVrSuffix(detail({ screenType: 5, viewAngle: 360 }))).toBe('_360_LR');
    expect(slrVrSuffix(detail({ stereomode: 'tb' }))).toBe('_180_TB');
    expect(slrVrSuffix(detail({ stereomode: 'mono' }))).toBe('_MONO180');
    expect(slrVrSuffix(detail({ isFlatScreenType: true, screenType: 3, stereomode: 'mono' }))).toBe('');
  });

  it('names a scene by performers, title and studio, filed under the first performer', () => {
    expect(slrSceneName({ performers: ['Miss Lexa', 'Chloe Wild'], title: 'Sharing is caring : a wife’s surprise', studio: 'Miss Lexa VR', vrSuffix: '_180_LR' }))
      .toEqual({ creator: 'Miss Lexa', stem: STEM });
    expect(slrSceneName({ performers: [], title: 'Solo', studio: null, vrSuffix: '' })).toEqual({ creator: 'SexLikeReal', stem: 'SexLikeReal - Solo (SexLikeReal)' });
    const many = Array.from({ length: 12 }, (_, i) => `Performer Number ${i + 1}`);
    const { stem } = slrSceneName({ performers: many, title: 'Party', studio: 'Big Studio', vrSuffix: '_MKX200' });
    expect(stem.startsWith('Performer Number 1, ')).toBe(true);
    expect(stem.endsWith('(Big Studio)_MKX200')).toBe(true);
    expect(Buffer.byteLength(stem)).toBeLessThanOrEqual(180 + '_MKX200'.length);
  });

  it('is one video file per scene, size and height left for the fetch', () => {
    expect(slrFile(detail())).toEqual({
      name: `${STEM}.mp4`, host: SLR_HOST, slrSceneId: 49489, slrLabel: 'sharing-is-caring-a-wifes-surprise-49489',
      url: 'https://www.sexlikereal.com/scenes/sharing-is-caring-a-wifes-surprise-49489', size: null, height: null,
      duration: 2328, vr: true, fps: 60, title: 'Sharing is caring : a wife’s surprise', performers: ['Miss Lexa', 'Chloe Wild'],
      studio: 'Miss Lexa VR', vrSuffix: '_180_LR',
    });
    expect(slrFile(detail({ isFlatScreenType: true, screenType: 3 })).name).toBe('Miss Lexa, Chloe Wild - Sharing is caring a wife’s surprise (Miss Lexa VR).mp4');
    expect(videoPreference(`${STEM}.mp4`, 0, true)).toBeGreaterThan(videoPreference('Sharing is caring.mp4', 0, true));
  });

  it('keeps its name in the folder, the sidecar and --retitle', () => {
    const video = slrFile(detail());
    const c = { id: 7, url: 'https://discuss.eroscripts.com/t/7', title: '[SLR] Sharing is caring', author: 'scripter', tags: ['vr'], scenes: [{ video }] };
    expect(sceneNameOf(c, c.scenes[0])).toEqual({ creator: 'Miss Lexa', stem: STEM });
    const rec = sceneSidecar(c, video, `${STEM}.mp4`, 'now');
    expect(rec).toMatchObject({ topicId: 7, video: `${STEM}.mp4`, file: `${STEM}.mp4`, slrSceneId: 49489, slr: { label: video.slrLabel, studio: 'Miss Lexa VR', vrSuffix: '_180_LR' } });
    const incoming = '/lib/EroScripts';
    expect(retitlePlan([{ dir: path.join(incoming, 'Miss Lexa', STEM), files: [`${STEM}.mp4`], rec }], incoming)).toEqual([]);
    const moved = retitlePlan([{ dir: path.join(incoming, 'scripter', 'old'), files: ['old.mp4', 'old.funscript'], rec: { ...rec, file: 'old.mp4' } }], incoming);
    expect(moved[0]).toMatchObject({ to: path.join(incoming, 'Miss Lexa', STEM), video: `${STEM}.mp4` });
  });
});

describe('a post whose only video is on SLR', () => {
  const scripts = [
    { name: 'Sharing is caring.funscript', host: 'forum', url: 'main' },
    { name: 'Sharing is caring.roll.funscript', host: 'forum', url: 'roll' },
  ];

  it('is a scene with the SLR video, not no-video, when the script fits the scene', async () => {
    const plan = await planWithStreams({ scripts, videos: [slrFile(detail())], vr: true, scriptMs: async () => 2_300_000 });
    expect(plan.scenes).toHaveLength(1);
    expect(plan.scenes[0]).toMatchObject({ axes: ['roll'], video: { host: SLR_HOST, slrSceneId: 49489, name: `${STEM}.mp4` } });
  });

  it('drops the scene when the script does not fit its length', async () => {
    const plan = await planWithStreams({ scripts, videos: [slrFile(detail())], vr: true, scriptMs: async () => 900_000 });
    expect(plan.scenes).toEqual([]);
    expect(plan.dropped).toEqual([{ url: 'https://www.sexlikereal.com/scenes/sharing-is-caring-a-wifes-surprise-49489', durationSec: 2328, scriptMs: 900_000 }]);
  });

  it('pairs scenes the names don\'t match with script sets by length', async () => {
    const two = [{ name: 'Part A.funscript', url: 'a' }, { name: 'Part B.funscript', url: 'b' }];
    const a = slrFile(detail({ id: 1, label: 'first-1', title: 'First', fullVideoLength: 1000 }));
    const b = slrFile(detail({ id: 2, label: 'second-2', title: 'Second', fullVideoLength: 2000 }));
    const lengths = { a: 1_950_000, b: 990_000 };
    const plan = await planWithStreams({ scripts: two, videos: [a, b], vr: true, scriptMs: async (f) => lengths[f.url] });
    expect(plan.scenes.map((sc) => [sc.base, sc.video.slrSceneId]).sort()).toEqual([['Part A', 2], ['Part B', 1]]);
  });

  it('is judged again under the rules that count SLR links (v14)', () => {
    expect(RULES_VERSION).toBeGreaterThanOrEqual(14);
  });
});

describe('the download choice', () => {
  it('takes maxResolution within the VR caps, never the trailer or the master', () => {
    expect(chooseSlrDownload(OPTIONS, { vr: true, fps: 60 })).toMatchObject({ type: 'maxResolution', height: 3840, size: 6505289695 });
  });

  it('falls back to optimalResolution over the height cap, and for a flat scene over 4K', () => {
    const big = OPTIONS.map((o) => (o.type === 'maxResolution' ? { ...o, height: 5760, width: 11520 } : o));
    expect(chooseSlrDownload(big, { vr: true })).toMatchObject({ type: 'optimalResolution', height: 2160 });
    const flat = [{ type: 'maxResolution', height: 4320, width: 7680 }, { type: 'optimalResolution', height: 2160, width: 3840 }];
    expect(chooseSlrDownload(flat, { vr: false })).toMatchObject({ type: 'optimalResolution' });
  });

  it('skips options over 60 fps, ones only a purchase gives, and those not downloadable', () => {
    expect(chooseSlrDownload(OPTIONS, { vr: true, fps: 90 })).toBeNull();
    const fps = [{ type: 'maxResolution', height: 3840, codec: 'h265' }, { type: 'optimalResolution', height: 2160, codec: 'h264_30fps' }];
    expect(chooseSlrDownload(fps, { vr: true, fps: 90 })).toMatchObject({ type: 'optimalResolution' });
    const locked = OPTIONS.map((o) => (o.type === 'maxResolution' ? { ...o, isAvailableThroughPurchase: true } : o.type === 'optimalResolution' ? { ...o, isDownloadable: false } : o));
    expect(chooseSlrDownload(locked, { vr: true })).toBeNull();
    expect(chooseSlrDownload(OPTIONS.filter((o) => o.type === 'trailer' || o.type === 'original'), { vr: true })).toBeNull();
  });

  it('reads the /v3 answer keyed by type too', () => {
    const v3 = Object.fromEntries(OPTIONS.map(({ type, ...o }) => [type, o]));
    expect(slrDownloadOptions(v3).map((o) => o.type)).toEqual(['maxResolution', 'trailer', 'original', 'optimalResolution']);
    expect(chooseSlrDownload(v3, { vr: true })).toMatchObject({ type: 'maxResolution' });
  });
});

describe('the SLR host (createSlr)', () => {
  const dirs = [];
  afterEach(() => { for (const d of dirs.splice(0)) fs.rmSync(d, { recursive: true, force: true }); });
  const userDataWith = (session) => {
    const d = fs.mkdtempSync(path.join(os.tmpdir(), 'slr-host-'));
    dirs.push(d);
    if (session) fs.writeFileSync(path.join(d, SLR_SESSION_FILE), JSON.stringify(session));
    return d;
  };
  const json = (body, init = {}) => new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' }, ...init });
  const exp = Math.floor(Date.now() / 1000) + 86_400;
  const TOKEN = `h.${Buffer.from(JSON.stringify({ exp })).toString('base64url')}.s`;
  /** A fake SLR: routes by URL (path + query), records every request. */
  function fakeSlr(routes) {
    const calls = [];
    const fetch = async (url, init = {}) => {
      const u = new URL(url);
      calls.push({ url: String(url), path: u.pathname + u.search, method: init.method || 'GET', headers: init.headers || {} });
      const route = routes[u.pathname + u.search] ?? routes[u.pathname] ?? routes[u.host];
      if (!route) return new Response('missing', { status: 404 });
      return typeof route === 'function' ? route(init) : json(route);
    };
    return { fetch, calls };
  }
  const clock = () => {
    let t = 0;
    return { now: () => t, sleep: async (ms) => { t += ms; } };
  };
  const quota = (remaining) => ({ data: { limits: { downloadVideo: { remaining, total: 35, availableAt: null } } } });
  const account = (remaining = 35, extra = {}) => ({
    '/api/get-access-token': () => json({ accessToken: TOKEN }),
    '/users/v1/user/overview': quota(remaining),
    '/content/v1/videos/49489/download-options': { data: { isVideoPurchased: false, options: OPTIONS } },
    ...extra,
  });

  it('turns a link lead into the scene\'s file, the requests paced', async () => {
    const { fetch, calls } = fakeSlr({ '/v3/scenes/sharing-is-caring-a-wifes-surprise-49489': { data: detail() }, '/v3/scenes/5': { data: detail({ id: 5, label: 'other-5' }) } });
    const c = clock();
    const slr = createSlr({ fetch, userData: '/nowhere', now: c.now, sleep: c.sleep });
    const leads = slrLeads({ hrefs: ['https://www.sexlikereal.com/scenes/sharing-is-caring-a-wifes-surprise-49489', 'https://sexlikereal.com/scenes/404040', 'https://sexlikereal.com/scenes/5'] });
    const r = await slr.filesFor(leads);
    expect(r.files.map((f) => f.slrSceneId)).toEqual([49489, 5]);
    expect(r.missed).toEqual([{ lead: leads[1], why: 'no such scene' }]);
    expect(r.down).toBe(false);
    expect(calls[0].headers).toMatchObject({ 'Client-Type': 'web', project: '1' });
    expect(calls[0].headers.Authorization).toBeUndefined();
    expect(c.now()).toBeGreaterThanOrEqual(2 * SLR_INTERVAL_MS);
  });

  it('accepts a title-search hit only when the main script fits its length', async () => {
    const hits = [
      { id: 900, label: 'teaser-900', title: 'Sharing is caring teaser', fullVideoLength: 60, actors: [{ name: 'Miss Lexa' }] },
      { id: 49489, label: 'sharing-is-caring-a-wifes-surprise-49489', title: 'Sharing is caring : a wife’s surprise', fullVideoLength: 2328, actors: [{ name: 'Miss Lexa' }] },
    ];
    const { fetch, calls } = fakeSlr({ '/v3/search': { data: hits }, '/v3/scenes/sharing-is-caring-a-wifes-surprise-49489': { data: detail() } });
    const slr = createSlr({ fetch, userData: '/nowhere', now: () => 0, sleep: async () => {} });
    const leads = slrLeads({ title: '[SLR] Miss Lexa - Sharing is caring' });
    const ok = await slr.filesFor(leads, { scriptLengths: [2_250_000], title: '[SLR] Miss Lexa - Sharing is caring' });
    expect(ok.files.map((f) => f.name)).toEqual([`${STEM}.mp4`]);
    expect(calls[0].path).toBe(`/v3/search?query=${encodeURIComponent('Miss Lexa Sharing is caring')}&tab=scenes`);
    const none = await slr.filesFor(leads, { scriptLengths: [1_000_000], title: '[SLR] Miss Lexa - Sharing is caring' });
    expect(none.files).toEqual([]);
    expect(none.missed[0].why).toMatch(/fits the script/);
    const unchecked = await slr.filesFor(leads, { title: 'x' });
    expect(unchecked.missed[0].why).toMatch(/no script length/);
  });

  it('says SLR is down (not "no such scene") when it fails', async () => {
    const { fetch } = fakeSlr({ '/v3/scenes/5': () => new Response('oops', { status: 502 }) });
    const slr = createSlr({ fetch, userData: '/nowhere', now: () => 0, sleep: async () => {} });
    const r = await slr.filesFor(slrLeads({ hrefs: ['https://sexlikereal.com/scenes/5'] }));
    expect(r.down).toBe(true);
    expect(r.missed[0].why).toMatch(/HTTP 502/);
  });

  it('is not logged in without a session file, or with an expired one', async () => {
    const none = createSlr({ fetch: fakeSlr({}).fetch, userData: userDataWith(null), now: () => 0, sleep: async () => {} });
    await expect(none.quota()).rejects.toBeInstanceOf(SlrLoginError);
    const { fetch } = fakeSlr({ '/api/get-access-token': () => new Response('{}', { status: 401 }) });
    const expired = createSlr({ fetch, userData: userDataWith({ refreshJwt: 'old' }), now: () => 0, sleep: async () => {} });
    await expect(expired.quota()).rejects.toThrow(/expired.*slr:scout -- --login/);
  });

  it('reads the quota with the session cookie and a bearer token, and saves a renewed cookie (0600)', async () => {
    const ud = userDataWith({ refreshJwt: 'old', expires: '2026-12-13T00:00:00.000Z', savedAt: 'then' });
    const { fetch, calls } = fakeSlr(account(12, {
      '/api/get-access-token': () => new Response(JSON.stringify({ accessToken: TOKEN }), {
        status: 200, headers: [['content-type', 'application/json'], ['set-cookie', 'refresh_jwt=new; Path=/; Max-Age=86400; HttpOnly']],
      }),
    }));
    const slr = createSlr({ fetch, userData: ud, now: () => Date.parse('2026-11-13T00:00:00Z'), sleep: async () => {} });
    expect(await slr.quota()).toEqual({ remaining: 12, total: 35, availableAt: null });
    expect(calls[0]).toMatchObject({ method: 'POST', headers: { Cookie: 'refresh_jwt=old' } });
    expect(calls[1].headers).toMatchObject({ Authorization: `Bearer ${TOKEN}`, 'Client-Type': 'web' });
    const file = path.join(ud, SLR_SESSION_FILE);
    expect(JSON.parse(fs.readFileSync(file, 'utf8'))).toEqual({ refreshJwt: 'new', expires: '2026-11-14T00:00:00.000Z', savedAt: '2026-11-13T00:00:00.000Z' });
    expect(fs.statSync(file).mode & 0o777).toBe(0o600);
    await slr.quota();
    expect(calls.filter((x) => x.path === '/api/get-access-token')).toHaveLength(1);
  });

  it('parses a renewed refresh_jwt, ignoring deletions and other cookies', () => {
    expect(rotatedRefreshJwt(['a=b', 'refresh_jwt=; Max-Age=0', 'refresh_jwt=xyz; Expires=Sun, 13 Dec 2026 10:20:30 GMT'], 0))
      .toEqual({ value: 'xyz', expires: '2026-12-13T10:20:30.000Z' });
    expect(rotatedRefreshJwt(['refresh_jwt=; Max-Age=0'])).toBeNull();
  });

  it('stops at a used-up quota before asking for a download link', async () => {
    const { fetch, calls } = fakeSlr(account(0, { '/content/v1/videos/49489/download': () => { throw new Error('must not be asked'); } }));
    const slr = createSlr({ fetch, userData: userDataWith({ refreshJwt: 'r' }), now: () => 0, sleep: async () => {} });
    await expect(slr.ready()).rejects.toBeInstanceOf(SlrQuotaError);
    await expect(slr.download(slrFile(detail()), '/tmp/never', { videoSeconds: () => 0 })).rejects.toThrow(/no video downloads left \(0 of 35/);
    expect(calls.some((x) => x.path.startsWith('/content/v1/videos/49489/download?'))).toBe(false);
  });

  it('plans the download from the options without asking for a link', async () => {
    const { fetch, calls } = fakeSlr(account());
    const slr = createSlr({ fetch, userData: userDataWith({ refreshJwt: 'r' }), now: () => 0, sleep: async () => {} });
    expect(await slr.plan(slrFile(detail()))).toMatchObject({ type: 'maxResolution', size: 6505289695, height: 3840 });
    expect(calls.map((x) => x.path)).toEqual(['/api/get-access-token', '/content/v1/videos/49489/download-options']);
  });

  it('downloads the chosen type from a fresh link and checks size and length', async () => {
    const ud = userDataWith({ refreshJwt: 'r' });
    const bytes = Buffer.from('video-bytes');
    const options = [{ type: 'maxResolution', height: 3840, width: 7680, size: bytes.length, codec: 'h265' }];
    const { fetch, calls } = fakeSlr(account(5, {
      '/content/v1/videos/49489/download-options': { data: { options } },
      '/content/v1/videos/49489/download?type=maxResolution': { data: { url: 'https://cdn.example/49489.mp4?sig=1', availableDownloadsCount: 4 } },
      'cdn.example': () => new Response(bytes, { status: 200, headers: { 'content-length': String(bytes.length) } }),
    }));
    const slr = createSlr({ fetch, userData: ud, now: () => 0, sleep: async () => {} });
    const tmp = path.join(ud, 'v.part');
    const pick = await slr.download(slrFile(detail()), tmp, { scriptMs: 2_300_000, videoSeconds: () => 2328 });
    expect(pick).toMatchObject({ type: 'maxResolution' });
    expect(fs.readFileSync(tmp)).toEqual(bytes);
    expect(calls.find((x) => x.url.startsWith('https://cdn.example')).headers.Authorization).toBeUndefined();
    await expect(slr.download(slrFile(detail()), tmp, { scriptMs: 2_300_000, videoSeconds: () => 600 })).rejects.toThrow(/not this scene's video/);
    await expect(slr.download(slrFile(detail()), tmp, { scriptMs: 1_000_000, videoSeconds: () => 2328 })).rejects.toThrow(/script 16:40/);
  });
});

describe('--manual-list and SLR videos', () => {
  const pd = (name, id) => ({ name, host: 'pixeldrain', url: `https://pixeldrain.com/api/file/${id}?download` });
  it('lists SLR videos as account downloads, which manual:open leaves alone', () => {
    const video = slrFile(detail());
    const queue = [
      { id: 1, title: 'Only SLR', url: 'https://discuss.eroscripts.com/t/1', scenes: [{ scripts: [{ name: 'a.funscript', host: 'forum' }], video }] },
      { id: 2, title: 'SLR, scripts on pixeldrain', url: 'https://discuss.eroscripts.com/t/2', scenes: [{ scripts: [pd('b.funscript', 'S1')], video }] },
    ];
    const posts = manualDownloads(queue);
    expect(posts.map((p) => [p.id, p.files.map((f) => f.name), p.account.map((f) => f.name)])).toEqual([
      [1, [], [`${STEM}.mp4`]],
      [2, ['b.funscript'], [`${STEM}.mp4`]],
    ]);
    const text = manualListText(posts, { folder: '/d/for-fs', generatedAt: 'now' });
    expect(text).toContain(`   ${SLR_ACCOUNT_DOWNLOAD}: ${STEM}.mp4  ${video.url}`);
    expect(text).toContain('2 posts, 1 files, and 2 SexLikeReal videos the account downloads');
    const open = postsToOpen(parseManualList(text), []);
    expect(open.map((p) => [p.id, p.files.map((f) => f.name)])).toEqual([[2, ['b.funscript']]]);
  });
});
