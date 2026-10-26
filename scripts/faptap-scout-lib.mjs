// FapTap Discovery scout: find faptap.net videos worth adding to the
// library, pick the stream to download, and turn FapTap's script into a
// funscript.
//
// Logic for `npm run faptap:scout` (scripts/faptap-scout.mjs), kept apart
// from the CLI so it can be tested without faptap.net.
//
// faptap.net has an open JSON API (no login): `/api/videos` lists videos
// newest first with their tags and description, `/api/videos/<id>` adds the
// script, `/api/assets/scripts/<id>.csv` is the script as `ms,pos` lines, and
// `/api/videos/<id>/bunny-sources` lists HLS streams per quality. Every
// FapTap script is single-axis (a script has one url, no axes), so every
// scene gets generated twist / roll / pitch companions after download.
//
// Rules: those of the EroScripts scout (scripts/eroscripts-scout-lib.mjs)
// except the multi-axis one.
//   • Never loli, shota, furry / anthro / yiff, futa / futanari (tags, title
//     or description), nor FapTap's furry tags that don't say so (scalie,
//     knot, Krystal, Renamon …).
//   • Animated only: an animation tag (animated, hentai, cgi, sfm, mmd …),
//     no live-action tag (amateur, homemade, pornstar, onlyfans …), and no
//     FapTap model / pornstar named in the title or listed as the video's
//     performer (FapTap tags are free text; live videos get animation tags
//     too). AI video counts. A music video (PMV / HMV / Cock Hero …) by a
//     creator already in the library is taken too; a library folder named
//     after a genre (`Cock Hero`, `PMV`) is no creator.
//   • Sex only (as on EroScripts), at least a minute long.
//   • Skip videos the library or the EroScripts scout list already has.
//   • Stream: the best Bunny HLS stream up to 4K (8K for VR); none below
//     720p (1080p for VR).

import {
  ANIMATION_TAGS, MIN_VIDEO_MS, MUSIC_TAGS, PREFERRED_TAGS, hasExcludedText, isCreatorLabel, isSexual, normName, sameVideo, scoreCandidate,
} from './eroscripts-scout-lib.mjs';

export const FAPTAP_ORIGIN = 'https://faptap.net';
export const SIDECAR = 'faptap.json';
/** Bump when the screening rules change, so every video is judged again. */
export const FAPTAP_RULES_VERSION = 1;
/** The one feed: every video, newest first. */
export const FEED = 'all';

// FapTap's own spellings of animation tags, on top of the EroScripts ones.
const FAPTAP_ANIMATION_TAGS = new Set([
  'animated', 'animated-2d', 'animated-3d', 'animated-cartoon', 'animated-porn', 'animation-sex', 'animation-w-sound',
  'hentai-anime', 'hentai-joi', 'ai-video', 'iwara',
]);
// Tags that say the video is live action.
const LIVE_TAGS = new Set([
  'real', 'real-sex', 'real-couple-homemade', 'real-orgasm', 'reality', 'homemade', 'pornstar', 'onlyfans', 'brazzers',
  'blacked', 'chaturbate', 'webcam', 'verified-amateurs', 'jav', 'celebrity', 'tiktok', 'cosplay', 'anime-cosplay', 'kbj',
  'egirl', 'sislovesme', 'dadcrush',
]);
// Furry content under tags that don't say furry / anthro / yiff.
const FURRY_TAGS = new Set([
  'scalie', 'knot', 'knotted', 'horse', 'dog', 'animal', 'my-little-pony', 'animal-crossing', 'krystal-star-fox',
  'renamon-digimon', 'five-nights-at-freddy-s', 'roxanne-wolf-five-nights-at-freddy-s',
]);
// Bunny stream heights: the cap keeps files playable, the floor drops
// preview-only uploads.
const MAX_HEIGHT = { flat: 2160, vr: 4320 };
const MIN_HEIGHT = { flat: 720, vr: 1080 };
// FapTap projection → the file-name suffix HereSphere / DeoVR (and the
// backend's detect_vr_format) read. Flat and flat 3D get none.
const PROJECTION_SUFFIX = {
  STEREO_180_LR: '_180_LR',
  STEREO_180_LR_SPHERICAL: '_180_LR',
  STEREO_180_TB: '_180_TB',
  STEREO_180_TB_SPHERICAL: '_180_TB',
  STEREO_360_LR: '_360_LR',
  STEREO_360_TB: '_360_TB',
  STEREO_FISHEYE_200_LR: '_MKX200',
  MONO_360: '_MONO360',
};

/** A FapTap tag name as an EroScripts-style slug: `Cum in Mouth` → `cum-in-mouth`. */
export function tagSlug(name) {
  return String(name || '').trim().toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '');
}

/** A listed video's tags as slugs, without duplicates. */
export function videoTags(video) {
  return [...new Set((video?.tags || []).map((t) => tagSlug(t?.name)).filter(Boolean))];
}

const htmlText = (html) => String(html || '').replace(/<[^>]+>/g, ' ').replace(/&nbsp;/g, ' ');
const isLive = (t) => LIVE_TAGS.has(t) || t === 'amateur' || t.startsWith('amateur-');

/** HereSphere / DeoVR suffix for a FapTap projection; '' for flat video. */
export function vrSuffix(projection) {
  return PROJECTION_SUFFIX[projection] || '';
}

/**
 * Judge a listed video before any further request.
 *
 * @param {{id: string, name: string, description?: string, duration?: number, tags?: {name: string}[],
 *   vr?: boolean, projection?: string, user?: {username?: string}}} video
 * @param {object} ctx
 * @param {string[]} ctx.creators creator names in the library (lower case)
 * @param {string[]} ctx.knownTitles normName of every library scene folder and video, and of the EroScripts scout's waiting posts
 * @param {string[]} [ctx.livePerformers] normName of FapTap's models and pornstars (see livePerformerNames)
 * @returns {{ok: false, reason: string} | {ok: true, tags: string[], vr: boolean, music: boolean, artist: string|null, preferred: string[]}}
 */
export function screenVideo(video, { creators = [], knownTitles = [], livePerformers = [] } = {}) {
  const tags = videoTags(video);
  const title = String(video.name || '');
  const text = htmlText(video.description);
  if (tags.some((t) => hasExcludedText(t) || FURRY_TAGS.has(t)) || hasExcludedText(title) || hasExcludedText(text)) {
    return { ok: false, reason: 'excluded' };
  }
  const name = normName(title);
  const words = ` ${name} `;
  const uploader = String(video.user?.username || '').toLowerCase();
  const artists = creators.filter((c) => c.length > 3 && !isCreatorLabel(c));
  const artist = artists.find((c) => words.includes(` ${normName(c)} `)) || (artists.includes(uploader) ? uploader : null);
  const animated = tags.some((t) => ANIMATION_TAGS.has(t) || FAPTAP_ANIMATION_TAGS.has(t)) && !tags.some(isLive)
    && !livePerformers.some((p) => words.includes(` ${p} `));
  const music = tags.some((t) => MUSIC_TAGS.has(t));
  if (!animated && !(music && artist)) return { ok: false, reason: 'not-animated' };
  if (!isSexual({ tags, title, text })) return { ok: false, reason: 'not-sexual' };
  if (!(Number(video.duration) * 1000 >= MIN_VIDEO_MS)) return { ok: false, reason: 'short' };
  // Both ways: a five-word title sharing four generic words isn't the same video.
  if (knownTitles.some((k) => k.length > 8 && sameVideo(name, k) && sameVideo(k, name))) return { ok: false, reason: 'in-library' };
  const vr = !!video.vr || tags.includes('vr') || tags.includes('virtual-reality');
  return { ok: true, tags, vr, music, artist, preferred: tags.filter((t) => PREFERRED_TAGS.has(t)) };
}

/** Performer categories FapTap gives real people. */
const LIVE_CATEGORIES = new Set(['model', 'pornstar']);

/**
 * Names worth matching in titles: FapTap's models and pornstars with at
 * least two words (`Molly Little`), as normName.
 *
 * @param {{name: string, category?: string}[]} performers `/api/performers`
 */
export function livePerformerNames(performers) {
  return [...new Set((performers || [])
    .filter((p) => LIVE_CATEGORIES.has(p?.category))
    .map((p) => normName(p.name))
    .filter((n) => n.split(' ').length >= 2 && n.length >= 6))];
}

/** True when a video's details list a model or pornstar as its performer. */
export function hasLivePerformer(detail) {
  return (detail?.performers || []).some((p) => LIVE_CATEGORIES.has(p?.category));
}

/**
 * The stream to download: the highest HLS (or MP4) quality within the cap,
 * or null when the best one is below the floor.
 *
 * @param {{url: string, quality: string|number, format?: string}[]} sources `/bunny-sources`
 * @param {boolean} vr
 */
export function pickSource(sources, vr) {
  const kind = vr ? 'vr' : 'flat';
  const usable = (sources || [])
    .map((s) => ({ ...s, height: parseInt(s?.quality, 10) }))
    .filter((s) => s.url && ['hls', 'mp4'].includes(String(s.format || 'hls').toLowerCase()) && s.height > 0 && s.height <= MAX_HEIGHT[kind])
    .sort((a, b) => b.height - a.height);
  return usable[0] && usable[0].height >= MIN_HEIGHT[kind] ? usable[0] : null;
}

/**
 * FapTap's `ms,pos` script as a funscript: actions in time order, one per
 * millisecond (the last wins), positions clamped to 0..100. Lines that
 * aren't two numbers are skipped.
 */
export function csvToFunscript(text) {
  const byTime = new Map();
  for (const line of String(text || '').split(/\r?\n/)) {
    const m = line.trim().match(/^(\d+(?:\.\d+)?)\s*[,;]\s*(-?\d+(?:\.\d+)?)$/);
    if (!m) continue;
    byTime.set(Math.round(Number(m[1])), Math.min(100, Math.max(0, Math.round(Number(m[2])))));
  }
  const actions = [...byTime].sort((a, b) => a[0] - b[0]).map(([at, pos]) => ({ at, pos }));
  return { version: '1.0', inverted: false, range: 100, actions };
}

/** A screened video as a saved candidate. */
export function toCandidate(video, screen) {
  const likes = Number(video.rating?.likes) || 0;
  return {
    id: String(video.id),
    title: String(video.name || '').trim(),
    url: `${FAPTAP_ORIGIN}/v/${video.id}`,
    author: video.user?.username || 'unknown',
    tags: screen.tags,
    likes,
    duration: Number(video.duration) || 0,
    vr: screen.vr,
    projection: video.projection || 'MONO_FLAT',
    music: screen.music,
    preferred: screen.preferred,
    artistInLibrary: screen.artist,
    score: +scoreCandidate({ vr: screen.vr, music: screen.music, artistInLibrary: !!screen.artist, preferred: screen.preferred, likes }).toFixed(2),
  };
}

/** Key of a fetched or rejected FapTap scene; the same as electron/rejected-videos.js `faptapRejectionKey`. */
export function fetchedKey(id) {
  return `faptap|${id}`;
}

/**
 * The saved list after a scan: candidates found now plus earlier ones not
 * yet fetched or rejected, best score first.
 *
 * @param {object[]} previous
 * @param {object[]} fresh
 * @param {(id: string) => boolean} isDone fetched, or turned down in the app
 */
export function mergeCandidates(previous, fresh, isDone) {
  const byId = new Map();
  for (const c of [...(previous || []), ...(fresh || [])]) byId.set(String(c.id), c);
  return [...byId.values()].filter((c) => !isDone(String(c.id))).sort((a, b) => b.score - a.score);
}
