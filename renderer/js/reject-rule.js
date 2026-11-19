// Triage rule, renderer side (threshold in content-preferences.js).
//
// • Rated DELETE_AT_OR_BELOW or lower (a favourite never is): deleted from
//   disk (electron/rejected-videos.js, through the `delete-rejected-video`
//   IPC), taken out of every playlist and its script association dropped.
// • Any other rating keeps the video where it is (the Review playlist for
//   2½–3½ stars was retired on 2026-11-10).
// • rejectVideo: the user deletes one video on purpose (the Underage review
//   playlist's Delete button), favourite or not, through the same rule.
// • deleteDuplicate: the swipe deck's Duplicates tab (a `delete-duplicate`
//   library edit) deletes one copy of a video the user has twice, favourite
//   or not, with reason `duplicate` (the scouts skip that video, not its post).
// • forgetDeletedVideo: what every deletion does after the files are gone.
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
 * Delete `videoPath` through the triage rule: rated DELETE_AT_OR_BELOW and no
 * longer a favourite, so the next sweep deletes it from disk, takes it out of
 * every playlist and records the rejection (the scouts never fetch it again).
 *
 * @param {object} data DataService (setRating)
 * @param {string} videoPath
 */
export function rejectVideo(data, videoPath) {
  data.setRating(videoPath, { rating: DELETE_AT_OR_BELOW, favorite: false });
}

/**
 * Forget a video deleted from disk: out of every playlist, its rating
 * cleared and its script association dropped.
 *
 * @param {object} data DataService (get, set, getPlaylists, removeVideoFromPlaylist, getRatings, setRating)
 * @param {string} videoPath
 */
export function forgetDeletedVideo(data, videoPath) {
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
}

/**
 * Delete every video rated too low.
 * A video whose deletion fails stays where it is, so the next sweep tries
 * again; a deleted one is forgotten (forgetDeletedVideo).
 *
 * @param {object} o
 * @param {object} o.data DataService (see forgetDeletedVideo)
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
    forgetDeletedVideo(data, videoPath);
    removed.push({ path: videoPath, reason });
  }
  return { removed, failed };
}

/**
 * Delete `videoPath` from disk as a duplicate of `keepPath`, favourite or
 * not, and forget it. Refuses while the kept copy is not on disk, so two
 * crossed choices never delete both copies. A video already gone deletes
 * nothing (a replayed edit) and is forgotten all the same.
 *
 * @param {object} o
 * @param {object} o.data DataService (see forgetDeletedVideo)
 * @param {(videoPath: string, reason: string) => Promise<{success: boolean, deleted?: string[], error?: string}>} o.deleteVideo
 * @param {(videoPath: string) => Promise<boolean>} o.keepExists
 * @param {string} o.videoPath
 * @param {string} o.keepPath
 * @returns {Promise<{deleted: string[]}>} the files deleted
 * @throws when the kept copy is missing or the deletion fails (the video stays)
 */
export async function deleteDuplicate({ data, deleteVideo, keepExists, videoPath, keepPath }) {
  if (videoPath === keepPath) throw new Error('a video can\'t be its own duplicate');
  if (!(await keepExists(keepPath))) throw new Error(`kept copy ${keepPath} is not on disk, ${videoPath} stays`);
  const result = await deleteVideo(videoPath, 'duplicate');
  if (!result?.success) throw new Error(`kept ${videoPath}: ${result?.error || 'unknown error'}`);
  forgetDeletedVideo(data, videoPath);
  return { deleted: result.deleted || [] };
}
