// Ratings + favourites — pure helpers over `library.ratings`.
//
// Shape (keyed by absolute VIDEO PATH, like `library.resumePositions`):
//   `{ rating: 0..5 in 0.5 steps, favorite: boolean, updatedAt: epochMs }`
//
// 0 means "unrated". An entry that is unrated AND not a favourite carries no
// information, so it is deleted rather than stored — the map only ever holds
// videos someone actually triaged.
//
// The 0–5 / 0.5 scale is HereSphere's, so a headset rating round-trips
// without rounding. The backend validates with the same rule
// (`backend/services/library_edits.py::normalize_rating`).
//
// DOM- and settings-free so the validation and the move rules are unit
// testable on their own.

import { planAssociationRehome } from './association-rehome.js';
import { TOP_RATED_MIN } from './content-preferences.js';

export const RATING_MAX = 5;
export const RATING_STEP = 0.5;

/**
 * `value` as a valid rating, or null when it isn't one.
 *
 * Valid: a finite number (or numeric string) in [0, 5] that is a multiple
 * of 0.5. Nothing is rounded or clamped — an out-of-range or off-step value
 * is a caller bug or hostile input, and silently "fixing" it would store a
 * rating nobody chose.
 *
 * @param {unknown} value
 * @returns {number|null}
 */
export function normalizeRating(value) {
  if (typeof value === 'boolean' || value === null || value === undefined || value === '') return null;
  const n = typeof value === 'string' ? Number(value.trim()) : value;
  if (typeof n !== 'number' || !Number.isFinite(n)) return null;
  if (n < 0 || n > RATING_MAX) return null;
  if (!Number.isInteger(n / RATING_STEP)) return null;
  return n;
}

/**
 * The stored entry for `path`, normalised, or null when it has none.
 * Tolerates hand-edited / older shapes: a bad rating reads as 0.
 *
 * @param {object} map — `library.ratings`
 * @param {string} path
 * @returns {{rating: number, favorite: boolean, updatedAt: number}|null}
 */
export function getRatingEntry(map, path) {
  const raw = map && path ? map[path] : null;
  if (!raw || typeof raw !== 'object') return null;
  return {
    rating: normalizeRating(raw.rating) ?? 0,
    favorite: raw.favorite === true,
    updatedAt: Number.isFinite(raw.updatedAt) ? raw.updatedAt : 0,
  };
}

/** Rating of `path` (0 when unrated). */
export function ratingOf(map, path) {
  return getRatingEntry(map, path)?.rating || 0;
}

/** True when `path` is a favourite. */
export function isFavorite(map, path) {
  return getRatingEntry(map, path)?.favorite === true;
}

/** Library rating-filter values; 'all' = no filter. */
export const RATING_FILTERS = ['all', 'favorites', 'top', 'unrated'];

/**
 * Does `path` pass the library rating filter? `favorites` = favourites,
 * `top` = rated >= TOP_RATED_MIN (same rule as the headset's Top Rated),
 * `unrated` = no rating (the triage queue). Anything else passes.
 */
export function matchesRatingFilter(map, path, filter) {
  switch (filter) {
    case 'favorites': return isFavorite(map, path);
    case 'top': return ratingOf(map, path) >= TOP_RATED_MIN;
    case 'unrated': return ratingOf(map, path) === 0;
    default: return true;
  }
}

/**
 * Apply a partial change to one video's entry and return the NEW map
 * (the input is not mutated) plus the resulting entry.
 *
 * `change.rating` / `change.favorite` are each optional; an absent field
 * keeps its current value. Throws on an invalid rating so a caller can
 * reject the edit instead of storing something nobody chose.
 *
 * @param {object} map
 * @param {string} path
 * @param {{rating?: unknown, favorite?: unknown}} change
 * @param {number} [now]
 * @returns {{map: object, entry: object|null}} entry null = removed
 */
export function applyRatingChange(map, path, change, now = Date.now()) {
  if (!path || typeof path !== 'string') throw new Error('ratings: path required');
  const c = change || {};
  const hasRating = c.rating !== undefined;
  const hasFavorite = c.favorite !== undefined;
  if (!hasRating && !hasFavorite) throw new Error('ratings: nothing to change');

  const current = getRatingEntry(map, path) || { rating: 0, favorite: false };
  let rating = current.rating;
  if (hasRating) {
    rating = normalizeRating(c.rating);
    if (rating === null) throw new Error(`ratings: invalid rating ${JSON.stringify(c.rating)}`);
  }
  const favorite = hasFavorite ? c.favorite === true : current.favorite;

  const next = { ...(map || {}) };
  if (rating === 0 && !favorite) {
    delete next[path];
    return { map: next, entry: null };
  }
  const entry = { rating, favorite, updatedAt: now };
  next[path] = entry;
  return { map: next, entry };
}

/**
 * Move rating entries to videos that MOVED, with exactly the association
 * rules (unambiguous 1:1 basename match, never robbing a live video, never
 * touching entries on an offline source). See association-rehome.js.
 *
 * @param {object} map — `library.ratings`
 * @param {string[]} scannedPaths
 * @param {(path: string) => boolean} [isUnavailable]
 * @returns {{map: object, moves: Array<{from: string, to: string}>}}
 */
export function rehomeRatings(map, scannedPaths, isUnavailable) {
  const stored = Object.keys(map || {});
  const moves = planAssociationRehome({ storedPaths: stored, scannedPaths, isUnavailable });
  if (moves.length === 0) return { map, moves };
  const next = { ...map };
  for (const { from, to } of moves) {
    next[to] = next[from];
    delete next[from];
  }
  return { map: next, moves };
}
