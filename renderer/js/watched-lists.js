// Where watched videos sink (user, 2026-11-10). A video is watched once it
// has a rating (ratings.js `isWatched`); in the lists below the watched ones
// move to the end (`watchedLast`), dimmed and badged, so they aren't played
// again by accident. On by default; `library.watchedLast` false turns it off.
//
// Lists that sink: the library (every view and sort), categories and
// collections (auto and the user's). Lists that keep their own order: every
// playlist — artist playlists and "Favourite artists" (artist-playlists.js,
// `library.artistPlaylists`), the smart playlists (smart-playlists.js; they
// hold only unwatched videos, best or newest first), the age screener's
// review playlist and the user's own — and the rating lists (rating filter /
// sort, the Watched collection).
//
// The backend gets the setting with the registration (`watched`), so the
// web remote and the headsets sink in the same lists.

import { AGE_SCREEN, FAVOURITES_PLAYLIST } from './content-preferences.js';
import { isSmartPlaylistName } from './smart-playlists.js';

export const WATCHED_LAST_KEY = 'library.watchedLast';
/** Id of the Watched collection (every rated video, newest rating first). Not a uuid, never stored in `library.collections`. */
export const WATCHED_COLLECTION_ID = ':watched';

/** List kinds. */
export const LIST_LIBRARY = 'library';
export const LIST_CATEGORY = 'category';
export const LIST_COLLECTION = 'collection';
export const LIST_SMART_PLAYLIST = 'smart';
export const LIST_ARTIST_PLAYLIST = 'artist';
export const LIST_RATING = 'rating';
export const LIST_WATCHED = 'watched';
export const LIST_HAND_PLAYLIST = 'hand';

const SINKING = new Set([LIST_LIBRARY, LIST_CATEGORY, LIST_COLLECTION]);

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();

/** True when watched videos sink in a list of this kind. */
export function sinksWatched(kind) {
  return SINKING.has(kind);
}

/** The setting: on unless set to false. */
export function watchedLastOn(value) {
  return value !== false;
}

/**
 * What kind of playlist `playlist` is: `artist` (an artist playlist the app
 * made, or "Favourite artists"), `smart` (a smart playlist), `rating` (the
 * age screener's review list) or `hand` (the user's own).
 *
 * @param {{name?: string}} playlist
 * @param {{created?: string[]}} [artistState] `library.artistPlaylists`
 */
export function playlistKind(playlist, artistState = {}) {
  const name = fold(playlist?.name);
  if (name === fold(FAVOURITES_PLAYLIST)) return LIST_ARTIST_PLAYLIST;
  if (name && new Set(artistState?.created || []).has(name)) return LIST_ARTIST_PLAYLIST;
  if (isSmartPlaylistName(name)) return LIST_SMART_PLAYLIST;
  if (name === fold(AGE_SCREEN.playlist)) return LIST_RATING;
  return LIST_HAND_PLAYLIST;
}

/**
 * The `watched` part of a backend registration: `{sink}`, the setting.
 *
 * @param {{get: (key: string) => unknown}} settings
 */
export function watchedRegistration(settings) {
  return { watched: { sink: watchedLastOn(settings.get(WATCHED_LAST_KEY)) } };
}
