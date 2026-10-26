// Discovery scout: find free EroScripts posts worth adding to the library,
// plan which scripts and which video file to download, and queue the
// finished scenes into the app's "Discovery" playlist.
//
// Logic for `npm run eroscripts:scout` (scripts/eroscripts-scout.mjs), kept
// apart from the CLI so it can be tested without a forum, MEGA or pixeldrain.
//
// Rules
//   • Free Scripts only (the CLI lists that category).
//   • Never: loli, shota, furry / anthro / yiff, futa / futanari (tags, title
//     or first post). Scenes already in the library stay.
//   • Animated only: a tag like animation / 3d / hentai / vam / sfm / mmd and
//     no `real` tag. AI-generated video counts. A music post (PMV / HMV /
//     Cock Hero …) by a creator already in the library is accepted as well.
//   • Ranking: VR +3, music +2, creator already in the library +2,
//     creampie / riding +2 (+0.5 for both), multi-axis +1, likes up to +3.
//   • Skip posts the library already links and scenes whose video is
//     already in the library.
//   • Video variant: VR ≤ 8K and ≤ 60 fps, low bitrate when offered;
//     flat ≤ 4K. Previews and trailers never.
//   • Single-axis scenes get generated twist / roll / pitch companions after
//     download (scripts/upgrade-multi-axis.mjs); hand-made axes always win.

import path from 'node:path';

export const ANIMATION_TAGS = new Set(['animation', '3d', 'hentai', 'vam', 'sfm', 'mmd', 'cgi', 'cartoon', 'blender', 'animate', 'anime', 'koikatsu', 'honey-select']);
export const MUSIC_TAGS = new Set(['music', 'pmv', 'hmv', 'beat-based', 'music-based', 'music-video', 'cock-hero', 'fap-hero', 'compilation']);
export const PREFERRED_TAGS = new Set(['creampie', 'riding']);
// Matched against tags, titles and first posts as whole words, so hyphenated
// tags (`futa-on-male`) match and `Futaba` doesn't.
const EXCLUDED_TEXT = /(?<![\p{L}\p{N}])(loli|lolicon|shota|shotacon|furry|furries|anthro|yiff|futa|futas|futanari|dickgirls?)(?![\p{L}\p{N}])/iu;

export const DISCOVERY_NAME = 'Discovery';
export const FEED_TAGS = ['multi-axis', 'animation', 'hentai', '3d', 'vam', 'sfm', 'mmd'];

const VIDEO_EXT_RE = /\.(mp4|m4v|mkv|webm|mov)$/i;
const SCRIPT_AXES = new Set(['surge', 'sway', 'twist', 'roll', 'pitch', 'vib', 'lube', 'pump', 'suction', 'valve']);
// TCode channel suffixes some posts use instead of names (`Scene.R0.funscript`).
const TCODE_AXES = { L0: '', L1: 'surge', L2: 'sway', R0: 'twist', R1: 'roll', R2: 'pitch' };
// Words that only describe the encode, never the scene.
const QUALITY_WORD = /^(\d{3,4}p|\d+k|\d+k\d+fps|\d+fps|vr|sbs|low|high|bitrate|source|uncompressed|hq|lq|h264|h265|hevc|x264|x265|hd|uhd)$/;

// ── Names ───────────────────────────────────────────────────────────────

export const isVideoName = (name) => VIDEO_EXT_RE.test(String(name || ''));

/** Lower-case letters and digits of any script (CJK included), single-spaced. */
export function normName(name) {
  return String(name || '').toLowerCase().replace(VIDEO_EXT_RE, '').replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
}

/** normName without encode words (`1080p`, `8K60FPS`, `VR`, `low bitrate` …). */
export function coreName(name) {
  return normName(name).split(' ').filter((w) => w && !QUALITY_WORD.test(w)).join(' ');
}

/**
 * `Scene.twist.funscript` → `{ base: 'Scene', axis: 'twist' }`; the main
 * script has axis ''. TCode suffixes map to names (R0 → twist, L0 → main).
 * A video extension left inside the base (`Scene.mp4.funscript`) is dropped.
 * null for anything that isn't a funscript.
 */
export function scriptParts(name) {
  const m = String(name || '').match(/^(.*?)(?:\.([A-Za-z0-9]+))?\.funscript$/i);
  if (!m) return null;
  const [, stem, suffix] = m;
  const clean = (s) => s.replace(VIDEO_EXT_RE, '');
  if (suffix && SCRIPT_AXES.has(suffix.toLowerCase())) return { base: clean(stem), axis: suffix.toLowerCase() };
  if (suffix && suffix.toUpperCase() in TCODE_AXES) return { base: clean(stem), axis: TCODE_AXES[suffix.toUpperCase()] };
  return { base: clean(suffix ? `${stem}.${suffix}` : stem), axis: '' };
}

const digitWords = (s) => s.split(' ').filter((w) => /^\d+$/.test(w));

/**
 * How well a video file name matches a script base, 0..1 (0 = not this
 * video). Same core name is 1; numbers must agree (Part 1 is not Part 2,
 * 7 is not 17); then containment of a long enough name, then ≥ 75 % of the
 * base's words.
 */
export function matchScore(base, videoName) {
  const b = coreName(base);
  const v = coreName(videoName);
  if (!b || !v) return 0;
  if (b === v) return 1;
  const vWords = new Set(v.split(' '));
  if (digitWords(b).some((n) => !vWords.has(n))) return 0;
  if (Math.min(b.length, v.length) >= 6 && (v.includes(b) || b.includes(v))) return 0.95;
  const words = b.split(' ');
  const hit = words.filter((w) => vWords.has(w)).length;
  return hit >= 2 && hit / words.length >= 0.75 ? 0.9 * (hit / words.length) : 0;
}

/** True when two core names are the same video (library duplicate check). */
export function sameVideo(a, b) {
  if (!a || !b) return false;
  if (a === b) return true;
  const bWords = new Set(b.split(' '));
  const aWords = a.split(' ');
  if (digitWords(a).some((n) => !bWords.has(n))) return false;
  if (Math.min(a.length, b.length) >= 8 && (a.includes(b) || b.includes(a))) return true;
  return aWords.length >= 3 && aWords.filter((w) => bWords.has(w)).length / aWords.length >= 0.75;
}

// ── Screening ───────────────────────────────────────────────────────────

/** True when text names excluded content (whole words, any script). */
export function hasExcludedText(text) {
  return EXCLUDED_TEXT.test(String(text || ''));
}

/**
 * First pass on a listed topic, before any request for its files.
 *
 * @param {{id: number, title: string, tags: string[]}} topic
 * @param {object} ctx
 * @param {Set<number>} ctx.knownTopics posts the library already links
 * @param {string[]} ctx.creators creator names in the library (lower case)
 * @param {string[]} ctx.sceneNames normName of every library scene folder
 * @returns {{ok: false, reason: string} | {ok: true, music: boolean, vr: boolean, artist: string|null}}
 */
export function screenTopic(topic, { knownTopics = new Set(), creators = [], sceneNames = [] } = {}) {
  const tags = topic.tags || [];
  if (knownTopics.has(topic.id)) return { ok: false, reason: 'known' };
  if (tags.some(hasExcludedText) || hasExcludedText(topic.title)) return { ok: false, reason: 'excluded' };
  const title = normName(topic.title);
  const artist = creators.find((c) => c.length > 3 && title.includes(normName(c))) || null;
  const animated = tags.some((t) => ANIMATION_TAGS.has(t)) && !tags.includes('real');
  const music = tags.some((t) => MUSIC_TAGS.has(t));
  if (!animated && !(music && artist)) return { ok: false, reason: 'not-animated' };
  if (sceneNames.some((s) => s.length > 8 && (title.includes(s) || s.includes(title)))) return { ok: false, reason: 'in-library' };
  return { ok: true, music, vr: tags.includes('vr'), artist };
}

/** Ranking score; higher downloads first. */
export function scoreCandidate({ vr = false, music = false, artistInLibrary = false, preferred = [], multiAxis = false, likes = 0 }) {
  return (vr ? 3 : 0)
    + (music ? 2 : 0)
    + (artistInLibrary ? 2 : 0)
    + (preferred.length ? 2 + 0.5 * (preferred.length - 1) : 0)
    + (multiAxis ? 1 : 0)
    + Math.min(3, likes / 20);
}

// ── Video variant ───────────────────────────────────────────────────────

function resolutionOf(n) {
  if (/16k/.test(n)) return 16;
  if (/12k/.test(n)) return 12;
  if (/8k|4320/.test(n)) return 8;
  if (/6k|5k|2880/.test(n)) return 6;
  if (/4k|2160/.test(n)) return 4;
  if (/1440|2k/.test(n)) return 2.5;
  if (/1080/.test(n)) return 2;
  if (/720/.test(n)) return 1;
  return 3;
}

/** Preference among a scene's video files; the highest wins. */
export function videoPreference(name, size, vr) {
  const n = String(name).toLowerCase();
  const res = resolutionOf(n);
  const fps = Number((n.match(/(\d{2,3})\s*fps/) || [])[1] || 60);
  const isVr = /(^|[^a-z])(vr|sbs|180|fisheye)([^a-z]|$)/.test(n);
  let score = 0;
  if (vr) score += (isVr ? 100 : 0) + (res <= 8 ? res * 5 : -50);
  else score += (isVr ? -30 : 0) + (res <= 4 ? res * 5 : -20);
  if (fps > 60) score -= 40;
  if (/low[- _]?bitrate|lowbit/.test(n)) score += 8;
  if (/high[- _]?bitrate/.test(n)) score -= 8;
  if (/preview|trailer|sample/.test(n)) score -= 200;
  return score + Math.min((size || 0) / 1e10, 1);
}

// ── Scene plan ──────────────────────────────────────────────────────────

/**
 * Pair a post's script sets with its video files.
 *
 * Scripts are grouped by base (bracket / punctuation variants of one name
 * merge; per axis the first file wins, so list forum uploads first). Each
 * group with a main script takes its best-matching video, best matches
 * first, and each video (all its encodes) pairs with one group only. A post
 * with a single script set whose videos don't name it takes its best video.
 * Handy-only scripts are ignored.
 *
 * @param {object} o
 * @param {{name: string}[]} o.scripts
 * @param {{name: string, size?: number}[]} o.videos
 * @param {boolean} [o.vr]
 * @param {string[]} [o.libraryVideos] coreName of every library video
 * @returns {{scenes: {base, axes: string[], scripts: object[], video: object, match: number}[], inLibrary: number}}
 */
export function planScenes({ scripts, videos, vr = false, libraryVideos = [] }) {
  const groups = new Map();
  for (const f of scripts) {
    const p = scriptParts(f.name);
    if (!p || /(^|[^a-z])handy([^a-z]|$)/i.test(p.base)) continue;
    const key = coreName(p.base) || p.base;
    const g = groups.get(key) || { base: p.base, files: [] };
    if (!g.files.some((x) => x.axis === p.axis)) g.files.push({ ...f, axis: p.axis });
    groups.set(key, g);
  }
  const withMain = [...groups.values()].filter((g) => g.files.some((f) => f.axis === ''));

  const options = new Map();
  for (const g of withMain) {
    for (const v of videos) {
      const m = matchScore(g.base, v.name);
      if (m > 0) options.set(g, [...(options.get(g) || []), { v, m }]);
    }
  }
  if (withMain.length === 1 && options.size === 0 && videos.length) {
    options.set(withMain[0], videos.map((v) => ({ v, m: 0.5 })));
  }

  const best = (g) => Math.max(...options.get(g).map((o) => o.m));
  const used = new Set();
  const scenes = [];
  let inLibrary = 0;
  for (const g of [...options.keys()].sort((a, b) => best(b) - best(a))) {
    const open = options.get(g).filter((o) => !used.has(coreName(o.v.name)));
    if (!open.length) continue;
    const top = Math.max(...open.map((o) => o.m));
    const video = open.filter((o) => o.m === top).map((o) => o.v)
      .sort((a, b) => videoPreference(b.name, b.size, vr) - videoPreference(a.name, a.size, vr))[0];
    used.add(coreName(video.name));
    if (libraryVideos.some((l) => sameVideo(coreName(g.base), l) || sameVideo(coreName(video.name), l))) { inLibrary++; continue; }
    const axes = [...new Set(g.files.map((f) => f.axis).filter(Boolean))].sort();
    scenes.push({ base: g.base, axes, scripts: g.files, video, match: top });
  }
  return { scenes, inLibrary };
}

// ── Folders ─────────────────────────────────────────────────────────────

const safeName = (s) => String(s).replace(/[/\\:*?"<>|]/g, '').replace(/\s+/g, ' ').trim();
const NOT_A_CREATOR = /multi|axis|(^|[^a-z])vr([^a-z]|$)|initiative|free|sr6|osr|handy|commission|request|vam|mmd|sfm/i;

/** Creator folder: the first leading `[Name]` / `(Name)` of the title that names someone, else the poster. */
export function creatorFolder(title, author) {
  const lead = (String(title).match(/^(\s*[[(（【][^\])）】]+[\])）】])+/) || [''])[0];
  const named = [...lead.matchAll(/[[(（【]([^\])）】]+)[\])）】]/g)].map((m) => m[1].trim()).find((x) => !NOT_A_CREATOR.test(x));
  return safeName(named || author) || 'unknown';
}

/**
 * Scene folder and file stem. A base that says nothing on its own (`7`,
 * `A`, `01`) is prefixed with the post title so it stays findable.
 */
export function sceneStem(base, title) {
  const stem = safeName(base);
  const letters = (coreName(stem).match(/\p{L}/gu) || []).length;
  if (letters >= 3) return stem;
  return safeName(`${title.replace(/[[(（【][^\])）】]*[\])）】]/g, ' ')} - ${stem}`).slice(0, 150);
}

/** File name for a scene's script in the scan's spelling. */
export function scriptFileName(stem, axis) {
  return `${stem}${axis ? `.${axis}` : ''}.funscript`;
}

// ── Queue & playlist ────────────────────────────────────────────────────

/** Download order: multi-axis posts first, then single-axis; each by score. */
export function downloadQueue(candidates) {
  const ok = candidates.filter((c) => c.scenes.length);
  const byScore = (a, b) => b.score - a.score;
  return [...ok.filter((c) => c.axisType === 'multi').sort(byScore), ...ok.filter((c) => c.axisType !== 'multi').sort(byScore)];
}

/**
 * Add videos to the Discovery playlist and make sure the incoming folder is a
 * library source, in a parsed config.json (mutated). Returns what changed.
 */
export function addToDiscovery(config, videoPaths, { incomingDir, newId, now = Date.now() }) {
  const changes = { sourceAdded: false, playlistCreated: false, added: [] };
  config.settings ||= {};
  config.settings.library ||= {};
  const lib = config.settings.library;
  lib.sources ||= [];
  if (!lib.sources.some((s) => path.resolve(s.path) === path.resolve(incomingDir))) {
    lib.sources.push({ id: newId(), name: DISCOVERY_NAME, path: incomingDir, enabled: true });
    changes.sourceAdded = true;
  }
  config.playlists ||= [];
  let pl = config.playlists.find((p) => String(p.name).trim().toLowerCase() === DISCOVERY_NAME.toLowerCase());
  if (!pl) {
    pl = { id: newId(), name: DISCOVERY_NAME, createdAt: now, videoPaths: [] };
    config.playlists.push(pl);
    changes.playlistCreated = true;
  }
  pl.videoPaths ||= [];
  for (const p of videoPaths) if (!pl.videoPaths.includes(p)) { pl.videoPaths.push(p); changes.added.push(p); }
  return changes;
}

/** Key of one fetched scene in the `eroscripts.json` sidecars: post id + chosen video file. */
export function fetchedKey(topicId, videoName) {
  return `${topicId}|${videoName}`;
}
