// Watched videos on the phone (user, 2026-11-10). A video is watched once
// it has a rating (`watched` from /api/remote/videos); a favourite alone
// doesn't count. So they aren't played again by accident, the lists the
// desktop sinks put them last, dimmed and badged: the library and folder
// views while the desktop setting is on (`watchedLast`), and every grouping
// whose `sinkWatched` is true (categories, collections; playlists keep
// their order). The rating views keep their order: the favourites / top rating
// filters and the rating sort. The virtual Watched collection (`:watched`)
// lists the rated videos newest rating first.

import { t } from './i18n.js';

/** Id of the virtual Watched collection (same as the desktop's). */
export const WATCHED_COLLECTION_ID = ':watched';

/** Rating filters that are rating lists themselves, so nothing sinks there. */
const RATING_LIST_FILTERS = new Set(['favorites', 'top']);

/** `videos` with the watched ones moved to the end; each group keeps its order. Returns a new array. */
export function watchedLast(videos) {
  const fresh = [];
  const seen = [];
  for (const v of videos || []) (v?.watched ? seen : fresh).push(v);
  return fresh.concat(seen);
}

/**
 * Whether watched videos sink in a list: the list sinks (`sink`: the
 * library's `watchedLast`, a grouping's `sinkWatched`), and the user isn't
 * looking at a rating list (`rating` filter favourites / top, or a
 * `rating:<dir>` sort).
 */
export function shouldSink({ sink, rating, sort } = {}) {
  if (sink !== true) return false;
  if (RATING_LIST_FILTERS.has(rating)) return false;
  return String(sort || '').split(':')[0] !== 'rating';
}

/** True for the virtual Watched collection of /api/remote/collections. */
export function isWatchedCollection(item) {
  return item?.kind === 'watched';
}

/** The collections with the Watched one renamed to `name` (its localized name). */
export function localizeWatchedCollection(collections, name) {
  return (collections || []).map((c) => (isWatchedCollection(c) ? { ...c, name } : c));
}

/** The videos named by `ids`, in that order; unknown ids are skipped. */
export function videosInOrder(videos, ids) {
  const byId = new Map((videos || []).map((v) => [v.id, v]));
  return (ids || []).map((id) => byId.get(id)).filter(Boolean);
}

/** The "Watched" badge for a card (`card__badge`) or row (`list__badge`), else null. */
export function createWatchedBadge(video, className) {
  if (!video?.watched) return null;
  const b = document.createElement('span');
  b.className = className;
  b.textContent = t('webRemote.triage.watched');
  const title = t('webRemote.triage.watchedTitle', { n: video.rating || 0 });
  b.title = title;
  b.setAttribute('aria-label', title);
  return b;
}
