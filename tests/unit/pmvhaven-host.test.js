/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// pmvhaven.com videos as video files for npm run eroscripts:scout
// (scripts/stream-hosts.mjs): the links a post holds, the API's video, the
// pseudo file, the paced client, pairing by length (planWithStreams), the
// sidecar and the frames source. No network: fetch, clock and sleep are fakes.
import { describe, it, expect } from 'vitest';
import {
  pmvhavenLinks, pmvhavenId, parsePmvhavenVideo, pmvhavenFiles, createPmvhaven, PMVHAVEN_HOST, PMVHAVEN_INTERVAL_MS,
} from '../../scripts/stream-hosts.mjs';
import { planWithStreams, sceneSidecar, STREAM_HOSTS, RULES_VERSION } from '../../scripts/eroscripts-scout-lib.mjs';
import { frameSource } from '../../scripts/scout-frames.mjs';

const ID = '6afa5655d00462f31106988b';
const OLD = '664220ad0dcc86a608aa8984';
const MP4 = 'https://pmvhavencloud.s3.eu-west-par.io.cloud.ovh.net/videos/TheGoon-Bot_-_Pink_Orbit_1794790997946_xy15lknd.mp4';

/** `GET /api/videos/<id>` as pmvhaven.com answers it (2026-11), trimmed. */
const apiVideo = (extra = {}) => ({
  success: true,
  data: {
    _id: ID, title: 'Pink Orbit', uploader: 'TheGoon-Bot', videoUrl: MP4, durationSeconds: 205, width: 1920, height: 1080,
    fileSize: 180279164, originalCreator: '', tags: ['Hypno'], ...extra,
  },
});

describe('pmvhavenLinks', () => {
  it('takes video pages, today\'s and the old site\'s, one per id, in link order', () => {
    expect(pmvhavenLinks([
      `https://pmvhaven.com/video/pink-orbit_${ID}?from=home`,
      'https://pmvhaven.com/profile/TheGoon-Bot',
      `https://www.pmvhaven.com/video/Goon-Edge-Leak-Repeat_${OLD}#t=3`,
      `https://pmvhaven.com/video/pink_orbit_again_${ID}`,
      'https://pmvhaven.com/search?q=goon',
      'https://pmvhaven.com/video/not-an-id_1234',
      `https://notpmvhaven.com/video/x_${ID}`,
    ])).toEqual([
      `https://pmvhaven.com/video/pink-orbit_${ID}`,
      `https://pmvhaven.com/video/Goon-Edge-Leak-Repeat_${OLD}`,
    ]);
    expect(pmvhavenLinks([`https://pmvhaven.com/video/${ID.toUpperCase()}`])).toEqual([`https://pmvhaven.com/video/${ID}`]);
  });

  it('pmvhavenId reads the id, null for anything else', () => {
    expect(pmvhavenId(`https://pmvhaven.com/video/pink-orbit_${ID}`)).toBe(ID);
    expect(pmvhavenId('https://pmvhaven.com/video/pink-orbit')).toBeNull();
    expect(pmvhavenId(null)).toBeNull();
  });
});

describe('parsePmvhavenVideo', () => {
  it('title, length, frame size, public mp4 and size', () => {
    expect(parsePmvhavenVideo(apiVideo({ title: 'Pink &amp; Orbit' }))).toEqual({
      id: ID, title: 'Pink & Orbit', durationSec: 205, width: 1920, height: 1080, direct: MP4, size: 180279164, uploader: 'TheGoon-Bot',
    });
  });

  it('null without a playable https mp4; unknown numbers are 0 / null', () => {
    expect(parsePmvhavenVideo(apiVideo({ videoUrl: null }))).toBeNull();
    expect(parsePmvhavenVideo(apiVideo({ videoUrl: 'http://insecure/x.mp4' }))).toBeNull();
    expect(parsePmvhavenVideo({ error: true, statusCode: 404 })).toBeNull();
    expect(parsePmvhavenVideo(apiVideo({ durationSeconds: null, fileSize: undefined, width: 'x' }))).toMatchObject({ durationSec: null, size: 0, width: 0 });
  });
});

describe('pmvhavenFiles', () => {
  it('one pseudo file named after the title and height, the page kept, not the mp4', () => {
    const page = `https://pmvhaven.com/video/pink-orbit_${ID}`;
    expect(pmvhavenFiles(page, parsePmvhavenVideo(apiVideo({ title: 'Pink Orbit [4K 60fps]' })))).toEqual([{
      name: 'Pink Orbit 1080p.mp4', size: 180279164, host: PMVHAVEN_HOST, url: page, height: 1080, duration: 205, pmvhavenId: ID,
    }]);
    const [untitled] = pmvhavenFiles(page, parsePmvhavenVideo(apiVideo({ title: '', height: 0 })));
    expect(untitled.name).toBe(`pmvhaven ${ID}.mp4`);
  });
});

/** A fake clock: sleep moves it on and records the waits. */
function fakeClock() {
  const c = { t: 0, sleeps: [] };
  c.now = () => c.t;
  c.sleep = async (ms) => { c.sleeps.push(ms); c.t += ms; };
  return c;
}
const response = (status, { body = '', headers = {} } = {}) => new Response(body, { status, headers });

describe('createPmvhaven', () => {
  function fakeSite(video = apiVideo()) {
    const calls = [];
    const clock = fakeClock();
    const fetch = async (url, init = {}) => {
      calls.push({ url, headers: init.headers || {}, at: clock.t });
      if (url.includes('/api/videos/000000000000000000000000')) return response(404, { body: '{"error":true}' });
      if (url.includes('/api/videos/')) return response(200, { body: JSON.stringify(video), headers: { 'content-type': 'application/json' } });
      if (init.headers?.Range === 'bytes=0-0') return response(206, { body: 'x', headers: { 'content-range': 'bytes 0-0/180279164' } });
      return response(200, { body: 'video bytes' });
    };
    return { calls, site: createPmvhaven({ fetch, now: clock.now, sleep: clock.sleep }) };
  }

  it('lists a page from the API, reads it again for the download, ≥ 1 s apart', async () => {
    const { calls, site } = fakeSite();
    const page = `https://pmvhaven.com/video/pink-orbit_${ID}`;
    const [file] = await site.files(page);
    expect(file).toMatchObject({ name: 'Pink Orbit 1080p.mp4', size: 180279164, duration: 205 });
    expect(calls[0].url).toBe(`https://pmvhaven.com/api/videos/${ID}`);
    expect(calls[0].headers['User-Agent']).toMatch(/Mozilla/);
    expect(await site.fresh(file)).toEqual({ direct: MP4, durationSec: 205, size: 180279164 });
    expect(await site.probeSize(file)).toBe(180279164);
    expect(calls.at(-1)).toMatchObject({ url: MP4, headers: { Range: 'bytes=0-0' } });
    expect(await (await site.stream(MP4)).text()).toBe('video bytes');
    const gaps = calls.slice(1).map((c, i) => c.at - calls[i].at);
    expect(gaps.every((g) => g >= PMVHAVEN_INTERVAL_MS)).toBe(true);
    expect(PMVHAVEN_INTERVAL_MS).toBeGreaterThanOrEqual(1000);
  });

  it('a gone video or one without an mp4 fails with status 404 (the post is no-video, not "host down")', async () => {
    const { site } = fakeSite(apiVideo({ videoUrl: '' }));
    await expect(site.files(`https://pmvhaven.com/video/x_${ID}`)).rejects.toMatchObject({ status: 404 });
    await expect(site.files('https://pmvhaven.com/video/x_000000000000000000000000')).rejects.toMatchObject({ status: 404, message: expect.stringMatching(/HTTP 404/) });
    const down = createPmvhaven({ fetch: async () => response(503), now: () => 0, sleep: async () => {} });
    await expect(down.files(`https://pmvhaven.com/video/x_${ID}`)).rejects.toMatchObject({ status: 503 });
  });
});

describe('a post linking pmvhaven', () => {
  const page = `https://pmvhaven.com/video/pink-orbit_${ID}`;
  const file = (durationSec, id = ID) => pmvhavenFiles(`https://pmvhaven.com/video/x_${id}`, parsePmvhavenVideo(apiVideo({ _id: id, durationSeconds: durationSec, title: 'Pink Orbit' })))[0];
  const scripts = [{ name: 'Some Other Name.funscript', host: 'forum', url: 'main' }];

  it('is a streaming host, judged again under the rules that count it (v15)', () => {
    expect(STREAM_HOSTS.has(PMVHAVEN_HOST)).toBe(true);
    expect(RULES_VERSION).toBeGreaterThanOrEqual(15);
  });

  it('pairs its script with the video of the right length, a teaser dropped', async () => {
    const plan = await planWithStreams({ scripts, videos: [file(30, OLD), file(205)], scriptMs: async () => 204_000 });
    expect(plan.scenes).toHaveLength(1);
    expect(plan.scenes[0].video).toMatchObject({ host: PMVHAVEN_HOST, pmvhavenId: ID, size: 180279164 });
    const none = await planWithStreams({ scripts, videos: [file(30)], scriptMs: async () => 204_000 });
    expect(none.scenes).toEqual([]);
  });

  it("keeps the video's id and page in eroscripts.json, and frames come from its public mp4", () => {
    const video = pmvhavenFiles(page, parsePmvhavenVideo(apiVideo()))[0];
    const post = { id: 1, url: 'https://discuss.eroscripts.com/t/x/1', title: 'Pink Orbit', author: 'a', tags: [], scenes: [{ video, scripts: [], durationMs: 204_000 }] };
    const rec = sceneSidecar(post, video, 'a - Pink Orbit.mp4', 'T');
    expect(rec.pmvhaven).toEqual({ id: ID, url: page });
    expect(rec).not.toHaveProperty('posterUrl');
    expect(frameSource(post)).toMatchObject({ kind: 'pmvhaven', host: PMVHAVEN_HOST, url: page, id: ID, lengthMs: 205_000, layout: 'flat' });
  });
});
