// Favourite artists and the favourite artist bonus.
//
// FAVOURITES in content-preferences.js is the starting set; the user adds and
// removes favourites in the web remote's Artists view, and
// `library.favouriteArtists` holds only those changes: `{name: true}` adds an
// artist, `{name: false}` removes a starting favourite. Names compare case-
// and Unicode-insensitively (NFC, lower case), as artist folders do
// (artist-playlists.js). The effective list replaces FAVOURITES everywhere:
// artist playlists and their cleanup, library scores, both scouts' screening,
// naming and ranking.
//
// A favourite's post or scene ranks RANKING.favouriteArtist higher, on top of
// artistInLibrary; `library.favouriteArtistScore` overrides that bonus (same
// range and steps as a tag score; null = the default). Changes arrive as
// library edits (library-edits.js). Pure and renderer-safe: no Node imports.

import { artistOf } from './artist-playlists.js';
import { FAVOURITES, RANKING } from './content-preferences.js';
import { normalizeTagScore } from './preference-score.js';

export const FAVOURITE_ARTISTS_KEY = 'library.favouriteArtists';
export const FAVOURITE_SCORE_KEY = 'library.favouriteArtistScore';

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();
const BUILT_IN = new Set(FAVOURITES.map(fold));

/**
 * The favourite artists: FAVOURITES not set false in `changes`, plus the
 * names set true, each spelled as built in or as stored.
 *
 * @param {Object<string, boolean>} [changes] `library.favouriteArtists`
 * @returns {string[]}
 */
export function effectiveFavourites(changes) {
  const out = new Map(FAVOURITES.map((name) => [fold(name), name]));
  if (changes && typeof changes === 'object') {
    for (const [name, favourite] of Object.entries(changes)) {
      const key = fold(name);
      if (!key) continue;
      if (favourite === false) out.delete(key);
      else if (favourite === true && !out.has(key)) out.set(key, name.trim());
    }
  }
  return [...out.values()];
}

/** The favourite artist bonus: `value` as a tag score, else RANKING.favouriteArtist. */
export function favouriteScore(value) {
  const score = normalizeTagScore(value);
  return score === null ? RANKING.favouriteArtist : score;
}

/** @param {object} data DataService (get) */
export function favouritesOf(data) {
  return effectiveFavourites(data.get(FAVOURITE_ARTISTS_KEY));
}

/** @param {object} data DataService (get) */
export function favouriteScoreOf(data) {
  return favouriteScore(data.get(FAVOURITE_SCORE_KEY));
}

/**
 * Make `name` a favourite or not. `library.favouriteArtists` keeps only the
 * change from FAVOURITES, under one spelling: any other key of the same name
 * goes, and a starting favourite set back to true (or another name set to
 * false) leaves no key. Returns `{artist, favourite}`; throws on an empty
 * name or a non-boolean `favourite`.
 *
 * @param {object} data DataService (get, set)
 * @param {string} name
 * @param {boolean} favourite
 */
export function setFavouriteArtist(data, name, favourite) {
  const artist = typeof name === 'string' ? name.trim() : '';
  const key = fold(artist);
  if (!key) throw new Error('favourite artist without a name');
  if (typeof favourite !== 'boolean') throw new Error(`invalid favourite ${JSON.stringify(favourite)}`);
  const current = data.get(FAVOURITE_ARTISTS_KEY) || {};
  const next = Object.fromEntries(Object.entries(current).filter(([n]) => fold(n) !== key));
  if (favourite !== BUILT_IN.has(key)) next[artist] = favourite;
  if (JSON.stringify(next) !== JSON.stringify(current)) data.set(FAVOURITE_ARTISTS_KEY, next);
  return { artist, favourite };
}

/**
 * Set the favourite artist bonus, stepped and clamped like a tag score; the
 * default (RANKING.favouriteArtist) clears the override (null). Returns
 * `{score}` as applied; throws when `score` isn't a number.
 *
 * @param {object} data DataService (get, set)
 * @param {number} score
 */
export function setFavouriteScore(data, score) {
  const value = normalizeTagScore(score);
  if (value === null) throw new Error(`invalid favourite artist score ${JSON.stringify(score)}`);
  const stored = value === RANKING.favouriteArtist ? null : value;
  if ((data.get(FAVOURITE_SCORE_KEY) ?? null) !== stored) data.set(FAVOURITE_SCORE_KEY, stored);
  return { score: value };
}

/**
 * The artist part of a backend registration (`/api/media/register`): the
 * favourite artists and the bonus (`{score, default}`), plus, when
 * `videoPaths` is given, every artist folder of the library with its videos
 * (`artists`, named by its most common spelling).
 *
 * @param {object} data DataService (get)
 * @param {string[]} [videoPaths] the library's videos
 */
export function artistRegistration(data, videoPaths) {
  const out = {
    favouriteArtists: favouritesOf(data),
    favouriteArtistScore: { score: favouriteScoreOf(data), default: RANKING.favouriteArtist },
  };
  if (!Array.isArray(videoPaths)) return out;
  const sourcePaths = (data.get('library.sources') || []).map((s) => s?.path).filter(Boolean);
  const groups = new Map();
  for (const p of videoPaths) {
    const artist = artistOf(p, sourcePaths);
    if (!artist) continue;
    const g = groups.get(fold(artist)) || { spellings: new Map(), videoPaths: [] };
    g.spellings.set(artist, (g.spellings.get(artist) || 0) + 1);
    g.videoPaths.push(p);
    groups.set(fold(artist), g);
  }
  out.artists = [...groups]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([, g]) => ({ name: [...g.spellings].sort((a, b) => b[1] - a[1])[0][0], videoPaths: g.videoPaths }));
  return out;
}
