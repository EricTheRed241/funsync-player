'use strict';

// FapTap's own thumbnail of a Discovery scene as its library thumbnail,
// like an EroScripts post preview: downloaded once by the main process,
// checked against frames of the video (electron/preview-validator.js) and,
// only when it shows the video, imported into userData/custom-thumbs
// (custom-thumbs.js `importThumbnail`, `<hash>-faptap.jpg`). Nothing
// hot-links FapTap.
//
// `faptap:scout` writes the URL into the scene's `faptap.json`
// (`thumbnailUrl`). Only FapTap's own hosts are fetched: faptap.net
// (`/api/assets/thumbnails/<id>.webp`) and its Bunny CDN pull zones
// (`vz-<zone>.b-cdn.net/…/thumbnail.jpg`). No cookies, https only, every
// redirect hop checked, size and time capped. No Electron dependency: the
// library scan (shared with the CLIs) uses the host check too.

const { readCapped } = require('./read-capped');

const FAPTAP_HOST = 'faptap.net';
const BUNNY_HOST_RE = /^vz-[a-z0-9-]+\.b-cdn\.net$/;
const MAX_BYTES = 5 * 1024 * 1024;
const MAX_REDIRECTS = 3;
const TIMEOUT_MS = 20_000;
const HEADERS = { 'User-Agent': 'Mozilla/5.0', Accept: 'image/*' };

/** Is `url` an https link to a FapTap thumbnail host? */
function isFaptapThumbnailUrl(url) {
  if (typeof url !== 'string' || !url) return false;
  let u;
  try { u = new URL(url); } catch { return false; }
  if (u.protocol !== 'https:' || u.username || u.password || u.port) return false;
  return u.hostname === FAPTAP_HOST || BUNNY_HOST_RE.test(u.hostname);
}

/**
 * Download a FapTap thumbnail into memory.
 *
 * @param {string} url
 * @param {{fetchImpl?: typeof fetch}} [opts]
 * @returns {Promise<{success: true, bytes: Buffer, contentType: string}
 *   | {success: false, error: string}>}
 */
async function downloadFaptapThumbnail(url, { fetchImpl = fetch } = {}) {
  let current = url;
  try {
    for (let hop = 0; hop <= MAX_REDIRECTS; hop++) {
      if (!isFaptapThumbnailUrl(current)) return { success: false, error: 'Unsupported image link' };
      const resp = await fetchImpl(current, {
        headers: HEADERS, redirect: 'manual', credentials: 'omit', signal: AbortSignal.timeout(TIMEOUT_MS),
      });
      if (resp.status >= 300 && resp.status < 400) {
        const location = resp.headers.get('location');
        if (!location) return { success: false, error: `Image download failed (${resp.status})` };
        current = new URL(location, current).href;
        continue;
      }
      if (!resp.ok) return { success: false, error: `Image download failed (${resp.status})` };
      const contentType = (resp.headers.get('content-type') || '').split(';')[0].trim().toLowerCase();
      if (!contentType.startsWith('image/')) return { success: false, error: 'Not an image' };
      if (Number(resp.headers.get('content-length')) > MAX_BYTES) return { success: false, error: 'Image too large' };
      const bytes = await readCapped(resp, MAX_BYTES);
      if (!bytes) return { success: false, error: 'Image too large' };
      return { success: true, bytes, contentType };
    }
    return { success: false, error: 'Too many redirects' };
  } catch (err) {
    return { success: false, error: err.message };
  }
}

/**
 * Score the thumbnail against the video and import it when it matches.
 *
 * @param {string} videoPath
 * @param {string} url
 * @param {object} deps
 * @param {(videoPath: string, candidates: object[], download: Function) =>
 *   Promise<{scored: object[], best: object|null}>} deps.scoreCandidates the preview validator's
 * @param {(bytes: Buffer) => Promise<{path: string, dataUrl: string}|null>} deps.importBytes
 * @param {(url: string) => Promise<object>} [deps.download]
 * @returns {Promise<{status: 'imported', path: string, dataUrl: string, distance: number}
 *   | {status: 'no-match', distance: number|null, reason?: string|null}
 *   | {status: 'failed', error: string}>}
 */
async function matchFaptapThumbnail(videoPath, url, { scoreCandidates, importBytes, download = downloadFaptapThumbnail }) {
  if (!isFaptapThumbnailUrl(url)) return { status: 'failed', error: 'Unsupported image link' };
  const { scored, best } = await scoreCandidates(videoPath, [{ url, origin: 'faptap' }], download);
  if (!best) {
    const s = scored[0];
    // No bytes: the download failed, which says nothing about the image.
    if (!s?.bytes) return { status: 'failed', error: s?.reason || 'Download failed' };
    return { status: 'no-match', distance: s.distance ?? null, reason: s.reason || null };
  }
  const imported = await importBytes(best.bytes);
  if (!imported) return { status: 'failed', error: 'Image could not be decoded' };
  return { status: 'imported', path: imported.path, dataUrl: imported.dataUrl, distance: best.distance };
}

module.exports = { isFaptapThumbnailUrl, downloadFaptapThumbnail, matchFaptapThumbnail, MAX_BYTES };
