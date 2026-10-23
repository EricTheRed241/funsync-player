/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Is an EroScripts post image a picture of the video? Candidate rules
// (what can't be one) and the perceptual-hash comparison against frames
// sampled from the video. See electron/preview-match.js.
import { describe, it, expect } from 'vitest';

const pm = await import('../../electron/preview-match.js');

const CDN = 'https://eroscripts-discourse.eroscripts.com';
const FORUM = 'https://discuss.eroscripts.com';
const SHA = (c) => c.repeat(40);
const up = (c, ext = 'webp') => `${CDN}/original/4X/a/b/c/${SHA(c)}.${ext}`;
const img = (src, w = 690, h = 388, extra = '') => `<img src="${src}" width="${w}" height="${h}"${extra}>`;
const topic = (cooked, extra = {}) => ({ post_stream: { posts: [{ cooked }] }, ...extra });
const urls = (r) => r.candidates.map((c) => c.url);
const reasonOf = (r, url) => r.excluded.find((e) => e.url === url)?.reason;

describe('collectPreviewCandidates', () => {
  it('keeps every poster-shaped upload of the first post, in order', () => {
    const r = pm.collectPreviewCandidates(topic(img(up('a')) + img(up('b'), 1920, 1080)));
    expect(urls(r)).toEqual([up('a'), up('b')]);
    expect(r.candidates[1]).toMatchObject({ origin: 'post', width: 1920, height: 1080 });
  });

  it('excludes avatars and profile pictures, by URL and by class', () => {
    const avatar = `${FORUM}/user_avatar/discuss.eroscripts.com/someone/240/1234_2.png`;
    const letter = `${FORUM}/letter_avatar_proxy/v4/letter/s/abc/240.png`;
    const named = `${CDN}/original/4X/a/b/c/my_avatar_big.png`;
    const classed = up('d');
    const r = pm.collectPreviewCandidates(topic(
      img(avatar, 480, 300) + img(letter, 480, 300) + img(named, 480, 300) + img(classed, 480, 300, ' class="avatar"'),
    ));
    expect(r.candidates).toEqual([]);
    expect([avatar, letter, named, classed].map((u) => reasonOf(r, u))).toEqual(['avatar', 'avatar', 'avatar', 'avatar']);
  });

  it('excludes emoji, by path and by class', () => {
    const r = pm.collectPreviewCandidates(topic(
      `<h3>${img(`${FORUM}/images/emoji/twitter/framed_picture.png?v=15`, 20, 20, ' class="emoji"')} Preview</h3>`
      + img(up('e'), 690, 388, ' class="emoji only-emoji"') + img(up('f')),
    ));
    expect(urls(r)).toEqual([up('f')]);
    expect(reasonOf(r, up('e'))).toBe('emoji');
  });

  it('excludes images inside quotes, mentions and link previews', () => {
    const quote = `<aside class="quote no-group" data-username="x"><div class="title">${img(up('1'), 480, 270, ' class="avatar"')} x:</div>`
      + `<blockquote><p>${img(up('2'))}</p></blockquote></aside>`;
    const onebox = `<aside class="onebox allowlistedgeneric"><article>${img(up('3'))}</article></aside>`;
    const mention = `<a class="mention" href="/u/x">${img(up('4'))}</a>`;
    const r = pm.collectPreviewCandidates(topic(quote + onebox + mention + img(up('5'))));
    expect(urls(r)).toEqual([up('5')]);
    expect(reasonOf(r, up('1'))).toBe('avatar');
    expect(reasonOf(r, up('2'))).toBe('quoted post');
    expect(reasonOf(r, up('3'))).toBe('link preview');
    expect(reasonOf(r, up('4'))).toBe('avatar');
  });

  it('excludes banners: images linking to Patreon, Ko-fi, stores or any other site', () => {
    const r = pm.collectPreviewCandidates(topic(
      `<a href="https://www.patreon.com/creator">${img(up('p'), 1200, 400)}</a>`
      + `<a href="https://ko-fi.com/creator"><img src="${up('k')}" width="900" height="400"></a>`
      + `<a href="https://shop.example.com/item">${img(up('s'))}</a>`
      + `<a href="${CDN}/original/4X/a/b/c/${SHA('l')}.webp" class="lightbox">${img(up('l'))}</a>`,
    ));
    expect(reasonOf(r, up('p'))).toBe('banner (patreon.com)');
    expect(reasonOf(r, up('k'))).toBe('banner (ko-fi.com)');
    expect(reasonOf(r, up('s'))).toBe('banner (shop.example.com)');
    // Discourse's lightbox links an image to its own original: not a banner.
    expect(urls(r)).toEqual([up('l')]);
  });

  it('excludes site images and other hosts', () => {
    const r = pm.collectPreviewCandidates(topic(
      img(`${FORUM}/images/badges/x.png`) + img(`${FORUM}/plugins/x/banner.png`) + img('https://i.imgur.com/abc.jpg') + img(up('g')),
    ));
    expect(urls(r)).toEqual([up('g')]);
    expect(r.excluded.map((e) => e.reason)).toEqual(['site image', 'site image', 'not a forum upload']);
  });

  it('excludes images under 320 px on the longest side', () => {
    const r = pm.collectPreviewCandidates(topic(img(up('a'), 300, 169) + img(up('b'), 320, 180)));
    expect(urls(r)).toEqual([up('b')]);
    expect(reasonOf(r, up('a'))).toBe('too small');
  });

  it('excludes aspect ratios outside 1.2–3.2 (portraits, squares, banners, heatmaps)', () => {
    const r = pm.collectPreviewCandidates(topic(
      img(up('1'), 400, 600) + img(up('2'), 500, 500) + img(up('3'), 1200, 300) + img(up('4'), 1200, 1000) + img(up('5'), 1280, 400),
    ));
    expect(urls(r)).toEqual([up('4'), up('5')]); // 1.2 and 3.2 are inside
    expect([up('1'), up('2'), up('3')].map((u) => reasonOf(r, u))).toEqual(['wrong aspect', 'wrong aspect', 'wrong aspect']);
  });

  it('adds the topic thumbnail and known URLs, once per upload', () => {
    const optimized = `${CDN}/optimized/4X/a/b/c/${SHA('a')}_2_690x388.webp`;
    const data = topic(img(up('a')) + img(up('b')), {
      thumbnails: [
        { width: 1920, height: 1080, url: up('a') },
        { width: 690, height: 388, url: optimized },
      ],
    });
    const r = pm.collectPreviewCandidates(data, { extraUrls: [up('c'), up('b')] });
    expect(urls(r)).toEqual([up('c'), up('b'), optimized]);
    expect(r.candidates.map((c) => c.origin)).toEqual(['known', 'known', 'thumbnail']);
  });

  it('caps the number of candidates', () => {
    const many = Array.from({ length: 12 }, (_, i) => img(up(i.toString(16)))).join('');
    expect(pm.collectPreviewCandidates(topic(many)).candidates).toHaveLength(pm.MAX_CANDIDATES);
  });
});

// ---- Hashing ----

/** A deterministic, textured grayscale picture (w×h), varied by `seed`. */
function picture(w, h, seed) {
  const g = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const u = x / w;
      const v = y / h;
      const val = 128
        + 50 * Math.sin(u * (3 + seed) * 2.1 + v * (2 + seed * 0.7) * 1.3 + seed)
        + 40 * Math.cos(v * (4 + (seed % 3)) * 2.7 - u * 1.9 * seed)
        + 25 * Math.sin((u * u + v) * 9 * (1 + seed * 0.13));
      g[y * w + x] = Math.max(0, Math.min(255, Math.round(val)));
    }
  }
  return g;
}

/** Nearest-neighbour resize, for a "re-encoded at another size" copy. */
function resize(gray, w, h, tw, th) {
  const out = new Uint8Array(tw * th);
  for (let y = 0; y < th; y++) for (let x = 0; x < tw; x++) out[y * tw + x] = gray[Math.floor(y * h / th) * w + Math.floor(x * w / tw)];
  return out;
}

function sideBySide(left, right, w, h) {
  const out = new Uint8Array(w * 2 * h);
  for (let y = 0; y < h; y++) {
    out.set(left.subarray(y * w, (y + 1) * w), y * w * 2);
    out.set(right.subarray(y * w, (y + 1) * w), y * w * 2 + w);
  }
  return out;
}

describe('pHash and distance', () => {
  it('is 64 bits, 0 for the same picture and small for a resized copy', () => {
    const a = picture(128, 72, 1);
    const ha = pm.pHash(a, 128, 72).hash;
    expect(ha).toMatch(/^[0-9a-f]{16}$/);
    expect(pm.hamming(ha, ha)).toBe(0);
    const big = resize(a, 128, 72, 640, 360);
    expect(pm.hamming(ha, pm.pHash(big, 640, 360).hash)).toBeLessThanOrEqual(4);
  });

  it('tells different pictures apart', () => {
    const ha = pm.pHash(picture(128, 72, 1), 128, 72).hash;
    for (const seed of [2, 3, 5, 8]) {
      expect(pm.hamming(ha, pm.pHash(picture(128, 72, seed), 128, 72).hash)).toBeGreaterThan(pm.MATCH_THRESHOLD);
    }
  });

  it('counts differing bits', () => {
    expect(pm.hamming('0000000000000000', 'ffffffffffffffff')).toBe(64);
    expect(pm.hamming('00000000000000f0', '0000000000000001')).toBe(5);
  });

  it('leaves flat images and frames out (black, fades, solid fills)', () => {
    const black = new Uint8Array(128 * 72).fill(4);
    expect(pm.imageHashes([{ gray: black, width: 128, height: 72 }])).toEqual([]);
    expect(pm.frameHashes(black, 128, 72)).toEqual([]);
  });
});

describe('threshold', () => {
  it('accepts a distance up to the threshold, not beyond', () => {
    expect(pm.MATCH_THRESHOLD).toBe(12);
    expect(pm.isAccepted({ distance: 12 })).toBe(true);
    expect(pm.isAccepted({ distance: 13 })).toBe(false);
    expect(pm.isAccepted(null)).toBe(false);
  });

  it('picks the accepted candidate with the best distance', () => {
    const scored = [
      { url: 'a', match: { distance: 9 } },
      { url: 'b', match: { distance: 3 } },
      { url: 'c', match: { distance: 1 }, note: 'over the threshold below' },
      { url: 'd', match: null },
    ];
    expect(pm.pickBest(scored).url).toBe('c');
    expect(pm.pickBest(scored.slice(0, 2)).url).toBe('b');
    expect(pm.pickBest([{ url: 'x', match: { distance: 20 } }, { url: 'y', match: null }])).toBeNull();
  });
});

describe('matching against video frames', () => {
  const W = 128;
  const H = 72;
  const sample = (frames, w = W, h = H) => ({ width: w, height: h, frames: frames.map((gray, i) => ({ pct: 0.1 + i * 0.1, gray })) });

  it('finds the frame a poster was taken from', () => {
    const frames = [picture(W, H, 3), picture(W, H, 4), picture(W, H, 1), picture(W, H, 6)];
    const poster = resize(picture(W, H, 1), W, H, 640, 360);
    const match = pm.bestMatch(pm.imageHashes([{ gray: poster, width: 640, height: 360 }]), pm.videoHashes(sample(frames)));
    expect(match).toMatchObject({ distance: expect.any(Number), pct: 0.30000000000000004, view: 'full' });
    expect(pm.isAccepted(match)).toBe(true);
  });

  it('rejects an image that is not in the video', () => {
    const frames = [picture(W, H, 3), picture(W, H, 4), picture(W, H, 6)];
    const banner = picture(640, 360, 9);
    const match = pm.bestMatch(pm.imageHashes([{ gray: banner, width: 640, height: 360 }]), pm.videoHashes(sample(frames)));
    expect(pm.isAccepted(match)).toBe(false);
  });

  it('uses the best frame of an animated preview', () => {
    const frames = [picture(W, H, 5)];
    const anim = [picture(W, H, 9), picture(W, H, 5), picture(W, H, 7)].map((gray) => ({ gray, width: W, height: H }));
    const match = pm.bestMatch(pm.imageHashes(anim), pm.videoHashes(sample(frames)));
    expect(match).toMatchObject({ distance: 0, frame: 1 });
  });

  it('compares side-by-side VR frames by their left eye as well as whole', () => {
    // A 2:1 SBS frame: the left eye is the poster, the right eye differs
    // (parallax), so the whole frame does not match it.
    const eye = picture(W, W, 2);
    const other = picture(W, W, 7);
    const vr = sideBySide(eye, other, W, W);
    const hashes = pm.frameHashes(vr, W * 2, W);
    expect(hashes.map((h) => h.view)).toEqual(['full', 'left']);

    const poster = [{ gray: resize(eye, W, W, 400, 400), width: 400, height: 400 }];
    const match = pm.bestMatch(pm.imageHashes(poster), pm.videoHashes(sample([vr], W * 2, W)));
    expect(match.view).toBe('left');
    expect(pm.isAccepted(match)).toBe(true);
    const fullOnly = pm.bestMatch(pm.imageHashes(poster), hashes.filter((h) => h.view === 'full').map((h) => ({ pct: 0, ...h })));
    expect(fullOnly.distance).toBeGreaterThan(match.distance);

    // ...and a poster of the whole SBS frame still matches the full view.
    const full = [{ gray: vr, width: W * 2, height: W }];
    expect(pm.bestMatch(pm.imageHashes(full), pm.videoHashes(sample([vr], W * 2, W)))).toMatchObject({ distance: 0, view: 'full' });
  });

  it('does not split flat-video frames', () => {
    expect(pm.frameHashes(picture(W, H, 1), W, H).map((h) => h.view)).toEqual(['full']);
  });
});
