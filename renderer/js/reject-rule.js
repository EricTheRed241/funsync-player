// Triage rules, renderer side (thresholds in content-preferences.js).
//
// • Rated DELETE_AT_OR_BELOW or lower (a favourite never is): deleted from
//   disk (electron/rejected-videos.js, through the `delete-rejected-video`
//   IPC) and taken out of every playlist.
// • Rated between that and KEEP_FROM: put in the REVIEW_PLAYLIST (made when
//   needed).
// • Rated KEEP_FROM or more: taken out of Review.
//
// Runs at startup and whenever a rating changes, from the desktop, the web
// remote or HereSphere (all go through the data service).

import { getRatingEntry } from './ratings.js';
import { DELETE_AT_OR_BELOW, KEEP_FROM, REVIEW_PLAYLIST } from './content-preferences.js';

// Videos the startup sweep already put in Review, so taking one out by hand sticks.
export const REVIEW_HISTORY_KEY = 'library.reviewAdded';

const sameName = (a, b) => String(a || '').trim().toLocaleLowerCase() === b.toLocaleLowerCase();

/**
 * What a rating means: 'delete' (rated, ≤ DELETE_AT_OR_BELOW, not a
 * favourite), 'review' (in between), 'keep' (≥ KEEP_FROM), or null (unrated).
 *
 * @param {{rating?: number, favorite?: boolean}|null} entry
 */
export function ratingVerdict(entry) {
  const rating = Number(entry?.rating) || 0;
  if (rating <= 0) return null;
  if (rating <= DELETE_AT_OR_BELOW) return entry.favorite === true ? 'review' : 'delete';
  return rating >= KEEP_FROM ? 'keep' : 'review';
}

/**
 * Delete every video rated too low.
 * A video whose deletion fails stays where it is, so the next sweep tries
 * again; a deleted one leaves every playlist and loses its rating.
 *
 * @param {object} o
 * @param {object} o.data DataService (getPlaylists, removeVideoFromPlaylist, getRatings, setRating)
 * @param {(videoPath: string, reason: string) => Promise<{success: boolean, error?: string}>} o.deleteVideo
 * @returns {Promise<{removed: {path: string, reason: string}[], failed: {path: string, error: string}[]}>}
 */
export async function sweepRejected({ data, deleteVideo }) {
  const removed = [];
  const failed = [];
  const targets = new Map();
  const ratings = data.getRatings();
  for (const p of Object.keys(ratings)) {
    if (ratingVerdict(getRatingEntry(ratings, p)) === 'delete') targets.set(p, 'low-rating');
  }
  for (const [videoPath, reason] of targets) {
    const result = await deleteVideo(videoPath, reason);
    if (!result?.success) {
      failed.push({ path: videoPath, error: result?.error || 'unknown error' });
      continue;
    }
    for (const pl of data.getPlaylists()) {
      if ((pl.videoPaths || []).includes(videoPath)) data.removeVideoFromPlaylist(pl.id, videoPath);
    }
    if (getRatingEntry(data.getRatings(), videoPath)) data.setRating(videoPath, { rating: 0, favorite: false });
    removed.push({ path: videoPath, reason });
  }
  return { removed, failed };
}

/**
 * Keep the Review playlist in line with ratings.
 *
 * Without `changedPath` (startup): every 2½–3½-star video not put in Review
 * before joins it, and every 4-star-plus one leaves. With `changedPath` (a
 * rating just changed): only that video moves, and a 2½–3½ one joins even
 * if it was taken out before.
 *
 * @param {object} data DataService (get, set, getPlaylists, addPlaylist, addVideosToPlaylist, removeVideoFromPlaylist, getRatings)
 * @param {string} [changedPath]
 * @returns {Promise<{added: number, removed: number}>}
 */
export async function syncReview(data, changedPath) {
  const ratings = data.getRatings();
  const history = new Set(data.get(REVIEW_HISTORY_KEY) || []);
  const paths = changedPath ? [changedPath] : Object.keys(ratings);
  const toAdd = [];
  const toRemove = [];
  for (const p of paths) {
    const verdict = ratingVerdict(getRatingEntry(ratings, p));
    if (verdict === 'review' && (changedPath || !history.has(p))) toAdd.push(p);
    else if (verdict === 'keep') toRemove.push(p);
  }
  let review = data.getPlaylists().find((pl) => sameName(pl.name, REVIEW_PLAYLIST));
  const fresh = toAdd.filter((p) => !(review?.videoPaths || []).includes(p));
  if (fresh.length && !review) review = await data.addPlaylist(REVIEW_PLAYLIST);
  if (fresh.length) data.addVideosToPlaylist(review.id, fresh);
  const leaving = review ? toRemove.filter((p) => (review.videoPaths || []).includes(p)) : [];
  for (const p of leaving) data.removeVideoFromPlaylist(review.id, p);
  if (toAdd.some((p) => !history.has(p))) data.set(REVIEW_HISTORY_KEY, [...new Set([...history, ...toAdd])]);
  return { added: fresh.length, removed: leaving.length };
}
