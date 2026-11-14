/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Main-process half of "EroScripts post preview as the video thumbnail":
// the download (EroScriptsAPI.downloadPreviewImage) and the shared import
// into custom-thumbs (custom-thumbs.js importThumbnail), which user
// uploads go through too.
import { describe, it, expect, vi, beforeAll, afterAll, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const log = (await import('../../electron/logger.js')).default;
const { EroScriptsAPI, postImages, topicPreviewUrl } = await import('../../electron/eroscripts-api.js');
const { importThumbnail, thumbStem } = await import('../../electron/custom-thumbs.js');

const CDN = 'https://eroscripts-discourse.eroscripts.com/original/4X/a/b/c/abc.webp';
const FORUM = 'https://discuss.eroscripts.com/uploads/short-url/abc.webp';

let prev;
let dir;
const realFetch = globalThis.fetch;

beforeAll(() => {
  prev = [log.transports.file.level, log.transports.console.level];
  log.transports.file.level = false;
  log.transports.console.level = false;
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'es-preview-'));
});
afterAll(() => {
  [log.transports.file.level, log.transports.console.level] = prev;
  fs.rmSync(dir, { recursive: true, force: true });
});
afterEach(() => { globalThis.fetch = realFetch; });

function response({ status = 200, type = 'image/webp', body = new Uint8Array([1, 2, 3]), length, location } = {}) {
  const headers = new Headers();
  if (type) headers.set('content-type', type);
  if (length != null) headers.set('content-length', String(length));
  if (location) headers.set('location', location);
  return new Response(status >= 300 && status < 400 ? null : body, { status, headers });
}

function loggedIn() {
  const api = new EroScriptsAPI();
  api.restoreSession('SECRET', 'me');
  return api;
}

describe('EroScriptsAPI.downloadPreviewImage', () => {
  it('fetches CDN images without the forum cookie', async () => {
    globalThis.fetch = vi.fn(async () => response());
    const res = await loggedIn().downloadPreviewImage(CDN);
    expect(res.success).toBe(true);
    expect([...res.bytes]).toEqual([1, 2, 3]);
    expect(res.contentType).toBe('image/webp');
    const [url, opts] = globalThis.fetch.mock.calls[0];
    expect(url).toBe(CDN);
    expect(opts.headers.Cookie).toBeUndefined();
    expect(opts.redirect).toBe('manual');
  });

  it('sends the cookie to the forum only, and drops it on a redirect to the CDN', async () => {
    globalThis.fetch = vi.fn()
      .mockResolvedValueOnce(response({ status: 302, location: CDN }))
      .mockResolvedValueOnce(response());
    const res = await loggedIn().downloadPreviewImage(FORUM);
    expect(res.success).toBe(true);
    const calls = globalThis.fetch.mock.calls;
    expect(calls[0][1].headers.Cookie).toBe('_t=SECRET');
    expect(calls[1][0]).toBe(CDN);
    expect(calls[1][1].headers.Cookie).toBeUndefined();
  });

  it.each([
    ['plain http', 'http://eroscripts-discourse.eroscripts.com/original/x.webp'],
    ['another host', 'https://evil.example.com/x.webp'],
    ['a look-alike host', 'https://eroscripts-discourse.eroscripts.com.evil.example/x.webp'],
    ['a non-URL', 'not a url'],
  ])('refuses %s without fetching', async (_label, url) => {
    globalThis.fetch = vi.fn();
    expect(await loggedIn().downloadPreviewImage(url)).toEqual({ success: false, error: 'Unsupported image link' });
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('refuses a redirect off the allow-list', async () => {
    globalThis.fetch = vi.fn().mockResolvedValueOnce(response({ status: 302, location: 'https://evil.example.com/x.png' }));
    expect((await loggedIn().downloadPreviewImage(CDN)).success).toBe(false);
    expect(globalThis.fetch).toHaveBeenCalledTimes(1);
  });

  it('refuses anything that is not an image', async () => {
    globalThis.fetch = vi.fn(async () => response({ type: 'text/html; charset=utf-8', body: '<html>' }));
    expect(await loggedIn().downloadPreviewImage(CDN)).toEqual({ success: false, error: 'Not an image' });
    globalThis.fetch = vi.fn(async () => response({ type: null }));
    expect(await loggedIn().downloadPreviewImage(CDN)).toEqual({ success: false, error: 'Not an image' });
  });

  it('refuses images over 15 MB, by header or by actual size', async () => {
    globalThis.fetch = vi.fn(async () => response({ length: 15 * 1024 * 1024 + 1 }));
    expect(await loggedIn().downloadPreviewImage(CDN)).toEqual({ success: false, error: 'Image too large' });

    // No Content-Length: the stream is cut off once it passes the cap.
    const big = new Uint8Array(15 * 1024 * 1024 + 10);
    globalThis.fetch = vi.fn(async () => response({ body: big }));
    expect(await loggedIn().downloadPreviewImage(CDN)).toEqual({ success: false, error: 'Image too large' });
  });

  it('reports HTTP errors', async () => {
    globalThis.fetch = vi.fn(async () => response({ status: 404 }));
    expect(await loggedIn().downloadPreviewImage(CDN)).toEqual({ success: false, error: 'Image download failed (404)' });
  });
});

describe('topic preview choice (getTopicImage)', () => {
  const topic = (extra) => new Response(JSON.stringify({ post_stream: { posts: [{ cooked: '' }] }, ...extra }));

  it("prefers the topic's poster-size thumbnail over the small list one", async () => {
    globalThis.fetch = vi.fn(async () => topic({
      image_url: 'https://cdn/original/x.webp',
      thumbnails: [
        { width: 1024, url: 'https://cdn/optimized/x_1024.webp' },
        { width: 690, url: 'https://cdn/optimized/x_690.webp' },
        { width: 400, url: 'https://cdn/optimized/x_400.webp' },
      ],
    }));
    expect(await new EroScriptsAPI().getTopicImage(1)).toBe('https://cdn/optimized/x_690.webp');
  });

  // Topic 311478's shape: a 160×90 "Thumbnail" for the topic list, the
  // real 690-wide previews below it, an emoji in the heading first.
  const COOKED = '<h3><img src="https://discuss.eroscripts.com/images/emoji/twitter/framed_picture.png?v=15" class="emoji" width="20" height="20"> Preview</h3>'
    + '<p><img src="https://cdn/original/thumb.webp" alt="thumbnail" width="160" height="90"></p>'
    + '<p><img src="/uploads/default/original/1X/big.webp?a=1&amp;b=2" alt="x" width="690" height="388"></p>';

  it('skips a tiny topic thumbnail and emoji for the first big post image', () => {
    const data = {
      image_url: 'https://cdn/original/thumb.webp',
      thumbnails: [{ width: 160, url: 'https://cdn/original/thumb.webp' }, { width: 100, url: 'https://cdn/optimized/thumb_100.webp' }],
      post_stream: { posts: [{ cooked: COOKED }] },
    };
    expect(topicPreviewUrl(data)).toBe('https://discuss.eroscripts.com/uploads/default/original/1X/big.webp?a=1&b=2');
  });

  it('still uses a small image when that is all the topic has', () => {
    const small = '<p><img src="https://cdn/original/thumb.webp" width="160" height="90"></p>';
    expect(topicPreviewUrl({ image_url: 'https://cdn/original/rep.webp', post_stream: { posts: [{ cooked: small }] } }))
      .toBe('https://cdn/original/rep.webp');
    expect(topicPreviewUrl({ post_stream: { posts: [{ cooked: small }] } })).toBe('https://cdn/original/thumb.webp');
    expect(topicPreviewUrl({ post_stream: { posts: [{ cooked: '<p>no images</p>' }] } })).toBeNull();
  });
});

describe('postImages (scout report gallery)', () => {
  const SHA = 'a'.repeat(40);
  const UP = 'https://discuss.eroscripts.com/uploads/default/original/1X';
  const img = (name, attrs = '') => `<p><img src="/uploads/default/original/1X/${name}" ${attrs}></p>`;
  const linked = (href, name, attrs = '') => `<p><a href="${href}"><img src="/uploads/default/original/1X/${name}" ${attrs}></a></p>`;
  // Markup as real posts have it: an emoji heading, a lightbox around the optimized copy, previews,
  // and the clutter around them (seen in the deck, 2026-11-13).
  const COOKED = '<h3><img src="https://discuss.eroscripts.com/images/emoji/twitter/framed_picture.png?v=15" class="emoji" alt=":framed_picture:" width="20" height="20"> Preview</h3>'
    + `<div class="lightbox-wrapper"><a class="lightbox" href="https://cdn/original/4X/a/b/c/${SHA}.jpeg" data-download-href="/uploads/short-url/x.jpeg?dl=1">`
    + `<img src="https://cdn/optimized/4X/a/b/c/${SHA}_2_690x388.jpeg" alt="p (1)" width="690" height="388"><div class="meta"></div></a></div>`
    + img('preview.gif', 'class="animated" alt="gif1" width="200" height="140"')
    + img('tall.webp', 'class="animated" alt="gif2" width="221" height="499"')
    + img('no-size.webp?a=1&amp;b=2', 'alt="clip"')
    + linked('https://www.patreon.com/someone', 'patreon.png', 'alt="" width="690" height="172"')
    + linked('https://shop.example.com/?ref=x', 'promo.jpeg', 'alt="ad" width="517" height="300"')
    + linked('https://mega.nz/folder/abc#key', 'mega.png', 'alt="MEGA" width="400" height="200"')
    + linked('https://discuss.eroscripts.com/t/portfolio/1', 'about.png', 'alt="About ME" width="310" height="207"')
    + img('heat.png', 'alt="MS" width="690" height="17"')
    + img('sticker.gif', 'class="animated" alt="AHEI_1" width="137" height="137"')
    + img('small.webp', 'class="animated" alt="Peach suck0" width="150" height="113"')
    + img('pixel.gif', 'class="animated" alt="g" width="1" height="1"')
    + img('named.png', 'alt="discord logo" width="400" height="400"')
    + '<p><img src="https://cdn/site.png" class="site-icon" width="32" height="32"></p>'
    + img('clip.mp4');

  it('keeps only scene previews: no linked buttons, banners, heatmaps, stickers, icons or named logos', () => {
    const data = {
      image_url: `${UP}/preview.gif`,
      post_stream: { posts: [{ username: 'maker', cooked: COOKED }] },
    };
    expect(postImages(data)).toEqual([
      `${UP}/preview.gif`, // the topic image leads
      `https://cdn/original/4X/a/b/c/${SHA}.jpeg`, // lightbox original, not the optimized copy
      `${UP}/tall.webp`,
      `${UP}/no-size.webp?a=1&b=2`, // no size to judge by: kept
    ]);
  });

  it("adds replies' images, the author's first, and never pads to 5", () => {
    const data = {
      post_stream: { posts: [
        { username: 'maker', cooked: img('main.gif', 'width="500" height="420"') },
        { username: 'fan', cooked: img('fan-shot.jpg', 'width="690" height="388"') + linked('https://giphy.com/x', 'reaction.gif', 'width="300" height="300"') },
        { username: 'maker', cooked: img('update.gif', 'width="480" height="270"') + img('main.gif', 'width="500" height="420"') },
      ] },
    };
    expect(postImages(data)).toEqual([`${UP}/main.gif`, `${UP}/update.gif`, `${UP}/fan-shot.jpg`]);
    // A topic image that isn't a good preview doesn't get in.
    expect(postImages({ image_url: `${UP}/banner.png`, post_stream: { posts: [{ cooked: img('banner.png', 'width="690" height="172"') }] } })).toEqual([]);
    expect(postImages({ image_url: 'https://cdn/original/rep.webp' })).toEqual([]);
    expect(postImages(null)).toEqual([]);
  });

  it('caps at max and dedupes the same upload', () => {
    const many = Array.from({ length: 8 }, (_, i) => img(`p${i}.jpg`, 'width="690" height="388"')).join('');
    const data = { image_url: '/uploads/default/original/1X/p3.jpg', post_stream: { posts: [{ cooked: many + img('p0.jpg', 'width="690" height="388"') }] } };
    expect(postImages(data)).toEqual([3, 0, 1, 2, 4].map((i) => `${UP}/p${i}.jpg`));
    expect(postImages(data, 2)).toEqual([`${UP}/p3.jpg`, `${UP}/p0.jpg`]);
  });
});

describe('importThumbnail (shared by uploads and previews)', () => {
  function fakeNativeImage({ width = 1280, decodable = true } = {}) {
    const img = (w) => ({
      isEmpty: () => !decodable,
      getSize: () => ({ width: w, height: Math.round(w * 9 / 16) }),
      resize: vi.fn(({ width: nw }) => img(nw)),
      toJPEG: vi.fn(() => Buffer.from(`jpeg-${w}`)),
    });
    return { createFromBuffer: vi.fn(() => img(width)) };
  }

  it('downscales to 640 px and writes a JPEG named after the video and source', async () => {
    const stem = thumbStem('/v/a.mp4', 'eroscripts');
    expect(stem).toMatch(/^[0-9a-f]{32}-eroscripts$/);
    expect(thumbStem('/v/a.mp4')).toBe(stem.replace('-eroscripts', ''));
    fs.writeFileSync(path.join(dir, `${stem}.webp`), 'stale raw copy');

    const res = await importThumbnail(Buffer.from('x'), { dir, stem, nativeImage: fakeNativeImage() });
    expect(res.path).toBe(path.join(dir, `${stem}.jpg`));
    expect(fs.readFileSync(res.path, 'utf8')).toBe('jpeg-640');
    expect(res.dataUrl).toBe(`data:image/jpeg;base64,${Buffer.from('jpeg-640').toString('base64')}`);
    expect(fs.existsSync(path.join(dir, `${stem}.webp`))).toBe(false);
  });

  it('decodes what nativeImage cannot through the fallback (webp)', async () => {
    const ni = fakeNativeImage();
    const empty = { isEmpty: () => true };
    const decoded = ni.createFromBuffer();
    ni.createFromBuffer = vi.fn().mockReturnValueOnce(empty).mockReturnValueOnce(decoded);
    const decodeFallback = vi.fn(async () => Buffer.from('png'));
    const res = await importThumbnail(Buffer.from('webp'), { dir, stem: 'fallback', nativeImage: ni, decodeFallback });
    expect(decodeFallback).toHaveBeenCalledTimes(1);
    expect(res.path).toBe(path.join(dir, 'fallback.jpg'));
  });

  it('returns null and keeps the existing file when the image cannot be decoded', async () => {
    const existing = path.join(dir, 'keep.jpg');
    fs.writeFileSync(existing, 'previous');
    const res = await importThumbnail(Buffer.from('junk'), {
      dir, stem: 'keep', nativeImage: fakeNativeImage({ decodable: false }), decodeFallback: async () => null,
    });
    expect(res).toBeNull();
    expect(fs.readFileSync(existing, 'utf8')).toBe('previous');
  });
});
