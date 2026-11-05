// The shared play queue: one queue owned by the desktop, shown and edited on
// the desktop (queue panel), the web remote and the headset.
//
//   manual    paths the user queued (`player.userQueue`), played first, unlimited
//   auto      computed here, appended after manual so the queue holds
//             QUEUE_AUTOFILL.size videos; empty when auto-fill is off; never
//             stored (recomputed deterministically)
//   played    `player.queuePlayed` = {path: epochMs}: when a video was played
//             or skipped, so auto-fill moves past it for replayAfterDays
//   autoFill  `player.queueAutoFill`
//
// Pure, no DOM: app.js holds the state and persists it; library-edits.js
// routes the phone's and the headset's queue edits to it.

import { AGE_SCREEN, DELETE_AT_OR_BELOW, QUEUE_AUTOFILL } from './content-preferences.js';
import { isFavorite, ratingOf } from './ratings.js';

export const DAY_MS = 86_400_000;
/** Most entries the played map keeps (newest win). */
export const PLAYED_CAP = 1000;
/** How long a played / skipped stamp holds a video back. */
export const REPLAY_AFTER_MS = QUEUE_AUTOFILL.replayAfterDays * DAY_MS;

const sameName = (a, b) => String(a || '').trim().toLocaleLowerCase() === String(b || '').trim().toLocaleLowerCase();

/** Paths of the age screener's review playlist ("Underage review"): never auto-filled. */
export function reviewPlaylistPaths(playlists) {
  const list = (playlists || []).find((p) => sameName(p?.name, AGE_SCREEN.playlist));
  return Array.isArray(list?.videoPaths) ? list.videoPaths : [];
}

/**
 * The auto-filled part of the queue, in play order.
 *
 * Candidates: library videos with a funscript that aren't in `manual`, aren't
 * `currentPath`, aren't in `reviewPaths` and aren't rated at or below
 * DELETE_AT_OR_BELOW (favourites excepted). Order: not played / skipped
 * within `replayAfterMs` first (those that were come after, the longest ago
 * first, so a small library cycles instead of repeating the last video);
 * unrated before rated; preference score high to low (absent = 0); newer
 * `dateAdded` first; path.
 *
 * @param {object} o
 * @param {Array<{path: string, hasFunscript?: boolean, dateAdded?: number}>} o.videos
 * @param {string[]} [o.manual]
 * @param {object} [o.ratings] — `library.ratings`
 * @param {Object<string, number>} [o.scores] — `library.preferenceScores`
 * @param {Object<string, number>} [o.played] — `player.queuePlayed`
 * @param {Iterable<string>} [o.reviewPaths]
 * @param {string|null} [o.currentPath]
 * @param {number} [o.now]
 * @param {number} [o.size] — total queue size, manual included
 * @param {number} [o.replayAfterMs]
 * @returns {string[]}
 */
export function autoFillQueue({
  videos,
  manual = [],
  ratings = {},
  scores = {},
  played = {},
  reviewPaths = [],
  currentPath = null,
  now = Date.now(),
  size = QUEUE_AUTOFILL.size,
  replayAfterMs = REPLAY_AFTER_MS,
}) {
  const take = Math.max(0, size - (manual?.length || 0));
  if (take === 0) return [];
  const skip = new Set(manual || []);
  for (const p of reviewPaths || []) skip.add(p);
  if (currentPath) skip.add(currentPath);

  const candidates = [];
  const seen = new Set();
  for (const v of videos || []) {
    const path = v?.path;
    if (typeof path !== 'string' || !path || !v.hasFunscript || skip.has(path) || seen.has(path)) continue;
    seen.add(path);
    const rating = ratingOf(ratings, path);
    if (rating > 0 && rating <= DELETE_AT_OR_BELOW && !isFavorite(ratings, path)) continue;
    const stamp = played?.[path];
    const recent = Number.isFinite(stamp) && now - stamp < replayAfterMs;
    candidates.push({
      path,
      recent: recent ? 1 : 0,
      playedAt: recent ? stamp : 0,
      rated: rating > 0 ? 1 : 0,
      score: Number.isFinite(scores?.[path]) ? scores[path] : 0,
      added: Number.isFinite(v.dateAdded) ? v.dateAdded : 0,
    });
  }
  candidates.sort((a, b) => (a.recent - b.recent)
    || (a.playedAt - b.playedAt)
    || (a.rated - b.rated)
    || (b.score - a.score)
    || (b.added - a.added)
    || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  return candidates.slice(0, take).map((c) => c.path);
}

/**
 * A copy of `played` with `path` stamped at `now`, pruned to the entries
 * inside the window and at most `cap` of them (newest kept).
 */
export function stampPlayed(played, path, now = Date.now(), { windowMs = REPLAY_AFTER_MS, cap = PLAYED_CAP } = {}) {
  const entries = Object.entries({ ...(played || {}), ...(path ? { [path]: now } : {}) })
    .filter(([, at]) => Number.isFinite(at) && now - at < windowMs)
    .sort((a, b) => b[1] - a[1])
    .slice(0, Math.max(0, cap));
  return Object.fromEntries(entries);
}

/**
 * Apply one queue change to `{manual, played, autoFill}`; returns the new
 * state, reusing the unchanged parts (compare by identity to persist only
 * what changed). Changes, all idempotent (the backend may replay them):
 *   {kind: 'add', path, position: 'end'|'next'}  not queued: insert at the
 *       end / front; queued: `next` moves it to the front, `end` no-op
 *   {kind: 'remove', path}  queued: removed; otherwise stamped as skipped
 *   {kind: 'played', path}  removed from manual and stamped played
 *   {kind: 'clear'}         manual = []
 *   {kind: 'auto-fill', enabled}
 */
export function applyQueueChange(state, change, { now = Date.now(), windowMs, cap } = {}) {
  const { manual = [], played = {}, autoFill = true } = state || {};
  const opts = { windowMs, cap };
  for (const k of Object.keys(opts)) if (opts[k] === undefined) delete opts[k];
  const path = change?.path;
  switch (change?.kind) {
    case 'add': {
      const idx = manual.indexOf(path);
      if (idx < 0) {
        return { manual: change.position === 'next' ? [path, ...manual] : [...manual, path], played, autoFill };
      }
      if (change.position === 'next' && idx > 0) {
        return { manual: [path, ...manual.filter((p) => p !== path)], played, autoFill };
      }
      return { manual, played, autoFill };
    }
    case 'remove':
      if (manual.includes(path)) return { manual: manual.filter((p) => p !== path), played, autoFill };
      return { manual, played: stampPlayed(played, path, now, opts), autoFill };
    case 'played':
      return {
        manual: manual.includes(path) ? manual.filter((p) => p !== path) : manual,
        played: stampPlayed(played, path, now, opts),
        autoFill,
      };
    case 'clear':
      return { manual: manual.length ? [] : manual, played, autoFill };
    case 'auto-fill':
      return { manual, played, autoFill: change.enabled === true };
    default:
      throw new Error(`unknown queue change ${JSON.stringify(change?.kind)}`);
  }
}
