// Where watched videos sink (user, 2026-11-10). A video is watched once it
// has a rating (ratings.js `isWatched`); in the lists below the watched ones
// move to the end (`watchedLast`), dimmed and badged, so they aren't played
// again by accident. On by default; `library.watchedLast` false turns it off.
//
// Lists that sink: the library (every view and sort), categories,
// collections (auto and the user's) and the app-made playlists
// (DISCOVERY_PLAYLIST). Lists that keep their own order: artist playlists
// and "Favourite artists" (artist-playlists.js, `library.artistPlaylists`),
// the rating lists (rating filter / sort, Review, the age screener's review
// playlist, the Watched collection) and every other playlist: the user's own.
//
// The backend gets the same decision with the registration (`watched`), so
// the web remote and the headsets sink in the same lists.

import { AGE_SCREEN, DISCOVERY_PLAYLIST, FAVOURITES_PLAYLIST, REVIEW_PLAYLIST } from './content-preferences.js';
import { ARTIST_PLAYLISTS_KEY } from './artist-playlists.js';

export const WATCHED_LAST_KEY = 'library.watchedLast';
/** Id of the Watched collection (every rated video, newest rating first). Not a uuid, never stored in `library.collections`. */
export const WATCHED_COLLECTION_ID = ':watched';

/** List kinds. */
export const LIST_LIBRARY = 'library';
export const LIST_CATEGORY = 'category';
export const LIST_COLLECTION = 'collection';
export const LIST_APP_PLAYLIST = 'app';
export const LIST_ARTIST_PLAYLIST = 'artist';
export const LIST_RATING = 'rating';
export const LIST_WATCHED = 'watched';
export const LIST_HAND_PLAYLIST = 'hand';

const SINKING = new Set([LIST_LIBRARY, LIST_CATEGORY, LIST_COLLECTION, LIST_APP_PLAYLIST]);

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
 * made, or "Favourite artists"), `rating` (Review, the age screener's
 * review list), `app` (Discovery) or `hand` (the user's own).
 *
 * @param {{name?: string}} playlist
 * @param {{created?: string[]}} [artistState] `library.artistPlaylists`
 */
export function playlistKind(playlist, artistState = {}) {
  const name = fold(playlist?.name);
  if (name === fold(FAVOURITES_PLAYLIST)) return LIST_ARTIST_PLAYLIST;
  if (name && new Set(artistState?.created || []).has(name)) return LIST_ARTIST_PLAYLIST;
  if (name === fold(REVIEW_PLAYLIST) || name === fold(AGE_SCREEN.playlist)) return LIST_RATING;
  if (name === fold(DISCOVERY_PLAYLIST)) return LIST_APP_PLAYLIST;
  return LIST_HAND_PLAYLIST;
}

/**
 * The `watched` part of a backend registration: `{sink, playlistIds}`,
 * `sink` the setting, `playlistIds` the playlists whose videos sink.
 *
 * @param {{get: (key: string) => unknown, getPlaylists?: () => object[]}} settings
 */
export function watchedRegistration(settings) {
  const playlists = settings.getPlaylists ? settings.getPlaylists() : [];
  const artistState = settings.get(ARTIST_PLAYLISTS_KEY) || {};
  return {
    watched: {
      sink: watchedLastOn(settings.get(WATCHED_LAST_KEY)),
      playlistIds: playlists.filter((pl) => sinksWatched(playlistKind(pl, artistState))).map((pl) => pl.id),
    },
  };
}
