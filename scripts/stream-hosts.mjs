// Videos a post links on a streaming site instead of uploading the file
// (docs/download-hosts/video-sites.md): rule34video.com and pmvhaven.com.
// Used by npm run eroscripts:scout (scripts/eroscripts-scout.mjs): the scan
// turns each linked video page into pseudo video files, which the post's
// scripts pair with like any other video file; --fetch reads the site again
// right before the download and streams the file.
//
// rule34video: plain Node fetch reads its pages, and the player's flashvars
// hold a direct mp4 per quality, up to 4K, one pseudo file per quality. Its
// links carry an access token, hence the fresh read. It answers 429 above ~3
// requests/s: requests go at least RULE34VIDEO_INTERVAL_MS apart, and a 429
// waits RATE_LIMIT_BACKOFF_MS before trying again.
//
// pmvhaven.com: its open JSON API (`/api/videos/<id>`) gives each video's
// public mp4 (one quality), length, size and frame size, so a page is one
// pseudo file; requests go PMVHAVEN_INTERVAL_MS apart.

import { createPacer } from './eroscripts-sync-lib.mjs';
import { NAME_NOISE_TOKEN } from '../shared/content-preferences.js';

export const RULE34VIDEO_HOST = 'rule34video';
export const RULE34VIDEO_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36';
export const RULE34VIDEO_HEADERS = Object.freeze({ 'User-Agent': RULE34VIDEO_UA, Referer: 'https://rule34video.com/' });
/** Least time between two rule34video requests (≤ 2.5 requests/s; it rate-limits above ~3/s). */
export const RULE34VIDEO_INTERVAL_MS = 400;
/** Wait after an HTTP 429, before trying the request again. */
export const RATE_LIMIT_BACKOFF_MS = 60_000;
/** Tries after a 429 before giving up on a request. */
export const RATE_LIMIT_RETRIES = 3;

// ── Links ───────────────────────────────────────────────────────────────

const RULE34VIDEO_PAGE = /^https?:\/\/(?:www\.)?rule34video\.com\/video\/(\d+)(?:\/([^/?#]*))?/i;

/**
 * The rule34video video pages among a post's links, as
 * `https://rule34video.com/video/<id>/<slug>/` (fragment and query
 * dropped), one per video id, in link order. Profile (`/members/…`) and
 * search (`/search/…`) links aren't videos.
 */
export function rule34videoLinks(hrefs) {
  const byId = new Map();
  for (const href of hrefs || []) {
    const m = RULE34VIDEO_PAGE.exec(String(href).split('#')[0]);
    if (m && !byId.has(m[1])) byId.set(m[1], `https://rule34video.com/video/${m[1]}/${m[2] || 'video'}/`);
  }
  return [...byId.values()];
}

/**
 * The cover of a rule34video video page, the image the site shows for it:
 * `https://rule34video.com/contents/videos_screenshots/<id rounded down to
 * 1000>/<id>/preview.jpg` (public, no token). The scout writes it into the
 * scene's `eroscripts.json` (`posterUrl`) and the app makes it the
 * library thumbnail (shared/rule34video-thumbnail.js). null when
 * `pageUrl` isn't a video page.
 */
export function rule34videoPosterUrl(pageUrl) {
  const m = RULE34VIDEO_PAGE.exec(String(pageUrl || ''));
  const id = m ? Number(m[1]) : 0;
  if (!Number.isSafeInteger(id) || id <= 0) return null;
  return `https://rule34video.com/contents/videos_screenshots/${Math.floor(id / 1000) * 1000}/${id}/preview.jpg`;
}

// ── Page ────────────────────────────────────────────────────────────────

const ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' };
const decodeEntities = (s) => String(s).replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (all, e) => {
  if (e[0] === '#') {
    const code = e[1] === 'x' || e[1] === 'X' ? parseInt(e.slice(2), 16) : Number(e.slice(1));
    return Number.isFinite(code) ? String.fromCodePoint(code) : all;
  }
  return ENTITIES[e.toLowerCase()] ?? all;
});

/** `PT0H3M21S` → 201; null when it isn't an ISO 8601 duration. */
export function isoDurationSec(text) {
  const m = /^P(?:(\d+)D)?T?(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?$/.exec(String(text || '').trim());
  if (!m || !m.slice(1).some((x) => x !== undefined)) return null;
  const [d = 0, h = 0, min = 0, s = 0] = m.slice(1).map((x) => Number(x || 0));
  return d * 86400 + h * 3600 + min * 60 + s;
}

/** The page's schema.org VideoObject (JSON-LD), or null. */
function videoObject(html) {
  for (const m of html.matchAll(/<script[^>]+application\/ld\+json[^>]*>([\s\S]*?)<\/script>/gi)) {
    try {
      const j = JSON.parse(m[1]);
      const hit = [j, ...(Array.isArray(j?.['@graph']) ? j['@graph'] : [])].flat().find((x) => x?.['@type'] === 'VideoObject');
      if (hit) return hit;
    } catch {
      // Not valid JSON (a stray control character in the description): take the two fields by hand.
      const field = (k) => {
        const f = new RegExp(`"${k}"\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)"`).exec(m[1]);
        if (!f) return undefined;
        try { return JSON.parse(`"${f[1]}"`); } catch { return f[1]; }
      };
      if (/"VideoObject"/.test(m[1])) return { name: field('name'), duration: field('duration') };
    }
  }
  return null;
}

/** Height of a quality label (`720p`, `4k`, `1080p60`), 0 when it names none. */
export function labelHeight(label) {
  const s = String(label || '').trim().toLowerCase();
  const p = /^(\d{3,4})p/.exec(s);
  if (p) return Number(p[1]);
  const k = /^(\d{1,2})k/.exec(s);
  return k ? Number(k[1]) * 540 : 0;
}

/**
 * What a rule34video video page offers: its title, length (JSON-LD) and the
 * player's mp4 per quality (`video_url` / `video_alt_url<N>` flashvars, each
 * with its `…_text` label: `360p`, `720p`, `1080p`, `4k`). The height comes
 * from the label, else the file name (`…_720p.mp4`). Scrambled links
 * (`function/0/…`, KVS's licence-code obfuscation) are left out.
 *
 * @returns {{title: string, durationSec: number|null, variants: {label: string, height: number, direct: string}[]}}
 */
export function parseRule34videoPage(html) {
  const text = String(html || '');
  const ld = videoObject(text);
  const title = decodeEntities(ld?.name || (/<title>([^<]*)<\/title>/i.exec(text) || [])[1] || '').trim();
  const durationSec = isoDurationSec(ld?.duration);
  const variants = [];
  for (const [, alt, n, direct] of text.matchAll(/\bvideo_(alt_)?url(\d*)\s*:\s*'([^']+)'/g)) {
    if (!/^https?:\/\//.test(direct)) continue;
    const label = (new RegExp(`\\bvideo_${alt || ''}url${n}_text\\s*:\\s*'([^']*)'`).exec(text) || [])[1] || '';
    const height = labelHeight(label) || Number((/_(\d{3,4})p?\.mp4/i.exec(direct) || [])[1] || 0);
    if (height && !variants.some((v) => v.height === height)) variants.push({ label, height, direct });
  }
  variants.sort((a, b) => a.height - b.height);
  return { title, durationSec, variants };
}

/**
 * A page title as a file name: quality words (`4K`, `1080p`, `60fps`; the
 * height goes at the end instead) and characters names can't hold dropped.
 */
export function streamTitle(title) {
  return String(title || '')
    .replace(/[/\\:*?"<>|]/g, ' ')
    .split(/\s+/)
    .filter((w) => w && !NAME_NOISE_TOKEN.test(w.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '')))
    .join(' ')
    .replace(/(?:\s*-\s*)+$/, '')
    .replace(/^(?:\s*-\s*)+/, '')
    .replace(/\s-(?:\s+-)+\s/g, ' - ')
    .trim();
}

/**
 * The pseudo video files of a parsed rule34video page (parseRule34videoPage):
 * one per quality, `{name: '<title> <height>p.mp4', size: 0, host:
 * 'rule34video', url: <page>, height, duration: <s>|null}`. The direct links
 * aren't kept: their token is read again before the download.
 */
export function rule34videoFiles(pageUrl, page) {
  const id = (RULE34VIDEO_PAGE.exec(pageUrl) || [])[1] || '';
  const base = streamTitle(page.title) || `rule34video ${id}`.trim();
  return page.variants.map((v) => ({
    name: `${base} ${v.height}p.mp4`, size: 0, host: RULE34VIDEO_HOST, url: pageUrl, height: v.height, duration: page.durationSec ?? null,
  }));
}

/** A name without the characters file names can't hold, whitespace collapsed. */
export const safeName = (s) => String(s).replace(/[/\\:*?"<>|]/g, '').replace(/\s+/g, ' ').trim();

/** Scene stems stay well under the 255-byte name limit with `.generated.twist.funscript`. */
export const STEM_BYTES = 180;

/** Cut to `max` UTF-8 bytes on a character boundary. */
export function capBytes(s, max) {
  let out = '';
  let n = 0;
  for (const ch of s) {
    n += Buffer.byteLength(ch);
    if (n > max) break;
    out += ch;
  }
  return out.trim();
}

// ── Length check ────────────────────────────────────────────────────────

/** A script may end this much after the video (rounding, a last action on the final frame). */
export const DURATION_SLACK_MS = 2000;
/** … and must cover at least this share of it (scripts may stop before the credits). */
export const DURATION_MIN_SHARE = 0.8;

/**
 * True when a script of `scriptMs` (last action) fits a video of
 * `videoSec`: it ends no later than the video (+2 s) and covers at least 80 %
 * of it. A 15–33 s teaser of a full-length scene fails.
 */
export function durationMatches(videoSec, scriptMs) {
  const videoMs = Number(videoSec) * 1000;
  if (!(videoMs > 0) || !(scriptMs > 0)) return false;
  return scriptMs <= videoMs + DURATION_SLACK_MS && scriptMs >= DURATION_MIN_SHARE * videoMs;
}

/**
 * At download time a script may end this much after the video: scripts often
 * run a few seconds past the file a host serves (an intro or outro cut, a
 * last action held), and the scene was already matched to the post
 * (durationMatches) before anything downloaded.
 */
export const DOWNLOAD_SLACK_MS = 60_000;

/**
 * durationMatches for a download: the script ends no later than the video
 * (+60 s) and covers at least 80 % of it, so a trailer or teaser still fails.
 */
export function downloadFits(videoSec, scriptMs) {
  const videoMs = Number(videoSec) * 1000;
  if (!(videoMs > 0) || !(scriptMs > 0)) return false;
  return scriptMs <= videoMs + DOWNLOAD_SLACK_MS && scriptMs >= DURATION_MIN_SHARE * videoMs;
}

/** A download may be this share off the size a host lists for it (when the server sends no Content-Length). */
export const LISTED_SIZE_SLACK = 0.01;

/**
 * null when a download of `size` bytes is whole, else why not. Whole: exactly
 * the `contentLength` the server sent; without one, within 1 % of the size
 * the host `listed` (hosts list sizes a few bytes off the file they serve:
 * SexLikeReal 40–60 bytes). Neither known: whole.
 */
export function wholeDownload(size, { contentLength = 0, listed = 0 } = {}) {
  const sent = Number(contentLength) || 0;
  if (sent > 0) return size === sent ? null : `size ${size} ≠ Content-Length ${sent}`;
  const want = Number(listed) || 0;
  if (want > 0 && Math.abs(size - want) > LISTED_SIZE_SLACK * want) return `size ${size}, listed ${want}: more than 1 % off`;
  return null;
}

// ── Requests ────────────────────────────────────────────────────────────

const defaultSleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * `fetch` for one host, paced: requests at least `intervalMs` apart, and on
 * HTTP 429 a wait of `backoffMs` and another try, `retries` times, then an
 * error. `fetch`, `now` and `sleep` are injectable for tests.
 */
export function pacedFetch({
  fetch = globalThis.fetch, intervalMs = RULE34VIDEO_INTERVAL_MS, backoffMs = RATE_LIMIT_BACKOFF_MS, retries = RATE_LIMIT_RETRIES,
  now = () => Date.now(), sleep = defaultSleep, label = RULE34VIDEO_HOST,
} = {}) {
  const pacer = createPacer({ intervalMs, now, sleep });
  return async (url, init = {}) => {
    for (let attempt = 0; ; attempt++) {
      await pacer.wait();
      const resp = await fetch(url, init);
      if (resp.status !== 429) return resp;
      await resp.body?.cancel?.().catch(() => {});
      if (attempt >= retries) throw new Error(`${label}: rate limited (HTTP 429)`);
      await sleep(backoffMs);
    }
  };
}

/**
 * rule34video reads for the scout, all through one pacedFetch:
 * - `files(pageUrl)`: the page's pseudo video files (rule34videoFiles).
 * - `probeSize(file)`: the chosen quality's size from a `Range: bytes=0-0`
 *   request (`content-range` total; the page read by `files` is reused), 0
 *   when unknown.
 * - `fresh(file)`: the page read again: `{direct, durationSec}` of the
 *   file's quality, with a new access token.
 * - `page(url)`: the page read now (parseRule34videoPage), every quality's
 *   fresh direct link with it (frames: scripts/scout-frames.mjs).
 * - `stream(direct)`: the download response (HTTP 200/206), else an error.
 */
export function createRule34video(opts = {}) {
  const get = pacedFetch(opts);
  const pages = new Map();
  async function readPage(url) {
    const resp = await get(url, { headers: RULE34VIDEO_HEADERS, signal: AbortSignal.timeout(60_000) });
    if (!resp.ok) {
      await resp.body?.cancel?.().catch(() => {});
      throw Object.assign(new Error(`rule34video: HTTP ${resp.status} for ${url}`), { status: resp.status });
    }
    const page = parseRule34videoPage(await resp.text());
    pages.set(url, page);
    return page;
  }
  const variantOf = (page, file) => page.variants.find((v) => v.height === file.height) || null;
  return {
    async files(url) {
      return rule34videoFiles(url, await readPage(url));
    },
    async probeSize(file) {
      const v = variantOf(pages.get(file.url) || await readPage(file.url), file);
      if (!v) return 0;
      const resp = await get(v.direct, { headers: { ...RULE34VIDEO_HEADERS, Range: 'bytes=0-0' }, signal: AbortSignal.timeout(60_000) });
      await resp.body?.cancel?.().catch(() => {});
      const total = Number((/\/(\d+)\s*$/.exec(resp.headers.get('content-range') || '') || [])[1]);
      return resp.status === 206 && total > 0 ? total : 0;
    },
    async fresh(file) {
      const page = await readPage(file.url);
      const v = variantOf(page, file);
      if (!v) throw new Error(`rule34video: no ${file.height}p any more on ${file.url}`);
      return { direct: v.direct, durationSec: page.durationSec };
    },
    async page(url) {
      return readPage(url);
    },
    async stream(direct) {
      const resp = await get(direct, { headers: RULE34VIDEO_HEADERS });
      if (!resp.ok || !resp.body) {
        await resp.body?.cancel?.().catch(() => {});
        throw new Error(`rule34video: HTTP ${resp.status}`);
      }
      return resp;
    },
  };
}

// ── PMVHaven ────────────────────────────────────────────────────────────

export const PMVHAVEN_HOST = 'pmvhaven';
export const PMVHAVEN_ORIGIN = 'https://pmvhaven.com';
export const PMVHAVEN_HEADERS = Object.freeze({ 'User-Agent': RULE34VIDEO_UA, Accept: 'application/json' });
/** Least time between two pmvhaven.com requests (its API is behind Cloudflare; a scan needs a few per post at most). */
export const PMVHAVEN_INTERVAL_MS = 1000;

// `/video/<slug>_<id>` (the site since 2025: `pink-orbit_6afa…`; before: `Goon-Edge-Leak_6642…`), or the bare id.
const PMVHAVEN_PAGE = /^https?:\/\/(?:www\.)?pmvhaven\.com\/video\/(?:([^/?#]*)_)?([0-9a-f]{24})(?=[/?#]|$)/i;

/** The video id (24 hex digits) of a pmvhaven.com video page, else null. */
export function pmvhavenId(url) {
  return (PMVHAVEN_PAGE.exec(String(url || '').split('#')[0]) || [])[2]?.toLowerCase() || null;
}

/**
 * The pmvhaven.com video pages among a post's links, as
 * `https://pmvhaven.com/video/<slug>_<id>` (query and fragment dropped),
 * one per video id, in link order. Profiles, playlists and search pages
 * aren't videos.
 */
export function pmvhavenLinks(hrefs) {
  const byId = new Map();
  for (const href of hrefs || []) {
    const m = PMVHAVEN_PAGE.exec(String(href).split('#')[0]);
    const id = m?.[2].toLowerCase();
    if (id && !byId.has(id)) byId.set(id, `${PMVHAVEN_ORIGIN}/video/${m[1] ? `${m[1]}_` : ''}${id}`);
  }
  return [...byId.values()];
}

/**
 * A video from `GET /api/videos/<id>` (`{success, data}`): id, title, length
 * (s), frame size, the public mp4 (`direct`, https only), its size in bytes
 * (0 when the API doesn't say) and uploader. null without a playable mp4
 * (removed, still processing).
 *
 * @returns {{id: string, title: string, durationSec: number|null, width: number, height: number, direct: string, size: number, uploader: string}|null}
 */
export function parsePmvhavenVideo(json) {
  const d = json?.data;
  const direct = typeof d?.videoUrl === 'string' ? d.videoUrl.trim() : '';
  if (!d || json.success === false || !/^https:\/\//i.test(direct)) return null;
  const num = (x) => (Number(x) > 0 ? Number(x) : 0);
  return {
    id: String(d._id || ''),
    title: decodeEntities(String(d.title || '')).trim(),
    durationSec: num(d.durationSeconds) || null,
    width: num(d.width),
    height: num(d.height),
    direct,
    size: num(d.fileSize),
    uploader: String(d.uploader || d.uploaderUsername || ''),
  };
}

/**
 * The pseudo video file of a pmvhaven.com page (parsePmvhavenVideo): `{name:
 * '<title> <height>p.mp4', size, host: 'pmvhaven', url: <page>, height,
 * duration: <s>|null, pmvhavenId}`. The mp4 link isn't kept: --fetch asks
 * the API again, in case the video was replaced or removed.
 */
export function pmvhavenFiles(pageUrl, video) {
  const id = pmvhavenId(pageUrl) || video.id;
  const base = streamTitle(video.title) || `pmvhaven ${id}`;
  const height = video.height || 0;
  return [{
    name: `${base}${height ? ` ${height}p` : ''}.mp4`, size: video.size || 0, host: PMVHAVEN_HOST, url: pageUrl, height, duration: video.durationSec ?? null, pmvhavenId: id,
  }];
}

/**
 * pmvhaven.com reads for the scout, all through one pacedFetch:
 * - `files(pageUrl)`: the page's pseudo video file (pmvhavenFiles); an error
 *   with `status` 404 when the video is gone or has no mp4.
 * - `video(id)`: the API's video now (parsePmvhavenVideo; frames:
 *   scripts/scout-frames.mjs).
 * - `probeSize(file)`: the mp4's size from a `Range: bytes=0-0` request, 0
 *   when unknown (the API usually gives it).
 * - `fresh(file)`: the video read again: `{direct, durationSec, size}`.
 * - `stream(direct)`: the download response (HTTP 200/206), else an error.
 */
export function createPmvhaven(opts = {}) {
  const get = pacedFetch({ intervalMs: PMVHAVEN_INTERVAL_MS, label: PMVHAVEN_HOST, ...opts });
  async function video(id) {
    const resp = await get(`${PMVHAVEN_ORIGIN}/api/videos/${id}`, { headers: PMVHAVEN_HEADERS, signal: AbortSignal.timeout(60_000) });
    if (!resp.ok) {
      await resp.body?.cancel?.().catch(() => {});
      throw Object.assign(new Error(`pmvhaven: HTTP ${resp.status} for video ${id}`), { status: resp.status });
    }
    const v = parsePmvhavenVideo(await resp.json());
    if (!v) throw Object.assign(new Error(`pmvhaven: video ${id} has no playable mp4`), { status: 404 });
    return v;
  }
  const idOf = (file) => file.pmvhavenId || pmvhavenId(file.url);
  return {
    async files(url) {
      const id = pmvhavenId(url);
      return id ? pmvhavenFiles(url, await video(id)) : [];
    },
    video,
    async probeSize(file) {
      const { direct } = await video(idOf(file));
      const resp = await get(direct, { headers: { 'User-Agent': RULE34VIDEO_UA, Range: 'bytes=0-0' }, signal: AbortSignal.timeout(60_000) });
      await resp.body?.cancel?.().catch(() => {});
      const total = Number((/\/(\d+)\s*$/.exec(resp.headers.get('content-range') || '') || [])[1]);
      return resp.status === 206 && total > 0 ? total : 0;
    },
    async fresh(file) {
      const v = await video(idOf(file));
      return { direct: v.direct, durationSec: v.durationSec, size: v.size };
    },
    async stream(direct) {
      const resp = await get(direct, { headers: { 'User-Agent': RULE34VIDEO_UA } });
      if (!resp.ok || !resp.body) {
        await resp.body?.cancel?.().catch(() => {});
        throw new Error(`pmvhaven: HTTP ${resp.status}`);
      }
      return resp;
    },
  };
}
