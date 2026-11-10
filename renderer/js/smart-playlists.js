// Smart playlists (user, 2026-11-10): app-made playlists that refill
// themselves (SMART_PLAYLISTS in content-preferences.js: "VR picks",
// "Quick ones", "Fresh downloads").
//
// Like the score lists (score-bands.js), each holds exactly its current
// members: the scanned, unwatched (unrated, ratings.js `isWatched`) videos
// that meet its rules, in its own order (best score or newest first). The
// desktop library rewrites them after each scan, whenever the scores change
// (tag scores, favourite artists, Recalculate, the backend's VR and axis
// decisions), whenever a rating changes (a rated video leaves at once) and
// when new durations are known. They're stored as ordinary playlists, so the
// phone, the headsets (`Playlist: <name>`) and the playlist views show them.
// They hold only unwatched videos, so watched-last doesn't apply: they keep
// their order (watched-lists.js).
//
// Made once, like the artist playlists: an existing playlist of the same
// name is taken over, the first one is made when it would hold a video, and
// one the user deletes isn't made again. That history is
// `library.smartPlaylists`: `{created: [key]}`. A smart playlist stays when
// it has nothing to show (playlist-cleanup.js leaves it), so the playlist
// list keeps its shape; videos taken out by hand come back on the next
// refresh while they still qualify.

import { SMART_PLAYLISTS } from './content-preferences.js';
import { isWatched } from './ratings.js';
import { titleOf } from './auto-collections.js';

export const SMART_PLAYLISTS_KEY = 'library.smartPlaylists';
export const DAY_MS = 24 * 60 * 60 * 1000;

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();
const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);

/** True when `name` is a smart playlist's name (any case or Unicode form). */
export function isSmartPlaylistName(name) {
  const key = fold(name);
  return !!key && SMART_PLAYLISTS.some((def) => fold(def.name) === key);
}

/**
 * Each smart playlist's members, in order.
 *
 * @param {object} o
 * @param {{path: string, dateAdded?: number}[]} o.videos the scanned videos (`dateAdded` = the scan's ms)
 * @param {Object<string, number>} [o.scores] `library.preferenceScores`
 * @param {object} [o.ratings] `library.ratings`
 * @param {Object<string, number>} [o.durations] `library.durationCache` (seconds)
 * @param {(path: string) => boolean} [o.vrOf] the backend's VR decision
 * @param {number} [o.now]
 * @param {typeof SMART_PLAYLISTS} [o.definitions]
 * @returns {Map<string, string[]>} key → video paths
 */
export function smartPlaylistMembers({
  videos = [], scores = {}, ratings = {}, durations = {}, vrOf = () => false, now = Date.now(), definitions = SMART_PLAYLISTS,
}) {
  const unwatched = videos.filter((v) => v?.path && !isWatched(ratings, v.path));
  const titles = new Map();
  const title = (p) => {
    if (!titles.has(p)) titles.set(p, titleOf(p));
    return titles.get(p);
  };
  const tie = (a, b) => title(a.path).localeCompare(title(b.path)) || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  const out = new Map();
  for (const def of definitions) {
    const members = unwatched.filter((v) => {
      if (def.vr && vrOf(v.path) !== true) return false;
      if (def.minScore !== undefined && !(num(scores[v.path]) >= def.minScore)) return false;
      if (def.maxSeconds !== undefined) {
        const d = num(Number(durations[v.path]));
        if (!(d > 0 && d < def.maxSeconds)) return false;
      }
      if (def.addedWithinDays !== undefined) {
        const added = num(v.dateAdded);
        if (!(added > 0 && now - added <= def.addedWithinDays * DAY_MS)) return false;
      }
      return true;
    });
    members.sort(def.order === 'added'
      ? (a, b) => (b.dateAdded - a.dateAdded) || tie(a, b)
      : (a, b) => ((num(scores[b.path]) ?? -Infinity) - (num(scores[a.path]) ?? -Infinity)) || tie(a, b));
    out.set(def.key, members.map((v) => v.path));
  }
  return out;
}

/**
 * What to create and rewrite.
 *
 * @param {object} o
 * @param {{id: string, name: string, videoPaths?: string[]}[]} o.playlists
 * @param {Map<string, string[]>} o.members from smartPlaylistMembers
 * @param {{created?: string[]}} [o.state] `library.smartPlaylists`
 * @param {typeof SMART_PLAYLISTS} [o.definitions]
 * @returns {{create: {key: string, name: string, videoPaths: string[]}[], update: {id: string, videoPaths: string[]}[],
 *   state: {created: string[]}}} `update` replaces a playlist's videos (its order included)
 */
export function planSmartPlaylists({ playlists = [], members, state = {}, definitions = SMART_PLAYLISTS }) {
  const byName = new Map(playlists.map((pl) => [fold(pl.name), pl]));
  const created = new Set(state.created || []);
  const create = [];
  const update = [];
  for (const def of definitions) {
    const want = members.get(def.key) || [];
    const existing = byName.get(fold(def.name));
    if (!existing) {
      if (created.has(def.key) || !want.length) continue;
      created.add(def.key);
      create.push({ key: def.key, name: def.name, videoPaths: want });
      continue;
    }
    created.add(def.key);
    const have = existing.videoPaths || [];
    if (have.length !== want.length || have.some((p, i) => p !== want[i])) update.push({ id: existing.id, videoPaths: want });
  }
  return { create, update, state: { ...state, created: [...created] } };
}

/**
 * Bring the smart playlists in line with the library now.
 *
 * @param {object} data DataService (get, set, getPlaylists, getRatings, addPlaylist, addVideosToPlaylist,
 *   removeVideoFromPlaylist, setPlaylistVideoPaths)
 * @param {{path: string, dateAdded?: number}[]} videos the scanned videos
 * @param {{vrOf: (path: string) => boolean, now?: number}} o
 * @returns {Promise<{created: string[], updated: number}>} names of new playlists, count of rewritten ones
 */
export async function applySmartPlaylists(data, videos, { vrOf, now = Date.now() }) {
  const members = smartPlaylistMembers({
    videos,
    scores: data.get('library.preferenceScores') || {},
    ratings: data.getRatings ? data.getRatings() : (data.get('library.ratings') || {}),
    durations: data.get('library.durationCache') || {},
    vrOf,
    now,
  });
  const before = data.get(SMART_PLAYLISTS_KEY) || {};
  const plan = planSmartPlaylists({ playlists: data.getPlaylists(), members, state: before });
  for (const c of plan.create) {
    const pl = await data.addPlaylist(c.name);
    data.addVideosToPlaylist(pl.id, c.videoPaths);
  }
  for (const u of plan.update) {
    const want = new Set(u.videoPaths);
    const have = data.getPlaylists().find((p) => p.id === u.id)?.videoPaths || [];
    for (const p of new Set(have)) if (!want.has(p)) data.removeVideoFromPlaylist(u.id, p);
    data.addVideosToPlaylist(u.id, u.videoPaths);
    const after = data.getPlaylists().find((p) => p.id === u.id)?.videoPaths || [];
    if (after.some((p, i) => p !== u.videoPaths[i])) data.setPlaylistVideoPaths(u.id, u.videoPaths);
  }
  if ((before.created || []).length !== plan.state.created.length) data.set(SMART_PLAYLISTS_KEY, plan.state);
  return { created: plan.create.map((c) => c.name), updated: plan.update.length };
}
