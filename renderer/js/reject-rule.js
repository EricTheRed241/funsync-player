// Triage rule, renderer side (threshold in content-preferences.js).
//
// • Rated DELETE_AT_OR_BELOW or lower (a favourite never is): deleted from
//   disk (electron/rejected-videos.js, through the `delete-rejected-video`
//   IPC), taken out of every playlist and its script association dropped.
// • Any other rating keeps the video where it is (the Review playlist for
//   2½–3½ stars was retired on 2026-11-10).
//
// Runs at startup and whenever a rating changes, from the desktop, the web
// remote or HereSphere (all go through the data service).

import { getRatingEntry } from './ratings.js';
import { DELETE_AT_OR_BELOW } from './content-preferences.js';

const ASSOCIATIONS_KEY = 'library.associations';

/**
 * What a rating means: 'delete' (rated, ≤ DELETE_AT_OR_BELOW, not a
 * favourite), 'keep' (any other rating), or null (unrated).
 *
 * @param {{rating?: number, favorite?: boolean}|null} entry
 */
export function ratingVerdict(entry) {
  const rating = Number(entry?.rating) || 0;
  if (rating <= 0) return null;
  return rating <= DELETE_AT_OR_BELOW && entry.favorite !== true ? 'delete' : 'keep';
}

/**
 * Delete every video rated too low.
 * A video whose deletion fails stays where it is, so the next sweep tries
 * again; a deleted one leaves every playlist, loses its rating and its
 * script association.
 *
 * @param {object} o
 * @param {object} o.data DataService (get, set, getPlaylists, removeVideoFromPlaylist, getRatings, setRating)
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
    const associations = data.get(ASSOCIATIONS_KEY);
    if (associations && Object.hasOwn(associations, videoPath)) {
      const next = { ...associations };
      delete next[videoPath];
      data.set(ASSOCIATIONS_KEY, next);
    }
    removed.push({ path: videoPath, reason });
  }
  return { removed, failed };
}
