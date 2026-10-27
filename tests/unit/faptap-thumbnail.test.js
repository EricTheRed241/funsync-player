/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Main-process half of FapTap thumbnails (electron/faptap-thumbnail.js):
// only FapTap's hosts are fetched (no cookies, every redirect hop checked,
// size capped), and the image is imported only when the preview validator
// says it shows the video.
import { describe, it, expect, vi } from 'vitest';

const {
  isFaptapThumbnailUrl, downloadFaptapThumbnail, matchFaptapThumbnail, MAX_BYTES,
} = await import('../../electron/faptap-thumbnail.js');

const WEBP = 'https://faptap.net/api/assets/thumbnails/2114528228239478785.webp';
const BUNNY = 'https://vz-5d2e7f1a-b3c.b-cdn.net/0a1b2c3d-4e5f/thumbnail.jpg';
const VIDEO = '/lib/_incoming/Zoot/Yoga/Yoga.mp4';

function response({ status = 200, type = 'image/webp', body = new Uint8Array([1, 2, 3]), length, location } = {}) {
  const headers = new Headers();
  if (type) headers.set('content-type', type);
  if (length != null) headers.set('content-length', String(length));
  if (location) headers.set('location', location);
  return new Response(status >= 300 && status < 400 ? null : body, { status, headers });
}

describe('isFaptapThumbnailUrl', () => {
  it.each([
    [WEBP, true],
    [BUNNY, true],
    ['http://faptap.net/api/assets/thumbnails/1.webp', false],
    ['https://www.faptap.net/api/assets/thumbnails/1.webp', false],
    ['https://faptap.net.evil.example/x.webp', false],
    ['https://evilfaptap.net/x.webp', false],
    ['https://cdn.b-cdn.net/thumbnail.jpg', false],
    ['https://vz-abc.b-cdn.net.evil.example/thumbnail.jpg', false],
    ['https://user:pw@faptap.net/x.webp', false],
    ['https://faptap.net:8443/x.webp', false],
    ['not a url', false],
    [null, false],
  ])('%s → %s', (url, ok) => {
    expect(isFaptapThumbnailUrl(url)).toBe(ok);
  });
});

describe('downloadFaptapThumbnail', () => {
  it('fetches the image without cookies, following redirects by hand', async () => {
    const fetchImpl = vi.fn(async () => response());
    const res = await downloadFaptapThumbnail(WEBP, { fetchImpl });
    expect(res).toMatchObject({ success: true, contentType: 'image/webp' });
    expect([...res.bytes]).toEqual([1, 2, 3]);
    const [url, opts] = fetchImpl.mock.calls[0];
    expect(url).toBe(WEBP);
    expect(opts.headers.Cookie).toBeUndefined();
    expect(opts.redirect).toBe('manual');
  });

  it('refuses other hosts without fetching, and redirects off FapTap', async () => {
    const fetchImpl = vi.fn(async () => response({ status: 302, location: 'https://evil.example/x.jpg' }));
    expect(await downloadFaptapThumbnail('https://evil.example/x.jpg', { fetchImpl })).toEqual({ success: false, error: 'Unsupported image link' });
    expect(fetchImpl).not.toHaveBeenCalled();
    expect(await downloadFaptapThumbnail(WEBP, { fetchImpl })).toEqual({ success: false, error: 'Unsupported image link' });
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it('follows a redirect to the Bunny CDN', async () => {
    const fetchImpl = vi.fn()
      .mockResolvedValueOnce(response({ status: 301, location: BUNNY }))
      .mockResolvedValueOnce(response({ type: 'image/jpeg' }));
    expect((await downloadFaptapThumbnail(WEBP, { fetchImpl })).success).toBe(true);
    expect(fetchImpl.mock.calls[1][0]).toBe(BUNNY);
  });

  it('refuses non-images, oversized images and HTTP errors', async () => {
    const one = (r) => downloadFaptapThumbnail(WEBP, { fetchImpl: async () => r });
    expect(await one(response({ type: 'text/html' }))).toEqual({ success: false, error: 'Not an image' });
    expect(await one(response({ length: MAX_BYTES + 1 }))).toEqual({ success: false, error: 'Image too large' });
    expect(await one(response({ body: new Uint8Array(MAX_BYTES + 1) }))).toEqual({ success: false, error: 'Image too large' });
    expect(await one(response({ status: 404 }))).toEqual({ success: false, error: 'Image download failed (404)' });
  });
});

describe('matchFaptapThumbnail', () => {
  const BYTES = Buffer.from([9, 9]);
  const imported = { path: '/ud/custom-thumbs/abc-faptap.jpg', dataUrl: 'data:image/jpeg;base64,AA' };

  function deps(scoredEntry, best) {
    return {
      scoreCandidates: vi.fn(async (_v, candidates) => ({
        scored: [{ ...candidates[0], ...scoredEntry }],
        best: best ? { ...candidates[0], ...scoredEntry } : null,
      })),
      importBytes: vi.fn(async () => imported),
    };
  }

  it('imports the thumbnail when it matches the video', async () => {
    const d = deps({ distance: 5, accepted: true, bytes: BYTES }, true);
    expect(await matchFaptapThumbnail(VIDEO, WEBP, d)).toEqual({ status: 'imported', ...imported, distance: 5 });
    expect(d.scoreCandidates).toHaveBeenCalledWith(VIDEO, [{ url: WEBP, origin: 'faptap' }], expect.any(Function));
    expect(d.importBytes).toHaveBeenCalledWith(BYTES);
  });

  it('imports nothing when it does not match', async () => {
    const d = deps({ distance: 23, accepted: false, bytes: BYTES, reason: 'does not match the video' }, false);
    expect(await matchFaptapThumbnail(VIDEO, WEBP, d)).toEqual({ status: 'no-match', distance: 23, reason: 'does not match the video' });
    expect(d.importBytes).not.toHaveBeenCalled();
  });

  it('reports a failed download as a failure, not a no-match', async () => {
    const d = deps({ distance: null, accepted: false, reason: 'Image download failed (404)' }, false);
    expect(await matchFaptapThumbnail(VIDEO, WEBP, d)).toEqual({ status: 'failed', error: 'Image download failed (404)' });
  });

  it('refuses other hosts before touching the video', async () => {
    const d = deps({}, false);
    expect((await matchFaptapThumbnail(VIDEO, 'https://evil.example/x.jpg', d)).status).toBe('failed');
    expect(d.scoreCandidates).not.toHaveBeenCalled();
  });

  it('fails when the matched image cannot be decoded', async () => {
    const d = deps({ distance: 4, accepted: true, bytes: BYTES }, true);
    d.importBytes.mockResolvedValue(null);
    expect(await matchFaptapThumbnail(VIDEO, BUNNY, d)).toEqual({ status: 'failed', error: 'Image could not be decoded' });
  });
});
