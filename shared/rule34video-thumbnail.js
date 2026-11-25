'use strict';

// rule34video's cover of a Discovery scene as its library thumbnail. When
// `eroscripts:scout --fetch` downloads a scene's video from a rule34video
// page, it writes the page's cover (`posterUrl`,
// `https://rule34video.com/contents/videos_screenshots/<id rounded down to
// 1000>/<id>/preview.jpg`) into the scene's `eroscripts.json`. The main
// process downloads it once (electron/thumbnail-download.js: no cookies,
// every redirect hop checked, size and time capped) and imports it into
// userData/custom-thumbs (`<hash>-rule34video.jpg`). Unlike an EroScripts
// preview or FapTap's thumbnail it isn't checked against frames: the cover
// belongs to the very video that was downloaded, and creators' titled
// posters (which never match a frame) are what's wanted. No Electron
// dependency: the library scan (shared with the CLIs) uses the URL check.

const { downloadImage } = require('./thumbnail-download');

const HEADERS = {
  'User-Agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36',
  Accept: 'image/*',
  Referer: 'https://rule34video.com/',
};

const POSTER_RE = /^https:\/\/rule34video\.com\/contents\/videos_screenshots\/(\d{1,12})\/(\d{1,12})\/preview\.jpg$/;

/**
 * Is `url` exactly a rule34video cover link? Whole-string match (https,
 * that host, no port, query or fragment), and the folder is the video id
 * rounded down to 1000, as the site lays them out.
 */
function isRule34videoPosterUrl(url) {
  if (typeof url !== 'string') return false;
  const m = POSTER_RE.exec(url);
  if (!m) return false;
  const id = Number(m[2]);
  return id > 0 && Number(m[1]) === Math.floor(id / 1000) * 1000 && String(id) === m[2] && String(Number(m[1])) === m[1];
}

/**
 * Download a rule34video cover into memory.
 *
 * @param {string} url
 * @param {{fetchImpl?: typeof fetch}} [opts]
 */
function downloadRule34videoPoster(url, { fetchImpl = fetch } = {}) {
  return downloadImage(url, { isAllowed: isRule34videoPosterUrl, headers: HEADERS, fetchImpl });
}

// Answers that won't change on the next try: the image is gone, isn't an
// image or is too big. Anything else (network, 5xx, 429, a DDoS-Guard page)
// might work later.
const SETTLED_ERROR = /\((404|410)\)|^Not an image$|^Image too large$|^Unsupported image link$|^Too many redirects$/;

/**
 * Download the cover and import it.
 *
 * @param {string} videoPath
 * @param {string} url
 * @param {object} deps
 * @param {(bytes: Buffer) => Promise<{path: string, dataUrl: string}|null>} deps.importBytes
 * @param {(url: string) => Promise<object>} [deps.download]
 * @returns {Promise<{status: 'imported', path: string, dataUrl: string}
 *   | {status: 'unusable', error: string}
 *   | {status: 'failed', error: string}>}
 *   'unusable': trying the same link again won't help (gone, not an
 *   image, too large, can't be decoded); 'failed': might work next time.
 */
async function importRule34videoPoster(videoPath, url, { importBytes, download = downloadRule34videoPoster }) {
  if (!isRule34videoPosterUrl(url)) return { status: 'unusable', error: 'Unsupported image link' };
  const dl = await download(url);
  if (!dl?.success) {
    const error = dl?.error || 'Download failed';
    return { status: SETTLED_ERROR.test(error) ? 'unusable' : 'failed', error };
  }
  const imported = await importBytes(dl.bytes);
  if (!imported) return { status: 'unusable', error: 'Image could not be decoded' };
  return { status: 'imported', path: imported.path, dataUrl: imported.dataUrl };
}

module.exports = { isRule34videoPosterUrl, downloadRule34videoPoster, importRule34videoPoster };
