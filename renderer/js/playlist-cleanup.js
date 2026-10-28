// Playlist cleanup after each library scan (after the artist playlists are
// brought up to date):
//
// • A playlist with no video left in the library goes, whoever made it. The
//   Discovery and Review playlists are made again when a video needs them.
// • An artist playlist the app made (artist-playlists.js) that holds fewer
//   than MIN_ARTIST_VIDEOS videos still in the library goes too (a favourite's
//   from one video, so only when empty). Its artist is forgotten in
//   `library.artistPlaylists`, so the playlist is made again, with all the
//   artist's videos, once the artist reaches the threshold.
//
// Only videos the scan found count: paths of deleted videos (kept in a
// playlist while the app was open, shown as missing) don't.

import { FAVOURITES_PLAYLIST, MIN_ARTIST_VIDEOS } from './content-preferences.js';
import { ARTIST_PLAYLISTS_KEY, FAVOURITES_STATE_KEY } from './artist-playlists.js';

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();

/**
 * Which playlists to delete, and the artist-playlist history afterwards.
 *
 * @param {object} o
 * @param {{id: string, name: string, videoPaths?: string[]}[]} o.playlists
 * @param {string[]} o.videoPaths the library's videos (this scan)
 * @param {{created?: string[], added?: Object<string, string[]>}} [o.state] `library.artistPlaylists`
 * @param {string[]} [o.favourites] favourite artist names
 * @returns {{remove: {id: string, name: string, videos: number}[], state: {created: string[], added: Object<string, string[]>}}}
 */
export function planPlaylistCleanup({ playlists = [], videoPaths = [], state = {}, favourites = [] }) {
  const inLibrary = new Set(videoPaths);
  const favouriteKeys = new Set(favourites.map(fold));
  const created = new Set(state.created || []);
  const forget = new Set();
  const remove = [];
  for (const pl of playlists) {
    const videos = (pl.videoPaths || []).filter((p) => inLibrary.has(p)).length;
    const key = fold(pl.name) === fold(FAVOURITES_PLAYLIST) ? FAVOURITES_STATE_KEY : fold(pl.name);
    const artistMade = created.has(key);
    const min = !artistMade || key === FAVOURITES_STATE_KEY || favouriteKeys.has(key) ? 1 : MIN_ARTIST_VIDEOS;
    if (videos >= min) continue;
    remove.push({ id: pl.id, name: pl.name, videos });
    if (artistMade) forget.add(key);
  }
  if (!forget.size) return { remove, state };
  const added = {};
  for (const [p, keys] of Object.entries(state.added || {})) {
    const rest = keys.filter((k) => !forget.has(k));
    if (rest.length) added[p] = rest;
  }
  return { remove, state: { ...state, created: [...created].filter((k) => !forget.has(k)), added } };
}

/**
 * Delete the playlists `planPlaylistCleanup` picks.
 *
 * @param {object} data DataService (get, set, getPlaylists, deletePlaylist)
 * @param {string[]} videoPaths
 * @param {string[]} [favourites]
 * @returns {{removed: string[]}} names of the deleted playlists
 */
export function applyPlaylistCleanup(data, videoPaths, favourites = []) {
  const before = data.get(ARTIST_PLAYLISTS_KEY) || {};
  const plan = planPlaylistCleanup({ playlists: data.getPlaylists(), videoPaths, state: before, favourites });
  for (const r of plan.remove) data.deletePlaylist(r.id);
  if (plan.state !== before) data.set(ARTIST_PLAYLISTS_KEY, plan.state);
  return { removed: plan.remove.map((r) => r.name) };
}
