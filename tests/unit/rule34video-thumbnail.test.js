/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Main-process half of rule34video covers (electron/rule34video-thumbnail.js):
// only the exact cover link is fetched (no cookies, every redirect hop
// checked, size capped), it is imported without a frame check, and a link
// that can't work again is told apart from a failure that may pass.
import { describe, it, expect, vi } from 'vitest';

const {
  isRule34videoPosterUrl, downloadRule34videoPoster, importRule34videoPoster,
} = await import('../../electron/rule34video-thumbnail.js');
const { MAX_BYTES } = await import('../../electron/thumbnail-download.js');
const { rule34videoPosterUrl } = await import('../../scripts/stream-hosts.mjs');

const COVER = 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg';
const VIDEO = '/lib/EroScripts/Agapilord/Scene/Scene.mp4';

function response({ status = 200, type = 'image/jpeg', body = new Uint8Array([1, 2, 3]), length, location } = {}) {
  const headers = new Headers();
  if (type) headers.set('content-type', type);
  if (length != null) headers.set('content-length', String(length));
  if (location) headers.set('location', location);
  return new Response(status >= 300 && status < 400 ? null : body, { status, headers });
}

describe('isRule34videoPosterUrl', () => {
  it.each([
    [COVER, true],
    ['https://rule34video.com/contents/videos_screenshots/0/999/preview.jpg', true],
    ['https://rule34video.com/contents/videos_screenshots/1000/1000/preview.jpg', true],
    ['http://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg', false],
    ['https://www.rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg', false],
    ['https://RULE34VIDEO.com/contents/videos_screenshots/3065000/3065491/preview.jpg', false],
    ['https://rule34video.com.evil.example/contents/videos_screenshots/3065000/3065491/preview.jpg', false],
    ['https://rule34video.com:443/contents/videos_screenshots/3065000/3065491/preview.jpg', false],
    ['https://a:b@rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg', false],
    [`${COVER}?w=1`, false],
    [`${COVER}#x`, false],
    ['https://rule34video.com/contents/videos_screenshots/3064000/3065491/preview.jpg', false],
    ['https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.png', false],
    ['https://rule34video.com/contents/videos_screenshots/3065000/3065491/320x180/1.jpg', false],
    ['https://rule34video.com/contents/videos_screenshots/03065000/3065491/preview.jpg', false],
    ['https://rule34video.com/contents/videos_screenshots/0/0/preview.jpg', false],
    ['https://rule34video.com/video/3065491/scene/', false],
    ['', false],
    [null, false],
    [42, false],
  ])('%s → %s', (url, ok) => {
    expect(isRule34videoPosterUrl(url)).toBe(ok);
  });

  it("accepts every cover the scout builds from a video page", () => {
    for (const page of ['https://rule34video.com/video/3065491/scene/', 'https://rule34video.com/video/42/x/', 'https://rule34video.com/video/1000/x/']) {
      expect(isRule34videoPosterUrl(rule34videoPosterUrl(page))).toBe(true);
    }
  });
});

describe('downloadRule34videoPoster', () => {
  it('fetches the cover without cookies, with the site as referrer', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(response());
    const res = await downloadRule34videoPoster(COVER, { fetchImpl });
    expect(res).toMatchObject({ success: true, contentType: 'image/jpeg' });
    expect([...res.bytes]).toEqual([1, 2, 3]);
    const [url, init] = fetchImpl.mock.calls[0];
    expect(url).toBe(COVER);
    expect(init).toMatchObject({ redirect: 'manual', credentials: 'omit' });
    expect(init.headers.Referer).toBe('https://rule34video.com/');
    expect(init.headers.Cookie).toBeUndefined();
  });

  it('refuses other links without fetching, and redirects anywhere else', async () => {
    const fetchImpl = vi.fn().mockResolvedValue(response({ status: 302, location: 'https://cdn.example/preview.jpg' }));
    expect(await downloadRule34videoPoster('https://evil.example/preview.jpg', { fetchImpl })).toEqual({ success: false, error: 'Unsupported image link' });
    expect(fetchImpl).not.toHaveBeenCalled();
    expect(await downloadRule34videoPoster(COVER, { fetchImpl })).toEqual({ success: false, error: 'Unsupported image link' });
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it('refuses non-images, oversized images and HTTP errors', async () => {
    const one = (resp) => downloadRule34videoPoster(COVER, { fetchImpl: vi.fn().mockResolvedValue(resp) });
    expect(await one(response({ type: 'text/html' }))).toEqual({ success: false, error: 'Not an image' });
    expect(await one(response({ length: MAX_BYTES + 1 }))).toEqual({ success: false, error: 'Image too large' });
    expect(await one(response({ body: new Uint8Array(MAX_BYTES + 1) }))).toEqual({ success: false, error: 'Image too large' });
    expect(await one(response({ status: 404 }))).toEqual({ success: false, error: 'Image download failed (404)' });
  });
});

describe('importRule34videoPoster', () => {
  const ok = { success: true, bytes: Buffer.from([1]), contentType: 'image/jpeg' };
  const stored = { path: '/ud/custom-thumbs/x-rule34video.jpg', dataUrl: 'data:image/jpeg;base64,AA' };

  it('imports the cover as is: no frame check', async () => {
    const download = vi.fn().mockResolvedValue(ok);
    const importBytes = vi.fn().mockResolvedValue(stored);
    expect(await importRule34videoPoster(VIDEO, COVER, { download, importBytes })).toEqual({ status: 'imported', ...stored });
    expect(download).toHaveBeenCalledWith(COVER);
    expect(importBytes).toHaveBeenCalledWith(ok.bytes);
  });

  it('calls a link that cannot work again unusable, anything else failed', async () => {
    const importBytes = vi.fn().mockResolvedValue(stored);
    const run = (dl) => importRule34videoPoster(VIDEO, COVER, { download: vi.fn().mockResolvedValue(dl), importBytes });
    for (const error of ['Image download failed (404)', 'Image download failed (410)', 'Not an image', 'Image too large']) {
      expect((await run({ success: false, error })).status).toBe('unusable');
    }
    for (const error of ['Image download failed (503)', 'Image download failed (429)', 'Image download failed (403)', 'fetch failed', 'The operation was aborted due to timeout']) {
      expect((await run({ success: false, error })).status).toBe('failed');
    }
    expect(importBytes).not.toHaveBeenCalled();
    const undecodable = await importRule34videoPoster(VIDEO, COVER, { download: vi.fn().mockResolvedValue(ok), importBytes: vi.fn().mockResolvedValue(null) });
    expect(undecodable).toEqual({ status: 'unusable', error: 'Image could not be decoded' });
  });

  it('never downloads a link that is not a cover', async () => {
    const download = vi.fn();
    const res = await importRule34videoPoster(VIDEO, 'https://evil.example/preview.jpg', { download, importBytes: vi.fn() });
    expect(res.status).toBe('unusable');
    expect(download).not.toHaveBeenCalled();
  });
});
