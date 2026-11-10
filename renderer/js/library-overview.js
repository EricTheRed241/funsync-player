// Library overview: what the desktop's Overview view shows, worked out from
// the scanned library and the settings. Capacity (videos against
// LIBRARY_MAX_VIDEOS, the cap the scouts' `--fetch` stops at, and the size
// on disk), ratings and favourites, tag scores and favourite artists. The
// same data the web remote's Tags and Artists views edit; the desktop edits
// it through library edits (library-edits.js) like the phone does.
//
// Only videos the library scanned count: ratings, tags and scores of deleted
// or moved-away videos stay in config.json but aren't the library's any more.
// Pure and renderer-safe: no DOM, no Node imports.

import { DELETE_AT_OR_BELOW, LIBRARY_MAX_VIDEOS, TOP_RATED_MIN } from './content-preferences.js';
import { getRatingEntry, RATING_MAX, RATING_STEP } from './ratings.js';
import { effectiveTagScores, normalizeTagScore, tagKey, TAG_SCORE_STEP } from './preference-score.js';
import { artistRegistration, favouritesOf } from './favourite-artists.js';
import { SCRIPT_TAGS_KEY } from './script-tags.js';

export const CAPACITY_HIGH = 0.9;

// Post tags that aren't about the video, left out of the tag list as the web
// remote's Tags view leaves them out (NOISE_TAGS and `len-*` in
// backend/services/video_tags.py): devices, VR (decided from the video),
// forum bookkeeping. One the user gave a score still shows.
const NOISE_TAGS = new Set([
  'the-handy', 'the-handy-2-pro', 'osr2', 'sr6', 'funsr-pro', 'syncbot', 'multi-axis', 'twist',
  'vr', 'non-vr',
  'no-free-links', 'straight',
]);
const isNoiseTag = (key) => NOISE_TAGS.has(key) || key.startsWith('len-');

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();
const bytesOf = (v) => (Number.isFinite(v?.size) && v.size > 0 ? v.size : 0);

/**
 * The library against its cap: `count` of `cap` (`percent`, rounded down so
 * 499 of 500 doesn't read 100 %), `free` places left, the size on disk, and
 * the same per source (most videos first). `level` is 'full' at the cap,
 * 'high' from CAPACITY_HIGH of it, else 'ok'.
 *
 * @param {{path: string, size?: number, sourceName?: string}[]} videos scanned videos
 * @param {{cap?: number}} [opts]
 */
export function capacitySummary(videos, { cap = LIBRARY_MAX_VIDEOS } = {}) {
  const list = Array.isArray(videos) ? videos : [];
  const sources = new Map();
  let bytes = 0;
  for (const v of list) {
    const size = bytesOf(v);
    bytes += size;
    const name = v?.sourceName || '';
    const s = sources.get(name) || { name, count: 0, bytes: 0 };
    s.count += 1;
    s.bytes += size;
    sources.set(name, s);
  }
  const count = list.length;
  const ratio = cap > 0 ? count / cap : 1;
  return {
    count,
    cap,
    free: Math.max(0, cap - count),
    percent: Math.floor(ratio * 100),
    level: count >= cap ? 'full' : ratio >= CAPACITY_HIGH ? 'high' : 'ok',
    bytes,
    sources: [...sources.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name)),
  };
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** A size in decimal units as Finder shows it: `542.6 GB`, `1.2 TB`, `0 B`. */
export function formatBytes(bytes, locale) {
  let n = Number(bytes) > 0 ? Number(bytes) : 0;
  let unit = 0;
  while (n >= 1000 && unit < BYTE_UNITS.length - 1) { n /= 1000; unit += 1; }
  const digits = unit === 0 || n >= 100 ? 0 : 1;
  return `${n.toLocaleString(locale, { minimumFractionDigits: digits, maximumFractionDigits: digits })} ${BYTE_UNITS[unit]}`;
}

/**
 * Ratings and favourites of the library's videos.
 *
 * `stars`: how many videos have each rating, RATING_MAX down to RATING_STEP
 * (every step listed, zeros included). `keep` = rated TOP_RATED_MIN or more
 * (the Top rated filter), `toDelete` = rated DELETE_AT_OR_BELOW or less and not a
 * favourite (the triage deletes those; normally 0). `entries`: every rated or
 * favourite video, favourites first, then highest rating, then latest change.
 *
 * @param {Object<string, object>} ratings `library.ratings`
 * @param {string[]} videoPaths the library's videos
 */
export function ratingSummary(ratings, videoPaths) {
  const paths = Array.isArray(videoPaths) ? videoPaths : [];
  const counts = new Map();
  const entries = [];
  let favourites = 0;
  let keep = 0;
  let toDelete = 0;
  for (const path of paths) {
    const entry = getRatingEntry(ratings || {}, path);
    if (!entry) continue;
    const rating = entry.rating || 0;
    const favorite = entry.favorite === true;
    if (!rating && !favorite) continue;
    entries.push({ path, rating, favorite, updatedAt: Number(entry.updatedAt) || 0 });
    if (favorite) favourites += 1;
    if (!rating) continue;
    counts.set(rating, (counts.get(rating) || 0) + 1);
    if (rating >= TOP_RATED_MIN) keep += 1;
    else if (rating <= DELETE_AT_OR_BELOW && !favorite) toDelete += 1;
  }
  const stars = [];
  for (let r = RATING_MAX; r >= RATING_STEP; r -= RATING_STEP) stars.push({ rating: r, count: counts.get(r) || 0 });
  const rated = stars.reduce((n, s) => n + s.count, 0);
  entries.sort((a, b) => (b.favorite - a.favorite) || (b.rating - a.rating) || (b.updatedAt - a.updatedAt) || a.path.localeCompare(b.path));
  return { total: paths.length, rated, unrated: paths.length - rated, favourites, keep, toDelete, stars, entries };
}

/**
 * Every tag of the library's videos, plus every tag with a score (a scored
 * tag no video has yet still steers the scouts), each with:
 * `videos` (library videos tagged with it, once per video, without case),
 * `score` (its effective score, 0 when none), `group` (its TAG_SCORE_GROUPS
 * group name, or null) and `changed` (the user set it; `library.tagScores`).
 * Device, length, VR and bookkeeping tags are left out unless scored. Most
 * used first, then by name.
 *
 * @param {object} data DataService (get)
 * @param {string[]} videoPaths the library's videos
 * @param {Object<string, string>} groups tag → group name (tagScoreInfo().groups)
 */
export function tagSummary(data, videoPaths, groups = {}) {
  const scriptTags = data.get(SCRIPT_TAGS_KEY) || {};
  const changes = data.get('library.tagScores') || {};
  const scores = effectiveTagScores(changes);
  const counts = new Map();
  for (const path of Array.isArray(videoPaths) ? videoPaths : []) {
    const tags = scriptTags[path]?.tags;
    if (!Array.isArray(tags)) continue;
    for (const key of new Set(tags.map(tagKey))) {
      if (key) counts.set(key, (counts.get(key) || 0) + 1);
    }
  }
  const changed = new Set(Object.keys(changes).map(tagKey).filter(Boolean));
  for (const key of [...Object.keys(scores), ...changed]) if (!counts.has(key)) counts.set(key, 0);
  return [...counts]
    .filter(([tag]) => !isNoiseTag(tag) || Object.hasOwn(scores, tag) || changed.has(tag))
    .map(([tag, videos]) => ({
      tag,
      videos,
      score: scores[tag] ?? 0,
      group: groups[tag] || null,
      changed: changed.has(tag),
    }))
    .sort((a, b) => b.videos - a.videos || a.tag.localeCompare(b.tag));
}

/**
 * Every artist folder of the library with its video count (the artist-
 * playlist rule, named by its most common spelling), plus favourites with no
 * videos. `favourite` follows the effective favourites (FAVOURITES with the
 * user's changes). Favourites first, then most videos, then by name.
 *
 * @param {object} data DataService (get)
 * @param {string[]} videoPaths the library's videos
 */
export function artistSummary(data, videoPaths) {
  const favourites = new Map(favouritesOf(data).map((name) => [fold(name), name]));
  const rows = (artistRegistration(data, Array.isArray(videoPaths) ? videoPaths : []).artists || [])
    .map((a) => ({ name: a.name, videos: a.videoPaths.length, favourite: favourites.has(fold(a.name)) }));
  const listed = new Set(rows.map((r) => fold(r.name)));
  for (const [key, name] of favourites) if (!listed.has(key)) rows.push({ name, videos: 0, favourite: true });
  return rows.sort((a, b) => (b.favourite - a.favourite) || (b.videos - a.videos)
    || a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
}

/** `score` one step up (`direction` 1) or down (-1), on the tag-score grid and range. */
export function stepTagScore(score, direction) {
  return normalizeTagScore((Number(score) || 0) + Math.sign(direction) * TAG_SCORE_STEP);
}

/** A score as shown: `0`, `+1.5`, `−2` (the web remote's spelling). */
export function formatTagScore(score) {
  const n = Number(score) || 0;
  if (n === 0) return '0';
  return `${n > 0 ? '+' : '−'}${Math.abs(n)}`;
}
