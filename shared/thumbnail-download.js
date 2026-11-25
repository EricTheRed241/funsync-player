'use strict';

// Safe download of a thumbnail image from an allow-listed host (FapTap's
// thumbnail, electron/faptap-thumbnail.js; rule34video's cover,
// electron/rule34video-thumbnail.js). No cookies, every redirect hop checked
// against the same allow-list, image content types only, size and time
// capped. No Electron dependency.

const { readCapped } = require('./read-capped');

const MAX_BYTES = 5 * 1024 * 1024;
const MAX_REDIRECTS = 3;
const TIMEOUT_MS = 20_000;

/**
 * Download an image into memory.
 *
 * @param {string} url
 * @param {object} opts
 * @param {(url: string) => boolean} opts.isAllowed host allow-list, checked on every hop
 * @param {Record<string, string>} opts.headers
 * @param {typeof fetch} [opts.fetchImpl]
 * @returns {Promise<{success: true, bytes: Buffer, contentType: string}
 *   | {success: false, error: string}>}
 */
async function downloadImage(url, { isAllowed, headers, fetchImpl = fetch }) {
  let current = url;
  try {
    for (let hop = 0; hop <= MAX_REDIRECTS; hop++) {
      if (!isAllowed(current)) return { success: false, error: 'Unsupported image link' };
      const resp = await fetchImpl(current, {
        headers, redirect: 'manual', credentials: 'omit', signal: AbortSignal.timeout(TIMEOUT_MS),
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

module.exports = { downloadImage, MAX_BYTES };
