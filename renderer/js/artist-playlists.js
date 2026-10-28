// One playlist per artist with at least MIN_ARTIST_VIDEOS videos, and per
// favourite artist from the first video, plus one FAVOURITES_PLAYLIST (the
// values are in content-preferences.js).
//
// The library is laid out `<source>/<artist>/<scene>/<video>` (the scout
// names Discovery's artist folders the same way), so a video's artist is the
// first folder under its source. After each library scan every artist folder
// holding at least MIN_ARTIST_VIDEOS videos, across all sources, gets a
// playlist named after it, and new videos of that artist join it. A
// favourite (FAVOURITES, matched to the folder name whatever its case or
// Unicode form) gets its playlist from one video, and every favourite's
// videos also go into FAVOURITES_PLAYLIST, grouped by artist.
//
// The user stays in charge, as with the EroScripts categories: an existing
// playlist of the same name is reused, a playlist is made once (a deleted
// one is not made again), and each video is added once (taking it out by
// hand sticks). That history is `library.artistPlaylists`: `{created:
// [artistKey], added: {videoPath: [artistKey]}}`, where the combined
// playlist's key is FAVOURITES_STATE_KEY (a colon can't be in a folder name).
// A playlist the cleanup removes for falling below the threshold
// (playlist-cleanup.js) is forgotten there, so it is made again later.

import { FAVOURITES_PLAYLIST, MIN_ARTIST_VIDEOS, NOT_AN_ARTIST } from './content-preferences.js';

export const ARTIST_PLAYLISTS_KEY = 'library.artistPlaylists';
export const FAVOURITES_STATE_KEY = ':favourites';

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();
const baseName = (p) => p.split(/[\\/]/).pop() || p;

/**
 * The artist folder of a video: the first folder under its source, when the
 * video sits at least one more folder deeper (`<artist>/<scene>/<video>`).
 *
 * @param {string} videoPath
 * @param {string[]} sourcePaths
 * @returns {string|null}
 */
export function artistOf(videoPath, sourcePaths) {
  const p = String(videoPath || '').replace(/\\/g, '/');
  const root = (sourcePaths || [])
    .map((s) => String(s || '').replace(/\\/g, '/').replace(/\/+$/, ''))
    .filter((s) => s && fold(p).startsWith(`${fold(s)}/`))
    .sort((a, b) => b.length - a.length)[0];
  if (!root) return null;
  const parts = p.slice(root.length + 1).split('/');
  if (parts.length < 3) return null;
  const artist = parts[0].trim();
  return artist && !NOT_AN_ARTIST.has(fold(artist)) ? artist : null;
}

/**
 * What to create and add.
 *
 * @param {object} o
 * @param {string[]} o.videoPaths the library's videos
 * @param {string[]} o.sourcePaths library source folders
 * @param {{id: string, name: string, videoPaths?: string[]}[]} o.playlists
 * @param {{created?: string[], added?: Object<string, string[]>}} [o.state] `library.artistPlaylists`
 * @param {string[]} [o.favourites] favourite artist names
 * @returns {{create: {name: string, videoPaths: string[]}[], extend: {id: string, videoPaths: string[]}[],
 *   state: {created: string[], added: Object<string, string[]>}}}
 *   `extend` holds the videos to append to each existing playlist
 */
export function planArtistPlaylists({ videoPaths = [], sourcePaths = [], playlists = [], state = {}, favourites = [] }) {
  const groups = new Map();
  for (const p of videoPaths) {
    const artist = artistOf(p, sourcePaths);
    if (!artist) continue;
    const g = groups.get(fold(artist)) || { spellings: new Map(), paths: [] };
    g.spellings.set(artist, (g.spellings.get(artist) || 0) + 1);
    g.paths.push(p);
    groups.set(fold(artist), g);
  }

  const favouriteKeys = new Set(favourites.map(fold).filter(Boolean));
  const byName = new Map(playlists.map((pl) => [fold(pl.name), pl]));
  const created = new Set(state.created || []);
  const added = { ...(state.added || {}) };
  const create = [];
  const extend = [];
  const plan = (key, name, paths) => {
    const existing = byName.get(fold(name));
    if (!existing && created.has(key)) return;
    created.add(key);
    const inList = new Set(existing?.videoPaths || []);
    const fresh = paths.filter((p) => !inList.has(p) && !(added[p] || []).includes(key));
    for (const p of paths) if (!(added[p] || []).includes(key)) added[p] = [...(added[p] || []), key];
    if (!existing) create.push({ name, videoPaths: paths });
    else if (fresh.length) extend.push({ id: existing.id, videoPaths: fresh });
  };

  const artists = [...groups]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, g]) => ({
      key,
      name: [...g.spellings].sort((a, b) => b[1] - a[1])[0][0],
      paths: [...g.paths].sort((a, b) => baseName(a).localeCompare(baseName(b), undefined, { numeric: true, sensitivity: 'base' })),
      favourite: favouriteKeys.has(key),
    }));
  const favouritePaths = artists.filter((a) => a.favourite).flatMap((a) => a.paths);
  if (favouritePaths.length) plan(FAVOURITES_STATE_KEY, FAVOURITES_PLAYLIST, favouritePaths);
  for (const a of artists) {
    if (a.favourite || a.paths.length >= MIN_ARTIST_VIDEOS) plan(a.key, a.name, a.paths);
  }
  return { create, extend, state: { created: [...created], added } };
}

/**
 * Create and fill the artist playlists for `videoPaths`.
 *
 * @param {object} data DataService (get, set, getPlaylists, addPlaylist, addVideosToPlaylist)
 * @param {string[]} videoPaths
 * @param {string[]} [favourites] favourite artist names (FAVOURITES in content-preferences.js)
 * @returns {Promise<{created: string[], extended: number}>} names of new playlists, count of grown ones
 */
export async function applyArtistPlaylists(data, videoPaths, favourites = []) {
  const sourcePaths = (data.get('library.sources') || []).map((s) => s?.path).filter(Boolean);
  const plan = planArtistPlaylists({ videoPaths, sourcePaths, playlists: data.getPlaylists(), state: data.get(ARTIST_PLAYLISTS_KEY) || {}, favourites });
  for (const c of plan.create) {
    const pl = await data.addPlaylist(c.name);
    data.addVideosToPlaylist(pl.id, c.videoPaths);
  }
  for (const e of plan.extend) data.addVideosToPlaylist(e.id, e.videoPaths);
  if (JSON.stringify(plan.state) !== JSON.stringify(data.get(ARTIST_PLAYLISTS_KEY) || {})) data.set(ARTIST_PLAYLISTS_KEY, plan.state);
  return { created: plan.create.map((c) => c.name), extended: plan.extend.length };
}
