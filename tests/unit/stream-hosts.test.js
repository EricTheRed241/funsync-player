/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// rule34video pages as video files for npm run eroscripts:scout
// (scripts/stream-hosts.mjs) and the length check that pairs them with a
// post's scripts (planWithStreams in scripts/eroscripts-scout-lib.mjs).
// No network: fetch, clock and sleep are fakes.
import { describe, it, expect } from 'vitest';
import {
  rule34videoLinks, parseRule34videoPage, rule34videoFiles, streamTitle, labelHeight, isoDurationSec, durationMatches,
  pacedFetch, createRule34video, rule34videoPosterUrl, RULE34VIDEO_INTERVAL_MS, RATE_LIMIT_BACKOFF_MS,
} from '../../scripts/stream-hosts.mjs';
import { planWithStreams, planScenes, sceneSidecar } from '../../scripts/eroscripts-scout-lib.mjs';

const GET = (id, q) => `https://rule34video.com/get_file/58/abc${q}/3122000/${id}/${id}_${q}.mp4/?v-acctoken=TOKEN${q}`;

/** A KVS page like rule34video's (2026-11): JSON-LD and the player's flashvars. */
function page({ title = 'Yuffie&#039;s Interrogation Techniques 4K - Nagoonimation', duration = 'PT0H3M21S', id = 3122623, alts = true } = {}) {
  const flash = [
    `video_id: '${id}'`,
    `video_title: '${title.replace(/'/g, "\\'")}'`,
    `event_reporting2: 'https://rule34video.com/get_file/58/x/3122000/${id}/${id}.mp4/?v-acctoken=EV'`,
    `video_url: '${GET(id, '360')}'`,
    "postfix: '_360.mp4'",
    alts ? "video_url_text: '360p'" : '',
    ...(alts ? [
      `video_alt_url: '${GET(id, '480p')}'`, "video_alt_url_text: '480p'",
      `video_alt_url2: '${GET(id, '720p')}'`, "video_alt_url2_text: '720p'", "video_alt_url2_hd: '1'",
      `video_alt_url3: '${GET(id, '1080p')}'`, "video_alt_url3_text: '1080p'",
      `video_alt_url4: '${GET(id, '2160p')}'`, "video_alt_url4_text: '4k'", "video_alt_url4_4k: '1'",
    ] : []),
    `timeline_screens_url: 'https://rule34video.com/contents/videos_screenshots/3122000/${id}/timelines/{time}.jpg'`,
  ].filter(Boolean).join(', ');
  return `<html><head><title>${title}</title>
<script type="application/ld+json">
{ "@context": "https://schema.org", "@type": "VideoObject", "name": "${title.replace(/&#039;/g, "'")}",
  "description": "Support The Artist: https:\\/\\/www.patreon.com\\/x", "duration": "${duration}" }
</script></head><body><script>var flashvars = { ${flash} };</script></body></html>`;
}

describe('rule34videoLinks', () => {
  it('takes video pages, not profiles or searches, one per video id', () => {
    expect(rule34videoLinks([
      'https://rule34video.com/video/3122623/yuffie-s-interrogation/#comments',
      'https://www.rule34video.com/video/3122623/other-slug/',
      'https://rule34video.com/members/123/',
      'https://rule34video.com/search/agapilord/',
      'http://rule34video.com/video/3291859/tifa?utm=x',
      'https://rule34video.com/video/4000000',
      'https://notrule34video.com/video/1/x/',
      'https://mega.nz/file/abc#key',
    ])).toEqual([
      'https://rule34video.com/video/3122623/yuffie-s-interrogation/',
      'https://rule34video.com/video/3291859/tifa/',
      'https://rule34video.com/video/4000000/video/',
    ]);
  });
});

describe('parseRule34videoPage', () => {
  it('reads the title, the JSON-LD length and every quality, 4k as 2160', () => {
    const p = parseRule34videoPage(page());
    expect(p.title).toBe("Yuffie's Interrogation Techniques 4K - Nagoonimation");
    expect(p.durationSec).toBe(201);
    expect(p.variants.map((v) => [v.label, v.height])).toEqual([['360p', 360], ['480p', 480], ['720p', 720], ['1080p', 1080], ['4k', 2160]]);
    expect(p.variants.at(-1).direct).toBe(GET(3122623, '2160p'));
    expect(p.variants.some((v) => v.direct.includes('TOKEN') === false)).toBe(false);
  });

  it('copes with a page without alternative qualities or labels (height from the file name)', () => {
    const p = parseRule34videoPage(page({ alts: false }));
    expect(p.variants).toEqual([{ label: '', height: 360, direct: GET(3122623, '360') }]);
  });

  it('leaves out scrambled links and copes with a missing length', () => {
    const html = page({ alts: false, duration: '' }).replace(`video_url: '${GET(3122623, '360')}'`, "video_url: 'function/0/https://rule34video.com/get_file/1/x_360.mp4/'");
    const p = parseRule34videoPage(html);
    expect(p.variants).toEqual([]);
    expect(p.durationSec).toBeNull();
  });

  it('reads ISO 8601 lengths and quality labels', () => {
    expect(isoDurationSec('PT0H3M21S')).toBe(201);
    expect(isoDurationSec('PT45S')).toBe(45);
    expect(isoDurationSec('PT1H0M0S')).toBe(3600);
    expect(isoDurationSec('3:21')).toBeNull();
    expect(labelHeight('4k')).toBe(2160);
    expect(labelHeight('8K')).toBe(4320);
    expect(labelHeight('1080p')).toBe(1080);
    expect(labelHeight('HD')).toBe(0);
  });
});

describe('rule34videoFiles', () => {
  it('makes one file per quality named <title> <height>p.mp4, quality words out of the title', () => {
    const url = 'https://rule34video.com/video/3122623/yuffie/';
    const files = rule34videoFiles(url, parseRule34videoPage(page()));
    expect(files.map((f) => f.name)).toEqual([360, 480, 720, 1080, 2160].map((h) => `Yuffie's Interrogation Techniques - Nagoonimation ${h}p.mp4`));
    expect(files[0]).toEqual({ name: "Yuffie's Interrogation Techniques - Nagoonimation 360p.mp4", size: 0, host: 'rule34video', url, height: 360, duration: 201 });
  });

  it('keeps names file-safe and falls back to the video id', () => {
    expect(streamTitle('Tifa / Aerith: Night 1080p60 (4K)')).toBe('Tifa Aerith Night');
    expect(rule34videoFiles('https://rule34video.com/video/42/x/', { title: '', durationSec: null, variants: [{ height: 720, direct: 'd' }] })[0].name).toBe('rule34video 42 720p.mp4');
  });

  it('feeds planScenes so the quality caps pick the variant: flat takes 4K', () => {
    const videos = rule34videoFiles('https://rule34video.com/video/1/x/', parseRule34videoPage(page({ title: 'Tifa Riding' })));
    const plan = planScenes({ scripts: [{ name: 'Tifa Riding.funscript', host: 'forum', url: 'u' }], videos });
    expect(plan.scenes[0].video.name).toBe('Tifa Riding 2160p.mp4');
  });

  it('takes 1080p over 360p / 480p when a page has no 4K', () => {
    const videos = rule34videoFiles('https://rule34video.com/video/1/x/', {
      title: 'YORHA 2B EXPERIENCE - FAPHERO EDITION', durationSec: 319, variants: [360, 480, 720, 1080].map((height) => ({ height })),
    });
    const plan = planScenes({ scripts: [{ name: 'Yorha 2B Experience.funscript', host: 'forum', url: 'u' }], videos });
    expect(plan.scenes[0].video.height).toBe(1080);
  });
});

describe('rule34videoPosterUrl', () => {
  it("builds the page's cover link: <id rounded down to 1000>/<id>/preview.jpg", () => {
    expect(rule34videoPosterUrl('https://rule34video.com/video/3065491/scene/'))
      .toBe('https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg');
    expect(rule34videoPosterUrl('https://rule34video.com/video/999/x/')).toBe('https://rule34video.com/contents/videos_screenshots/0/999/preview.jpg');
    expect(rule34videoPosterUrl('https://rule34video.com/video/3000/x/')).toBe('https://rule34video.com/contents/videos_screenshots/3000/3000/preview.jpg');
  });

  it('is null for anything but a video page', () => {
    for (const u of ['https://rule34video.com/members/1/', 'https://rule34video.com/video/0/x/', 'https://example.com/video/5/x/', '', null, undefined]) {
      expect(rule34videoPosterUrl(u)).toBeNull();
    }
  });
});

describe('sceneSidecar', () => {
  const post = { id: 291491, url: 'https://discuss.eroscripts.com/t/x/291491', title: 'Yorha 2B Experience', author: 'Agapilord', tags: ['pmv'], scenes: [{}] };

  it("writes a rule34video scene's cover as posterUrl", () => {
    const [video] = rule34videoFiles('https://rule34video.com/video/3065491/yorha/', {
      title: 'YORHA 2B EXPERIENCE', durationSec: 319, variants: [{ height: 1080 }],
    });
    expect(sceneSidecar(post, video, 'Agapilord - Yorha 2B Experience.mp4', 'T')).toEqual({
      topicId: 291491, url: post.url, title: post.title, author: 'Agapilord', tags: ['pmv'],
      video: 'YORHA 2B EXPERIENCE 1080p.mp4', file: 'Agapilord - Yorha 2B Experience.mp4', scenes: 1,
      posterUrl: 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg',
      at: 'T',
    });
  });

  it('writes no posterUrl for other hosts, and keeps a VRBangers scene its site name', () => {
    const forum = sceneSidecar(post, { name: 'a.mp4', host: 'forum', url: 'https://discuss.eroscripts.com/uploads/a.mp4' }, 'a.mp4', 'T');
    expect(forum).not.toHaveProperty('posterUrl');
    const vrb = sceneSidecar(post, {
      name: 'b.mp4', host: 'vrbangers', url: 'https://vrbangers.com/video/b/', slug: 'b', title: 'B', performers: ['P'], projection: '180x180_3dh',
    }, 'b.mp4', 'T');
    expect(vrb.vrbangers).toEqual({ slug: 'b', url: 'https://vrbangers.com/video/b/', title: 'B', performers: ['P'], projection: '180x180_3dh' });
    expect(vrb).not.toHaveProperty('posterUrl');
  });
});

describe('durationMatches', () => {
  it('takes a script that ends ≤ 2 s after the video and covers ≥ 80 % of it', () => {
    expect(durationMatches(200, 202_000)).toBe(true);
    expect(durationMatches(200, 202_001)).toBe(false);
    expect(durationMatches(200, 160_000)).toBe(true);
    expect(durationMatches(200, 159_999)).toBe(false);
    expect(durationMatches(0, 1000)).toBe(false);
    expect(durationMatches(200, 0)).toBe(false);
  });
});

describe('planWithStreams', () => {
  const scripts = [
    { name: 'Tifa Riding.funscript', host: 'forum', url: 'main' },
    { name: 'Tifa Riding.twist.funscript', host: 'forum', url: 'twist' },
  ];
  const r34 = (id, title, duration) => rule34videoFiles(`https://rule34video.com/video/${id}/x/`, {
    title, durationSec: duration, variants: [{ height: 1080 }, { height: 2160 }],
  });

  it('drops a teaser page and pairs the script with the full video', async () => {
    const asked = [];
    // Same title, so planScenes takes the first page (the teaser) until its length rules it out.
    const plan = await planWithStreams({
      scripts, videos: [...r34(1, 'Tifa Riding', 25), ...r34(2, 'Tifa Riding', 200)],
      scriptMs: async (f) => { asked.push(f.url); return 198_500; },
    });
    expect(plan.scenes).toHaveLength(1);
    expect(plan.scenes[0].video).toMatchObject({ url: 'https://rule34video.com/video/2/x/', height: 2160 });
    expect(plan.scenes[0].axes).toEqual(['twist']);
    expect(plan.dropped).toEqual([{ url: 'https://rule34video.com/video/1/x/', durationSec: 25, scriptMs: 198_500 }]);
    expect(new Set(asked)).toEqual(new Set(['main']));
  });

  it('pairs a page whose title names no script set by length, the set with most axes first', async () => {
    const sets = [
      { name: '(Nagoonimation) FF - Yuffie Kisaragi.funscript', host: 'forum', url: 'soft' },
      { name: '(Nagoonimation) FF - Yuffie Kisaragi (Hardcore).funscript', host: 'forum', url: 'hard' },
      { name: '(Nagoonimation) FF - Yuffie Kisaragi (Hardcore).twist.funscript', host: 'forum', url: 'hard-twist' },
    ];
    const lengths = { soft: 199_000, hard: 200_500 };
    const page = r34(3122623, "Yuffie's Interrogation Techniques - No Male Audio", 201);
    const plan = await planWithStreams({ scripts: sets, videos: page, scriptMs: async (f) => lengths[f.url] ?? null });
    expect(plan.scenes).toHaveLength(1);
    expect(plan.scenes[0]).toMatchObject({ base: '(Nagoonimation) FF - Yuffie Kisaragi (Hardcore)', axes: ['twist'] });
    expect(plan.scenes[0].video.height).toBe(2160);

    const short = await planWithStreams({ scripts: sets, videos: r34(1, 'Something Else', 30), scriptMs: async (f) => lengths[f.url] ?? null });
    expect(short.scenes).toEqual([]);
  });

  it('leaves no scene when the only page is a teaser (the post is no-video)', async () => {
    const plan = await planWithStreams({ scripts, videos: r34(1, 'Tifa Riding', 33), scriptMs: async () => 200_000 });
    expect(plan.scenes).toEqual([]);
    expect(plan.dropped).toHaveLength(1);
  });

  it('keeps the page when the script or the video length is unknown, and leaves other hosts alone', async () => {
    expect((await planWithStreams({ scripts, videos: r34(1, 'Tifa Riding', 33), scriptMs: async () => null })).scenes).toHaveLength(1);
    expect((await planWithStreams({ scripts, videos: r34(1, 'Tifa Riding', null), scriptMs: async () => 200_000 })).scenes).toHaveLength(1);
    const forum = [{ name: 'Tifa Riding.mp4', size: 1e8, host: 'forum', url: 'v' }];
    let asked = 0;
    const plan = await planWithStreams({ scripts, videos: forum, scriptMs: async () => { asked++; return 1; } });
    expect(plan.scenes[0].video.host).toBe('forum');
    expect(asked).toBe(0);
  });
});

/** A fake clock: sleep moves it on and records the waits. */
function fakeClock() {
  const c = { t: 0, sleeps: [] };
  c.now = () => c.t;
  c.sleep = async (ms) => { c.sleeps.push(ms); c.t += ms; };
  return c;
}
const response = (status, { body = '', headers = {} } = {}) => new Response(status === 204 ? null : body, { status, headers });

describe('pacedFetch', () => {
  it('spaces requests at least 400 ms apart', async () => {
    const clock = fakeClock();
    const at = [];
    const get = pacedFetch({ fetch: async () => { at.push(clock.t); return response(200); }, now: clock.now, sleep: clock.sleep });
    for (let i = 0; i < 4; i++) await get('u');
    expect(at).toEqual([0, 400, 800, 1200]);
    expect(RULE34VIDEO_INTERVAL_MS).toBeGreaterThanOrEqual(400);
  });

  it('waits 60 s after a 429 and tries again, then gives up', async () => {
    const clock = fakeClock();
    const statuses = [429, 200];
    const get = pacedFetch({ fetch: async () => response(statuses.shift()), now: clock.now, sleep: clock.sleep });
    expect((await get('u')).status).toBe(200);
    expect(clock.sleeps).toEqual([RATE_LIMIT_BACKOFF_MS]);
    expect(RATE_LIMIT_BACKOFF_MS).toBe(60_000);

    const always = pacedFetch({ fetch: async () => response(429), now: clock.now, sleep: clock.sleep, retries: 2 });
    await expect(always('u')).rejects.toThrow(/rate limited/);
  });
});

describe('createRule34video', () => {
  function fakeSite() {
    const calls = [];
    let token = 0;
    const fetch = async (url, init = {}) => {
      calls.push({ url, headers: init.headers || {} });
      if (url.includes('/video/')) {
        token++;
        return response(200, { body: page({ title: 'Tifa Riding' }).replaceAll('TOKEN', `T${token}-`) });
      }
      if (init.headers?.Range === 'bytes=0-0') return response(206, { body: 'x', headers: { 'content-range': 'bytes 0-0/588570864' } });
      return response(200, { body: 'video bytes' });
    };
    const clock = fakeClock();
    return { calls, site: createRule34video({ fetch, now: clock.now, sleep: clock.sleep }) };
  }

  it('lists a page, probes only the chosen quality, and reads the page again for a fresh link', async () => {
    const { calls, site } = fakeSite();
    const url = 'https://rule34video.com/video/3122623/tifa/';
    const files = await site.files(url);
    expect(files).toHaveLength(5);
    const pick = files.at(-1);
    expect(await site.probeSize(pick)).toBe(588570864);
    expect(calls).toHaveLength(2);
    expect(calls[1].url).toContain('_2160p.mp4');
    expect(calls[1].headers).toMatchObject({ Referer: 'https://rule34video.com/', Range: 'bytes=0-0' });
    expect(calls[1].headers['User-Agent']).toMatch(/Mozilla/);

    const fresh = await site.fresh(pick);
    expect(fresh).toEqual({ direct: expect.stringContaining('T2-2160p'), durationSec: 201 });
    const resp = await site.stream(fresh.direct);
    expect(await resp.text()).toBe('video bytes');
    expect(calls.at(-1).headers).toMatchObject({ Referer: 'https://rule34video.com/' });
  });

  it('fails when the page is gone or lost the quality', async () => {
    const { site } = fakeSite();
    await expect(site.fresh({ url: 'https://rule34video.com/video/1/x/', height: 4320 })).rejects.toThrow(/no 4320p/);
    const gone = createRule34video({ fetch: async () => response(404), now: () => 0, sleep: async () => {} });
    await expect(gone.files('https://rule34video.com/video/1/x/')).rejects.toThrow(/HTTP 404/);
  });
});
