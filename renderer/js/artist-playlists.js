// One playlist per artist with more than six videos.
//
// The library is laid out `<source>/<artist>/<scene>/<video>` (the scout
// names Discovery's artist folders the same way), so a video's artist is the
// first folder under its source. After each library scan every artist folder
// holding at least MIN_ARTIST_VIDEOS videos, across all sources, gets a
// playlist named after it, and new videos of that artist join it.
//
// The user stays in charge, as with the EroScripts categories: an existing
// playlist of the same name is reused, a playlist is made once (a deleted
// one is not made again), and each video is added once (taking it out by
// hand sticks). That history is `library.artistPlaylists`: `{created:
// [artistKey], added: {videoPath: [artistKey]}}`.

export const ARTIST_PLAYLISTS_KEY = 'library.artistPlaylists';
/** "More than six videos". */
export const MIN_ARTIST_VIDEOS = 7;
// Folders that hold videos of many artists.
const NOT_AN_ARTIST = new Set(['unknown', 'misc', 'various', 'other', 'others', 'new folder', 'cock hero', 'fap hero', 'pmv', 'hmv', 'compilation', 'compilations']);

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
 * @returns {{create: {name: string, videoPaths: string[]}[], extend: {id: string, videoPaths: string[]}[],
 *   state: {created: string[], added: Object<string, string[]>}}}
 *   `extend` holds the videos to append to each existing playlist
 */
export function planArtistPlaylists({ videoPaths = [], sourcePaths = [], playlists = [], state = {} }) {
  const groups = new Map();
  for (const p of videoPaths) {
    const artist = artistOf(p, sourcePaths);
    if (!artist) continue;
    const g = groups.get(fold(artist)) || { spellings: new Map(), paths: [] };
    g.spellings.set(artist, (g.spellings.get(artist) || 0) + 1);
    g.paths.push(p);
    groups.set(fold(artist), g);
  }

  const byName = new Map(playlists.map((pl) => [fold(pl.name), pl]));
  const created = new Set(state.created || []);
  const added = { ...(state.added || {}) };
  const create = [];
  const extend = [];
  for (const [key, g] of [...groups].sort(([a], [b]) => a.localeCompare(b))) {
    if (g.paths.length < MIN_ARTIST_VIDEOS) continue;
    const name = [...g.spellings].sort((a, b) => b[1] - a[1])[0][0];
    const paths = [...g.paths].sort((a, b) => baseName(a).localeCompare(baseName(b), undefined, { numeric: true, sensitivity: 'base' }));
    const existing = byName.get(fold(name));
    if (!existing && created.has(key)) continue;
    created.add(key);
    const inList = new Set(existing?.videoPaths || []);
    const fresh = paths.filter((p) => !inList.has(p) && !(added[p] || []).includes(key));
    for (const p of paths) if (!(added[p] || []).includes(key)) added[p] = [...(added[p] || []), key];
    if (!existing) create.push({ name, videoPaths: paths });
    else if (fresh.length) extend.push({ id: existing.id, videoPaths: fresh });
  }
  return { create, extend, state: { created: [...created], added } };
}

/**
 * Create and fill the artist playlists for `videoPaths`.
 *
 * @param {object} data DataService (get, set, getPlaylists, addPlaylist, addVideosToPlaylist)
 * @param {string[]} videoPaths
 * @returns {Promise<{created: string[], extended: number}>} names of new playlists, count of grown ones
 */
export async function applyArtistPlaylists(data, videoPaths) {
  const sourcePaths = (data.get('library.sources') || []).map((s) => s?.path).filter(Boolean);
  const plan = planArtistPlaylists({ videoPaths, sourcePaths, playlists: data.getPlaylists(), state: data.get(ARTIST_PLAYLISTS_KEY) || {} });
  for (const c of plan.create) {
    const pl = await data.addPlaylist(c.name);
    data.addVideosToPlaylist(pl.id, c.videoPaths);
  }
  for (const e of plan.extend) data.addVideosToPlaylist(e.id, e.videoPaths);
  if (JSON.stringify(plan.state) !== JSON.stringify(data.get(ARTIST_PLAYLISTS_KEY) || {})) data.set(ARTIST_PLAYLISTS_KEY, plan.state);
  return { created: plan.create.map((c) => c.name), extended: plan.extend.length };
}
