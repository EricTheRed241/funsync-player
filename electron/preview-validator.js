// Validates EroScripts preview images against the video they'd illustrate
// (see preview-match.js for the rules and the hashing). This module owns the
// I/O around it: frame samples per video (cached by path + size + mtime, at
// most SAMPLE_CONCURRENCY at once: the library lives on a slow external
// drive), image decoding, and the scoring of a topic's candidates.
//
// Decoding and sampling are injected so the app (backend sampler +
// nativeImage/Chromium) and the dry-run CLI (backend sampler in a Python
// child + ffmpeg) share the logic.

const fs = require('fs');
const path = require('path');
const pm = require('./preview-match');

/** Fewest frames sampled per video, across the middle 90% of its duration.
 *  The backend returns every keyframe there (up to 400) when it can; see
 *  backend/services/ffmpeg.py sample_gray_frames. */
const SAMPLE_COUNT = 12;
/** Width of the sampled grayscale frames. A 32×32 hash input needs little. */
const SAMPLE_WIDTH = 128;
const SAMPLE_CONCURRENCY = 2;
// Bumped when the sampling or hashing changes, so old cache entries die.
const CACHE_VERSION = 1;
const CACHE_FILE = 'preview-match-cache.json';

/** A tiny counting semaphore. */
function limiter(max) {
  let active = 0;
  const queue = [];
  const next = () => {
    if (active >= max || queue.length === 0) return;
    active++;
    const { fn, resolve, reject } = queue.shift();
    Promise.resolve().then(fn).then(resolve, reject).finally(() => { active--; next(); });
  };
  return (fn) => new Promise((resolve, reject) => { queue.push({ fn, resolve, reject }); next(); });
}

/**
 * @param {object} deps
 * @param {string} [deps.cacheDir] where preview-match-cache.json lives (none: memory only)
 * @param {(videoPath: string, count: number, width: number) => Promise<{width: number,
 *   height: number, frames: Array<{pct: number, gray: Uint8Array}>}>} deps.sampleVideo
 * @param {(bytes: Buffer) => Promise<Array<{gray: Uint8Array, width: number, height: number,
 *   srcWidth: number, srcHeight: number}>>} deps.decodeImage frames of an image
 *   (several for an animation), grayscale, plus the decoded size
 * @param {{warn?: Function}} [deps.log]
 * @param {number} [deps.sampleCount]
 */
function createPreviewValidator({ cacheDir, sampleVideo, decodeImage, log = console, sampleCount = SAMPLE_COUNT }) {
  const cachePath = cacheDir ? path.join(cacheDir, CACHE_FILE) : null;
  let cache = null; // videoPath → { size, mtimeMs, hashes }
  const inflight = new Map();
  const limit = limiter(SAMPLE_CONCURRENCY);
  let saveTimer = null;

  const loadCache = () => {
    if (cache) return cache;
    cache = {};
    if (!cachePath) return cache;
    try {
      const data = JSON.parse(fs.readFileSync(cachePath, 'utf8'));
      if (data?.version === CACHE_VERSION && data.sampleCount === sampleCount && data.entries) cache = data.entries;
    } catch { /* first run or unreadable: start over */ }
    return cache;
  };

  const saveCache = () => {
    if (!cachePath) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      try {
        fs.mkdirSync(path.dirname(cachePath), { recursive: true });
        const tmp = `${cachePath}.tmp`;
        fs.writeFileSync(tmp, JSON.stringify({ version: CACHE_VERSION, sampleCount, entries: cache }));
        fs.renameSync(tmp, cachePath);
      } catch (err) {
        log.warn?.('[PreviewMatch] cache write failed:', err.message);
      }
    }, 500);
    saveTimer.unref?.();
  };

  /**
   * Hashes of the video's sampled frames. Cached per path + size + mtime.
   * @returns {Promise<Array<{pct: number, view: string, hash: string}>>}
   */
  async function videoHashes(videoPath) {
    const stat = await fs.promises.stat(videoPath);
    const entries = loadCache();
    const hit = entries[videoPath];
    if (hit && hit.size === stat.size && hit.mtimeMs === stat.mtimeMs) return hit.hashes;
    const key = `${videoPath}\0${stat.size}\0${stat.mtimeMs}`;
    if (inflight.has(key)) return inflight.get(key);
    const job = limit(async () => {
      const sample = await sampleVideo(videoPath, sampleCount, SAMPLE_WIDTH);
      const hashes = pm.videoHashes(sample);
      entries[videoPath] = { size: stat.size, mtimeMs: stat.mtimeMs, hashes };
      saveCache();
      return hashes;
    }).finally(() => inflight.delete(key));
    inflight.set(key, job);
    return job;
  }

  /**
   * Decode and hash an image.
   * @returns {Promise<{hashes: Array<{frame: number, hash: string}>, width: number,
   *   height: number, reason: string|null}>} `reason` says why it can't match
   */
  async function hashImage(bytes) {
    let frames;
    try {
      frames = await decodeImage(bytes);
    } catch {
      frames = null;
    }
    if (!frames?.length) return { hashes: [], width: 0, height: 0, reason: 'undecodable' };
    const { srcWidth: width, srcHeight: height } = frames[0];
    const sizeReason = pm.dimensionExclusion(width, height);
    if (sizeReason) return { hashes: [], width, height, reason: sizeReason };
    const hashes = pm.imageHashes(frames);
    return { hashes, width, height, reason: hashes.length ? null : 'too flat' };
  }

  /**
   * Score an image against a video.
   * @returns {Promise<{distance: number|null, accepted: boolean, frame: number,
   *   pct: number|null, view: string|null, width: number, height: number, reason: string|null}>}
   */
  async function scoreImage(videoPath, bytes, vHashes) {
    const vid = vHashes || await videoHashes(videoPath);
    const img = await hashImage(bytes);
    const match = img.reason ? null : pm.bestMatch(img.hashes, vid);
    const accepted = pm.isAccepted(match);
    let reason = img.reason;
    if (!reason && !match) reason = 'no comparable video frames';
    if (!reason && !accepted) reason = 'does not match the video';
    return {
      distance: match ? match.distance : null,
      accepted,
      frame: match ? match.frame : 0,
      pct: match ? match.pct : null,
      view: match ? match.view : null,
      width: img.width,
      height: img.height,
      reason,
    };
  }

  /**
   * Score a topic's candidates for a video. Downloads go one at a time.
   *
   * @param {string} videoPath
   * @param {Array<{url: string, origin: string}>} candidates
   * @param {(url: string) => Promise<{success: boolean, bytes?: Buffer, error?: string}>} download
   * @returns {Promise<{scored: Array<object>, best: object|null}>}
   */
  async function scoreCandidates(videoPath, candidates, download) {
    const vid = await videoHashes(videoPath);
    const scored = [];
    for (const c of candidates) {
      const dl = await download(c.url);
      if (!dl?.success) {
        scored.push({ ...c, distance: null, accepted: false, reason: dl?.error || 'download failed' });
        continue;
      }
      scored.push({ ...c, ...(await scoreImage(videoPath, dl.bytes, vid)), bytes: dl.bytes });
    }
    const best = pm.pickBest(scored.map((s) => ({ ...s, match: s.distance == null ? null : { distance: s.distance } })));
    return { scored, best: best ? scored.find((s) => s.url === best.url) : null };
  }

  return { videoHashes, hashImage, scoreImage, scoreCandidates };
}

module.exports = { createPreviewValidator, limiter, SAMPLE_COUNT, SAMPLE_WIDTH, SAMPLE_CONCURRENCY };
