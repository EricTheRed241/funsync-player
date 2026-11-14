// SexLikeReal (sexlikereal.com) as a video host for npm run eroscripts:scout
// (scripts/eroscripts-scout.mjs): a Free Scripts post whose video is an SLR
// scene gets that video from the user's paid SLR account.
//
// Scan / --add (no login): a post's `sexlikereal.com/scenes/<label>-<id>`
// links, else, for a post without any video that names SLR, SLR's scene
// search on its title (a hit counts only when its length fits the post's main
// script: durationMatches). Each scene (`/v3/scenes/<id>`, public) becomes one
// pseudo video file named `<Performers> - <Title> (<Studio>)<VR suffix>.mp4`
// with its length, so it pairs with the post's scripts like any other video
// (planWithStreams checks the length).
//
// --fetch: the session `slr:scout --login` saved (`<userData>/slr-session.json`,
// the refresh_jwt cookie) buys an access token; before each video the
// account's download quota (downloadVideo, 35 per rolling 7 days) must have
// one left; the download type is chosen from the scene's download options
// within the caps (never the trailer), and a fresh link is asked for right
// before the download (that request uses up one download). Endpoints and
// answers as SLR's own web client uses them (www.sexlikereal.com/_astro
// bundles, 2026-11-13): `GET /content/v1/videos/<id>/download-options` →
// `{data: {options: [{type, height, width, size, codec, isDownloadable,
// isAvailableThroughPurchase}]}}`, `GET /content/v1/videos/<id>/download?type=`
// → `{data: {url, availableDownloadsCount}}`. API reads go at least
// SLR_INTERVAL_MS apart. Tokens and cookies are never printed.

import fs from 'node:fs';
import path from 'node:path';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { canonicalArtist } from '../renderer/js/artist-playlists.js';
import { FLAT_MAX_HEIGHT, VR_MAX_FPS, VR_MAX_HEIGHT } from '../renderer/js/content-preferences.js';
import { capBytes, durationMatches, pacedFetch, safeName, STEM_BYTES } from './stream-hosts.mjs';

export const SLR_HOST = 'slr';
export const SLR_API = 'https://api.sexlikereal.com';
export const SLR_SITE = 'https://www.sexlikereal.com';
/** Least time between two SLR requests. */
export const SLR_INTERVAL_MS = 350;
/** The session `npm run slr:scout -- --login` saves in userData (mode 0600). */
export const SLR_SESSION_FILE = 'slr-session.json';
export const SLR_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36';
/** How --manual-list shows an SLR video: --fetch takes it with the account, nothing to do by hand. */
export const SLR_ACCOUNT_DOWNLOAD = 'SLR (account download)';
const API_HEADERS = Object.freeze({ 'Client-Type': 'web', project: '1', Accept: 'application/json', 'User-Agent': SLR_UA });
const LOGIN_HINT = 'log in with npm run slr:scout -- --login';

/** No SLR session, or an expired one: SLR scenes can't be downloaded (exit code 3). */
export class SlrLoginError extends Error {
  constructor(message) {
    super(message);
    this.name = 'SlrLoginError';
  }
}

/** The account's video downloads are used up for now (exit code 2: work left). */
export class SlrQuotaError extends Error {
  constructor(quota) {
    const at = typeof quota.availableAt === 'number' ? quota.availableAt * 1000 : Date.parse(quota.availableAt ?? '');
    const when = Number.isFinite(at) ? `; the next one frees up ${new Date(at).toISOString()}` : '';
    super(`SexLikeReal: no video downloads left (0 of ${quota.total ?? '?'} per rolling 7 days${when})`);
    this.name = 'SlrQuotaError';
    this.quota = quota;
  }
}

// ── Links ───────────────────────────────────────────────────────────────

const SCENE_LINK = /^https?:\/\/(?:www\.)?sexlikereal\.com\/(?:[a-z]{2}\/)?scenes\/([a-z0-9][a-z0-9-]*?)\/?(?:[?#]|$)/i;

/**
 * The SLR scene pages among a post's links, as `{id, label, url}`, one per
 * scene id, in link order: `sexlikereal.com/scenes/<label>` where the label
 * ends in `-<id>`, or `/scenes/<id>`; with or without `www.`, query and
 * fragment dropped. `label` is null for a bare id.
 */
export function slrLinks(hrefs) {
  const byId = new Map();
  for (const href of hrefs || []) {
    const ref = SCENE_LINK.exec(String(href).trim())?.[1].toLowerCase();
    const id = ref && /(?:^|-)(\d+)$/.exec(ref)?.[1];
    if (!id || byId.has(id)) continue;
    const label = ref === id ? null : ref;
    byId.set(id, { id: Number(id), label, url: `${SLR_SITE}/scenes/${label || id}` });
  }
  return [...byId.values()];
}

const SLR_WORD = /(?:^|[^a-z0-9])(?:slr|sex\s*like\s*real)(?:[^a-z0-9]|$)/i;

/**
 * True when a post names SexLikeReal: `SLR` or `SexLikeReal` in its title or
 * a tag, or `sexlikereal` in its first post (what the `slr` search feed finds).
 */
export function namesSlr({ title = '', tags = [], text = '' } = {}) {
  return SLR_WORD.test(String(title)) || (tags || []).some((t) => SLR_WORD.test(String(t).replace(/-/g, ' '))) || /sex\s*like\s*real/i.test(String(text));
}

// Title parts and words that label the post or the site, not the scene.
const LABEL_PART = /^(?:slr|sex\s*like\s*real(?:\.com)?|slr\s*originals?|vr|funscripts?|scripts?|multi[- ]?axis|\d+[- ]?axis|remaster(?:ed)?|requests?|requested|\d+k(?:\s*\d+\s*fps)?|\d{3,4}p|\d+\s*fps)$/i;
const LABEL_WORD = /^(?:slr|sexlikereal|funscripts?|multi-?axis|\d+k|\d{3,4}p|\d+fps)$/i;
const letters = (s) => (String(s).match(/\p{L}/gu) || []).length;

/**
 * Searches for the scene of a post titled `title`, most specific first, at
 * most 3: the title's parts (split at brackets, `|`, `:` and ` - `) without
 * those that only label the post (`SLR`, `SexLikeReal`, `VR`, `Script`,
 * `Multi-Axis`, `8K` …) joined, then each part, longest first. Emoji codes
 * go. [] when nothing with 3 letters is left.
 */
export function slrSearchQueries(title) {
  const parts = String(title || '').replace(/:[a-z0-9_+-]+:/gi, ' ')
    .split(/[[\]()（）【】{}|｜:]|\s+[-–—]\s+/)
    .filter((p) => !LABEL_PART.test(p.trim()))
    .map((p) => p.split(/\s+/).filter((w) => w && !LABEL_WORD.test(w.replace(/[^\p{L}\p{N}-]/gu, ''))).join(' ').trim())
    .filter((p) => letters(p) >= 3 && !LABEL_PART.test(p));
  const queries = [parts.join(' '), ...[...parts].sort((a, b) => b.length - a.length)];
  return [...new Set(queries.filter(Boolean))].slice(0, 3);
}

/**
 * Where a post's SLR scenes may be: one `link` lead per scene link
 * (slrLinks); without any, and when the post offers no video of its own
 * (`hasVideo`) but names SLR (namesSlr), one `search` lead with its title's
 * queries (slrSearchQueries).
 *
 * @returns {({from: 'link', id: number, label: string|null, url: string} | {from: 'search', queries: string[]})[]}
 */
export function slrLeads({ hrefs = [], title = '', tags = [], text = '', hasVideo = false } = {}) {
  const links = slrLinks(hrefs);
  if (links.length) return links.map((l) => ({ from: 'link', ...l }));
  if (hasVideo || !namesSlr({ title, tags, text })) return [];
  const queries = slrSearchQueries(title);
  return queries.length ? [{ from: 'search', queries }] : [];
}

const words = (s) => new Set(String(s || '').normalize('NFKD').replace(/\p{M}+/gu, '').toLowerCase()
  .split(/[^\p{L}\p{N}]+/u).filter((w) => w.length >= 3 && !LABEL_WORD.test(w)));

/**
 * The search hit (`/v3/search?tab=scenes` data: `{id, label, title,
 * fullVideoLength, actors, studio}`) that is the post's scene: its length
 * fits one of the post's main scripts (`scriptLengths`, ms; durationMatches:
 * the script ends at most 2 s after the video and covers ≥ 80 % of it), and
 * its title, performers or studio share a word with the post title; the one
 * sharing the most words, then the first. null when none fits.
 */
export function pickSearchHit(hits, { scriptLengths = [], title = '' } = {}) {
  const want = words(title);
  let best = null;
  for (const h of hits || []) {
    if (!scriptLengths.some((ms) => durationMatches(h?.fullVideoLength, ms))) continue;
    const have = words([h.title, h.studio?.name, ...(h.actors || []).map((a) => a?.name)].join(' '));
    const shared = [...want].filter((w) => have.has(w)).length;
    if (shared && (!best || shared > best.shared)) best = { h, shared };
  }
  return best?.h || null;
}

// ── Pseudo file ─────────────────────────────────────────────────────────

// SLR's screen types (its web client's enum): 3 flat, 4 dome, 5 sphere, 6
// MKX200, 7 MKX220, 8 VRCA220, 9 fisheye, 10 RF52, 11 fisheye 190.
const SCREEN_SUFFIX = { 6: '_MKX200', 7: '_MKX220', 8: '_VRCA220', 9: '_FISHEYE190', 10: '_RF52', 11: '_FISHEYE190' };
const LENS_SUFFIX = { mkx200: '_MKX200', mkx220: '_MKX220', vrca220: '_VRCA220', rf52: '_RF52' };

/**
 * The file-name suffix HereSphere / DeoVR (and the app's VR detection) read
 * for a scene's projection (`/v3/scenes/<id>` data): '' for flat; the lens
 * for fisheye screen types (`_MKX200`, `_MKX220`, `_VRCA220`, `_RF52`, else
 * `_FISHEYE190`); else equirectangular 180 or 360 by its stereo mode:
 * `_180_LR`, `_180_TB`, `_MONO180`, `_360_LR` ….
 */
export function slrVrSuffix(scene) {
  const type = Number(scene?.screenType);
  if (scene?.isFlatScreenType === true || type === 3) return '';
  const fixed = SCREEN_SUFFIX[type] || LENS_SUFFIX[String(scene?.projectionParams?.cameraLens || '').toLowerCase()];
  if (fixed) return fixed;
  const angle = type === 5 || Number(scene?.viewAngle) >= 360 ? 360 : 180;
  const mode = String(scene?.stereomode || '').toLowerCase();
  if (mode === 'mono') return `_MONO${angle}`;
  return /^(?:tb|ou)/.test(mode) ? `_${angle}_TB` : `_${angle}_LR`;
}

/**
 * Name of an SLR scene (the pseudo file, or a sidecar's `slr` record:
 * `{performers, title, studio, vrSuffix}`): folder = the first performer
 * (canonicalArtist), else the studio; stem = `<Performers> - <Title>
 * (<Studio>)` plus the VR suffix, as `Miss Lexa, Chloe Wild - Sharing is
 * caring a wife’s surprise (Miss Lexa VR)_180_LR`. Without performers the
 * studio stands in for them; without a studio it is `SexLikeReal`.
 * Performers past the length cap are left out from the end.
 */
export function slrSceneName({ performers = [], title = '', studio = null, vrSuffix = '' }) {
  const people = (performers || []).map((p) => safeName(p)).filter(Boolean);
  const label = safeName(studio || '') || 'SexLikeReal';
  const core = safeName(title || '') || 'Scene';
  const named = (who) => `${who.length ? who.join(', ') : label} - ${core} (${label})`;
  let who = people;
  while (who.length > 1 && Buffer.byteLength(named(who)) > STEM_BYTES) who = who.slice(0, -1);
  return {
    creator: safeName(canonicalArtist(people[0] || '')) || label,
    stem: capBytes(named(who), STEM_BYTES).replace(/[\s\-–—,;.(]+$/, '') + (vrSuffix || ''),
  };
}

/**
 * A scene's details (`/v3/scenes/<id>` data) as the scout's one video file:
 * `{name: '<stem>.mp4' (slrSceneName), host: 'slr', slrSceneId, slrLabel,
 * url: <scene page>, size: null, height: null, duration: <s>|null, vr, fps,
 * title, performers, studio, vrSuffix}`. Size and height come from the
 * download options, read with the account at --fetch.
 */
export function slrFile(scene) {
  const id = Number(scene.id);
  const label = String(scene.label || id);
  const performers = (scene.actors || []).map((a) => a?.name).filter(Boolean);
  const studio = scene.studio?.name || null;
  const vr = scene.isFlatScreenType !== true;
  const vrSuffix = vr ? slrVrSuffix(scene) : '';
  const title = String(scene.title || '');
  return {
    name: `${slrSceneName({ performers, title, studio, vrSuffix }).stem}.mp4`,
    host: SLR_HOST, slrSceneId: id, slrLabel: label, url: `${SLR_SITE}/scenes/${label}`, size: null, height: null,
    duration: Number(scene.fullVideoLength) || null, vr, fps: Number(scene.fps) || null, title, performers, studio, vrSuffix,
  };
}

// ── Download choice ─────────────────────────────────────────────────────

/** Download options as a list, from the array content/v1 answers or the object keyed by type /v3 answers. */
export function slrDownloadOptions(options) {
  if (Array.isArray(options)) return options.filter((o) => o && o.type);
  return Object.entries(options || {}).map(([type, o]) => ({ ...o, type }));
}

/** The download types worth taking, best first: SLR's encode at the scene's top resolution, then its smaller one. Never `original` (the studio master, a far higher bitrate) or `trailer`. */
const DOWNLOAD_TYPES = ['maxResolution', 'optimalResolution'];

/**
 * The download to take of a scene's options (slrDownloadOptions):
 * `maxResolution` when it is within the caps, else `optimalResolution`
 * (VR: height ≤ VR_MAX_HEIGHT and ≤ VR_MAX_FPS frames per second, the
 * codec's `_<n>fps` or the scene's `fps`; flat: height ≤ FLAT_MAX_HEIGHT),
 * downloadable without buying the scene. null when neither fits.
 */
export function chooseSlrDownload(options, { vr = true, fps = null } = {}) {
  const cap = vr ? VR_MAX_HEIGHT : FLAT_MAX_HEIGHT;
  const list = slrDownloadOptions(options);
  for (const type of DOWNLOAD_TYPES) {
    const o = list.find((x) => x.type === type);
    if (!o || o.isDownloadable === false || o.isAvailableThroughPurchase === true) continue;
    const height = Number(o.height || o.resolution) || 0;
    if (!height || height > cap) continue;
    const rate = Number(/_(\d+)fps/i.exec(String(o.codec || ''))?.[1]) || Number(fps) || 0;
    if (vr && rate > VR_MAX_FPS) continue;
    return { ...o, height };
  }
  return null;
}

// ── Session ─────────────────────────────────────────────────────────────

const clock = (ms) => `${Math.floor(ms / 60_000)}:${String(Math.round(ms / 1000) % 60).padStart(2, '0')}`;
const gb = (n) => `${(n / 1e9).toFixed(2)} GB`;

/** When an access token (a JWT) expires, in ms; null when it doesn't say. */
function tokenExpiry(token) {
  try {
    const exp = JSON.parse(Buffer.from(String(token).split('.')[1], 'base64url').toString('utf8')).exp;
    return Number.isFinite(exp) ? exp * 1000 : null;
  } catch {
    return null;
  }
}

/** A new `refresh_jwt` among a response's Set-Cookie headers: `{value, expires: ISO|null}`, null when there is none (or it's deleted). */
export function rotatedRefreshJwt(setCookies, now = Date.now()) {
  let out = null;
  for (const header of setCookies || []) {
    const m = /^\s*refresh_jwt=([^;]*)/i.exec(header);
    if (!m) continue;
    const attrs = header.split(';').slice(1).map((a) => a.trim());
    const maxAge = attrs.find((a) => /^max-age=/i.test(a));
    const expiresAttr = attrs.find((a) => /^expires=/i.test(a));
    const age = maxAge ? Number(maxAge.split('=')[1]) : null;
    if (!m[1] || (age !== null && age <= 0)) continue;
    const at = age !== null ? now + age * 1000 : expiresAttr ? Date.parse(expiresAttr.slice(8)) : NaN;
    out = { value: m[1], expires: Number.isFinite(at) ? new Date(at).toISOString() : null };
  }
  return out;
}

/** Save the session atomically, readable by this user only (0600). */
function writeSession(file, session) {
  const tmp = `${file}.${process.pid}.tmp`;
  try {
    fs.writeFileSync(tmp, `${JSON.stringify(session, null, 2)}\n`, { mode: 0o600 });
    fs.renameSync(tmp, file);
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    throw err;
  }
}

/**
 * SexLikeReal for the scout, every request through one pacedFetch
 * (≥ SLR_INTERVAL_MS apart, a wait after HTTP 429):
 *
 * - `filesFor(leads, {scriptLengths, title})`: the scenes slrLeads point at,
 *   one pseudo video file each (slrFile) → `{files, missed: [{lead, why}],
 *   down}`. A search lead takes pickSearchHit's hit (none without
 *   `scriptLengths`, the post's main scripts in ms). `down`: SLR failed for
 *   a reason other than a missing scene, so the post may have a video later.
 * - `quota()`: the account's video downloads `{remaining, total,
 *   availableAt}`; needs the session (SlrLoginError without one).
 * - `ready()`: quota(), and a SlrQuotaError when none is left.
 * - `plan(file)`: the download options read now and the one to take
 *   (chooseSlrDownload); an error when none fits. Uses up nothing.
 * - `download(file, tmp, {scriptMs, videoSeconds})`: ready, plan, a fresh
 *   link (this uses up one download), stream into `tmp`, then the size (the
 *   option's byte count) and the length (ffprobe through `videoSeconds`,
 *   durationMatches with the main script, else the scene's length) must
 *   agree. Returns the option taken.
 */
export function createSlr({
  fetch = globalThis.fetch, userData, intervalMs = SLR_INTERVAL_MS, now = () => Date.now(), sleep, log = () => {},
} = {}) {
  const get = pacedFetch({ fetch, intervalMs, now, sleep, label: 'sexlikereal' });
  let token = null;
  let tokenUntil = 0;

  async function accessToken() {
    if (token && now() < tokenUntil) return token;
    const file = path.join(userData, SLR_SESSION_FILE);
    let session;
    try { session = JSON.parse(fs.readFileSync(file, 'utf8')); } catch (err) {
      throw new SlrLoginError(err.code === 'ENOENT' ? `SexLikeReal: not logged in (${LOGIN_HINT})` : `SexLikeReal: ${file} is unreadable (${LOGIN_HINT})`);
    }
    if (!session?.refreshJwt) throw new SlrLoginError(`SexLikeReal: not logged in (${LOGIN_HINT})`);
    const resp = await get(`${SLR_SITE}/api/get-access-token`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Accept: 'application/json', 'User-Agent': SLR_UA, Cookie: `refresh_jwt=${session.refreshJwt}` },
      body: '{}',
      signal: AbortSignal.timeout(60_000),
    });
    if (resp.status === 401 || resp.status === 403) {
      await resp.body?.cancel?.().catch(() => {});
      throw new SlrLoginError(`SexLikeReal: the session has expired (${LOGIN_HINT})`);
    }
    if (!resp.ok) {
      await resp.body?.cancel?.().catch(() => {});
      throw new Error(`sexlikereal: get-access-token HTTP ${resp.status}`);
    }
    const body = await resp.json().catch(() => null);
    if (!body?.accessToken) throw new SlrLoginError(`SexLikeReal: the session has expired (${LOGIN_HINT})`);
    const rotated = rotatedRefreshJwt(resp.headers.getSetCookie?.() || [], now());
    if (rotated && rotated.value !== session.refreshJwt) {
      writeSession(file, { refreshJwt: rotated.value, expires: rotated.expires || session.expires || null, savedAt: new Date(now()).toISOString() });
      log('saved the renewed session');
    }
    token = body.accessToken;
    // Reused for the run until a minute before it expires (30 days today); 10 minutes when it doesn't say.
    tokenUntil = (tokenExpiry(token) ?? now() + 11 * 60_000) - 60_000;
    return token;
  }

  /** A JSON API answer; null for 404. A 401 with a token is an expired session. */
  async function api(pathname, { auth = false } = {}) {
    const headers = { ...API_HEADERS, ...(auth ? { Authorization: `Bearer ${await accessToken()}` } : {}) };
    const resp = await get(`${SLR_API}${pathname}`, { headers, signal: AbortSignal.timeout(60_000) });
    if (!resp.ok) {
      await resp.body?.cancel?.().catch(() => {});
      if (resp.status === 404) return null;
      if (auth && resp.status === 401) {
        token = null;
        throw new SlrLoginError(`SexLikeReal: the session has expired (${LOGIN_HINT})`);
      }
      throw Object.assign(new Error(`sexlikereal: HTTP ${resp.status} for ${pathname.split('?')[0]}`), { status: resp.status });
    }
    return resp.json();
  }

  const detail = async (ref) => (await api(`/v3/scenes/${encodeURIComponent(ref)}`))?.data || null;
  const search = async (query) => {
    const data = (await api(`/v3/search?query=${encodeURIComponent(query)}&tab=scenes`))?.data;
    return Array.isArray(data) ? data : [];
  };

  const host = {
    async filesFor(leads, { scriptLengths = [], title = '' } = {}) {
      const files = [];
      const missed = [];
      let down = false;
      const add = (scene) => {
        const file = slrFile(scene);
        if (!files.some((f) => f.slrSceneId === file.slrSceneId)) files.push(file);
      };
      for (const lead of leads) {
        try {
          if (lead.from === 'link') {
            const scene = await detail(lead.label || lead.id);
            if (scene?.id) add(scene); else missed.push({ lead, why: 'no such scene' });
            continue;
          }
          if (!scriptLengths.length) { missed.push({ lead, why: 'no script length to check a search hit against' }); continue; }
          let hit = null;
          for (const q of lead.queries) {
            hit = pickSearchHit(await search(q), { scriptLengths, title });
            if (hit) break;
          }
          const scene = hit && await detail(hit.label || hit.id);
          if (scene?.id) add(scene); else missed.push({ lead, why: `no scene found for ${lead.queries.map((q) => `"${q}"`).join(', ')} fits the script` });
        } catch (err) {
          down = true;
          missed.push({ lead, why: err.message });
        }
      }
      return { files, missed, down };
    },
    async quota() {
      const limit = (await api('/users/v1/user/overview', { auth: true }))?.data?.limits?.downloadVideo;
      const remaining = Number(limit?.remaining);
      if (!limit || !Number.isFinite(remaining)) throw new Error('sexlikereal: the user overview has no video download limit');
      return { remaining, total: Number(limit.total) || null, availableAt: limit.availableAt ?? null };
    },
    async ready() {
      const quota = await host.quota();
      if (quota.remaining <= 0) throw new SlrQuotaError(quota);
      return quota;
    },
    async plan(file) {
      const answer = await api(`/content/v1/videos/${file.slrSceneId}/download-options`, { auth: true });
      const options = slrDownloadOptions(answer?.data?.options);
      const pick = chooseSlrDownload(options, { vr: file.vr !== false, fps: file.fps });
      if (!pick) {
        const offered = options.map((o) => `${o.type} ${o.height || o.resolution}p`).join(', ') || 'none offered';
        throw new Error(`sexlikereal: no download within the ${file.vr === false ? 'flat' : 'VR'} caps for scene ${file.slrSceneId} (${offered})`);
      }
      return pick;
    },
    async download(file, tmp, { scriptMs = null, videoSeconds } = {}) {
      if (scriptMs && file.duration && !durationMatches(file.duration, scriptMs)) {
        throw new Error(`sexlikereal: video ${clock(file.duration * 1000)}, script ${clock(scriptMs)}; not this scene's video`);
      }
      const quota = await host.ready();
      const pick = await host.plan(file);
      const answer = await api(`/content/v1/videos/${file.slrSceneId}/download?type=${encodeURIComponent(pick.type)}`, { auth: true });
      const url = answer?.data?.url;
      if (!url) throw new Error(`sexlikereal: no download link for scene ${file.slrSceneId} (${pick.type})`);
      log(`${pick.type} ${pick.height}p ${pick.codec || ''}, ${pick.size ? gb(pick.size) : 'size unknown'}; ${quota.remaining - 1} of ${quota.total ?? '?'} downloads left after this one`);
      const resp = await get(url, { headers: { 'User-Agent': SLR_UA } });
      if (!resp.ok || !resp.body) {
        await resp.body?.cancel?.().catch(() => {});
        throw new Error(`sexlikereal: HTTP ${resp.status} for the ${pick.type} file of scene ${file.slrSceneId}`);
      }
      await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp));
      const want = Number(pick.size) || Number(resp.headers.get('content-length')) || 0;
      const size = fs.statSync(tmp).size;
      if (want && size !== want) throw new Error(`sexlikereal: size ${size} ≠ ${want} for scene ${file.slrSceneId}`);
      const got = videoSeconds(tmp);
      if (!got) throw new Error(`sexlikereal: scene ${file.slrSceneId} isn't a readable video (ffprobe)`);
      const fits = scriptMs ? durationMatches(got, scriptMs) : !file.duration || durationMatches(got, file.duration * 1000) || durationMatches(file.duration, got * 1000);
      if (!fits) throw new Error(`sexlikereal: downloaded video ${clock(got * 1000)}, ${scriptMs ? `script ${clock(scriptMs)}` : `scene ${clock(file.duration * 1000)}`}; not this scene's video`);
      return pick;
    },
  };
  return host;
}
