/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The I/O around preview matching: per-video frame samples cached by
// path + size + mtime, bounded sampling concurrency, and scoring a topic's
// candidates. See electron/preview-validator.js.
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const { createPreviewValidator, limiter, SAMPLE_CONCURRENCY } = await import('../../electron/preview-validator.js');

const W = 128;
const H = 72;
function picture(seed, w = W, h = H) {
  const g = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const u = x / w;
      const v = y / h;
      g[y * w + x] = Math.max(0, Math.min(255, Math.round(128
        + 50 * Math.sin(u * (3 + seed) * 2.1 + v * (2 + seed * 0.7) * 1.3 + seed)
        + 40 * Math.cos(v * (4 + (seed % 3)) * 2.7 - u * 1.9 * seed)
        + 25 * Math.sin((u * u + v) * 9 * (1 + seed * 0.13)))));
    }
  }
  return g;
}

// An "encoded image" in these tests is just JSON naming the picture seed
// and the size it claims to have; the fake decoder renders it.
const encoded = (seeds, srcWidth = 1280, srcHeight = 720) => Buffer.from(JSON.stringify({ seeds, srcWidth, srcHeight }));
async function decodeImage(bytes) {
  const { seeds, srcWidth, srcHeight } = JSON.parse(bytes.toString());
  if (!seeds) return [];
  return seeds.map((s) => ({ gray: picture(s), width: W, height: H, srcWidth, srcHeight }));
}

let dir;
let video;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'preview-validator-'));
  video = path.join(dir, 'video.mp4');
  fs.writeFileSync(video, 'v1');
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

function makeValidator(frameSeeds = [3, 1, 4]) {
  const sampleVideo = vi.fn(async () => ({ width: W, height: H, frames: frameSeeds.map((s, i) => ({ pct: 0.1 + i * 0.3, gray: picture(s) })) }));
  const v = createPreviewValidator({ cacheDir: dir, sampleVideo, decodeImage, log: { warn() {} } });
  return { v, sampleVideo };
}

describe('frame sample cache', () => {
  it('samples a video once, again after it changes, and keeps it across restarts', async () => {
    const { v, sampleVideo } = makeValidator();
    const first = await v.videoHashes(video);
    expect(first.length).toBeGreaterThan(0);
    await v.videoHashes(video);
    expect(sampleVideo).toHaveBeenCalledTimes(1);

    fs.writeFileSync(video, 'v2 longer'); // size (and mtime) change
    await v.videoHashes(video);
    expect(sampleVideo).toHaveBeenCalledTimes(2);

    await new Promise((r) => setTimeout(r, 700)); // write-behind
    const cache = JSON.parse(fs.readFileSync(path.join(dir, 'preview-match-cache.json'), 'utf8'));
    expect(cache.entries[video].size).toBe(9);
    const again = makeValidator();
    await again.v.videoHashes(video);
    expect(again.sampleVideo).not.toHaveBeenCalled();
  });

  it('shares one sampling between concurrent requests for the same video', async () => {
    const { v, sampleVideo } = makeValidator();
    await Promise.all([v.videoHashes(video), v.videoHashes(video), v.videoHashes(video)]);
    expect(sampleVideo).toHaveBeenCalledTimes(1);
  });

  it('samples at most SAMPLE_CONCURRENCY videos at once (slow external drive)', async () => {
    let active = 0;
    let peak = 0;
    const sampleVideo = vi.fn(async () => {
      active++;
      peak = Math.max(peak, active);
      await new Promise((r) => setTimeout(r, 10));
      active--;
      return { width: W, height: H, frames: [{ pct: 0.5, gray: picture(1) }] };
    });
    const v = createPreviewValidator({ sampleVideo, decodeImage });
    const videos = Array.from({ length: 6 }, (_, i) => {
      const p = path.join(dir, `v${i}.mp4`);
      fs.writeFileSync(p, String(i));
      return p;
    });
    await Promise.all(videos.map((p) => v.videoHashes(p)));
    expect(sampleVideo).toHaveBeenCalledTimes(6);
    expect(peak).toBe(SAMPLE_CONCURRENCY);
  });

  it('limiter runs everything, never more than max at once', async () => {
    const run = limiter(2);
    let active = 0;
    let peak = 0;
    const results = await Promise.all([1, 2, 3, 4, 5].map((n) => run(async () => {
      active++;
      peak = Math.max(peak, active);
      await new Promise((r) => setTimeout(r, 5));
      active--;
      return n * 2;
    })));
    expect(results).toEqual([2, 4, 6, 8, 10]);
    expect(peak).toBe(2);
  });
});

describe('scoring', () => {
  it('accepts an image of a sampled frame, with its distance and position', async () => {
    const { v } = makeValidator();
    const s = await v.scoreImage(video, encoded([1]));
    expect(s).toMatchObject({ accepted: true, distance: 0, view: 'full', reason: null, width: 1280, height: 720 });
    expect(s.pct).toBeCloseTo(0.4);
  });

  it('rejects an image that is not in the video', async () => {
    const { v } = makeValidator();
    expect(await v.scoreImage(video, encoded([9]))).toMatchObject({ accepted: false, reason: 'does not match the video' });
  });

  it('rejects undecodable images and the wrong size or shape once decoded', async () => {
    const { v } = makeValidator();
    expect(await v.scoreImage(video, encoded(null))).toMatchObject({ accepted: false, distance: null, reason: 'undecodable' });
    expect(await v.scoreImage(video, encoded([1], 300, 169))).toMatchObject({ accepted: false, reason: 'too small' });
    expect(await v.scoreImage(video, encoded([1], 600, 900))).toMatchObject({ accepted: false, reason: 'wrong aspect' });
  });

  it('scores every candidate and picks the closest accepted one (ties: listed first)', async () => {
    const { v } = makeValidator([3, 1, 4]);
    const files = { a: encoded([9]), b: encoded([7, 4]), c: encoded([1]) };
    const download = vi.fn(async (url) => (url === 'x' ? { success: false, error: 'Image download failed (404)' } : { success: true, bytes: files[url] }));
    const candidates = ['a', 'x', 'b', 'c'].map((url) => ({ url, origin: 'post' }));
    const { scored, best } = await v.scoreCandidates(video, candidates, download);
    expect(scored.map((s) => [s.url, s.accepted])).toEqual([['a', false], ['x', false], ['b', true], ['c', true]]);
    expect(scored[1].reason).toBe('Image download failed (404)');
    expect(scored[2].frame).toBe(1); // the second frame of the animation
    expect(best.url).toBe('b');
    expect(best.bytes).toBe(files.b);
  });

  it('returns no best when nothing matches', async () => {
    const { v } = makeValidator();
    const { best } = await v.scoreCandidates(video, [{ url: 'a' }], async () => ({ success: true, bytes: encoded([9]) }));
    expect(best).toBeNull();
  });
});
