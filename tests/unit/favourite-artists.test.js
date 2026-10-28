/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Favourite artists (renderer/js/favourite-artists.js): FAVOURITES with the
// user's changes, the favourite artist bonus, storing both, and what the
// backend registration carries.
import { describe, it, expect, vi } from 'vitest';
import {
  artistRegistration, effectiveFavourites, favouriteScore, favouriteScoreOf, favouritesOf, setFavouriteArtist, setFavouriteScore,
  FAVOURITE_ARTISTS_KEY, FAVOURITE_SCORE_KEY,
} from '../../renderer/js/favourite-artists.js';
import { FAVOURITES, RANKING } from '../../renderer/js/content-preferences.js';
import { scoutPreferences } from '../../scripts/scout-live.mjs';

const fakeData = (init = {}) => {
  const store = { ...init };
  return { store, get: (k) => store[k], set: vi.fn((k, v) => { store[k] = v; }) };
};

describe('effectiveFavourites', () => {
  it('starts from FAVOURITES', () => {
    expect(effectiveFavourites()).toEqual(FAVOURITES);
    expect(effectiveFavourites({})).toEqual(FAVOURITES);
  });

  it('drops a starting favourite set false and adds names set true, whatever their case or Unicode form', () => {
    const list = effectiveFavourites({ scyllahmv: false, '  JJULEZ ': false, 'Cafe\u0301 Girl': true, Shayuki: true, Nobody: false });
    expect(list).toEqual([...FAVOURITES.filter((f) => f !== 'ScyllaHMV' && f !== 'JJULEZ'), 'Cafe\u0301 Girl', 'Shayuki']);
  });

  it('keeps the starting spelling of a starting favourite set true, and ignores anything not a boolean', () => {
    expect(effectiveFavourites({ AGAPILORD: true, Shayuki: 'yes', '': true })).toEqual(FAVOURITES);
  });
});

describe('setFavouriteArtist', () => {
  it('stores only changes, under one spelling, and writes nothing when nothing changes', () => {
    const data = fakeData();
    expect(setFavouriteArtist(data, ' Shayuki ', true)).toEqual({ artist: 'Shayuki', favourite: true });
    setFavouriteArtist(data, 'scyllahmv', false);
    expect(data.store[FAVOURITE_ARTISTS_KEY]).toEqual({ Shayuki: true, scyllahmv: false });
    setFavouriteArtist(data, 'SHAYUKI', true);
    expect(data.store[FAVOURITE_ARTISTS_KEY]).toEqual({ scyllahmv: false, SHAYUKI: true });
    // Back to the start: re-adding a starting favourite and removing an added one leave no key.
    setFavouriteArtist(data, 'ScyllaHMV', true);
    setFavouriteArtist(data, 'shayuki', false);
    expect(data.store[FAVOURITE_ARTISTS_KEY]).toEqual({});
    expect(favouritesOf(data)).toEqual(FAVOURITES);
    const writes = data.set.mock.calls.length;
    setFavouriteArtist(data, 'Agapilord', true);
    setFavouriteArtist(data, 'Nobody', false);
    expect(data.set.mock.calls.length).toBe(writes);
  });

  it('matches names case- and Unicode-insensitively', () => {
    const data = fakeData({ [FAVOURITE_ARTISTS_KEY]: { 'Cafe\u0301 Girl': true } });
    setFavouriteArtist(data, 'CAFÉ GIRL', false);
    expect(data.store[FAVOURITE_ARTISTS_KEY]).toEqual({});
  });

  it('rejects an empty name or a favourite that is not a boolean', () => {
    const data = fakeData();
    expect(() => setFavouriteArtist(data, ' ', true)).toThrow(/without a name/);
    expect(() => setFavouriteArtist(data, null, true)).toThrow(/without a name/);
    expect(() => setFavouriteArtist(data, 'Shayuki', 'yes')).toThrow(/invalid favourite/);
    expect(data.set).not.toHaveBeenCalled();
  });
});

describe('favourite artist bonus', () => {
  it('is RANKING.favouriteArtist unless the user set one, stepped and clamped like a tag score', () => {
    expect(RANKING.favouriteArtist).toBe(5);
    expect([undefined, null, 'x', NaN].map(favouriteScore)).toEqual(Array(4).fill(RANKING.favouriteArtist));
    expect([0, 2.26, -1.3, 42, -42, '3'].map(favouriteScore)).toEqual([0, 2.5, -1.5, 10, -10, 3]);
  });

  it('stores the override, and clears it when set back to the default', () => {
    const data = fakeData();
    expect(favouriteScoreOf(data)).toBe(RANKING.favouriteArtist);
    expect(setFavouriteScore(data, 0)).toEqual({ score: 0 });
    expect(data.store[FAVOURITE_SCORE_KEY]).toBe(0);
    expect(favouriteScoreOf(data)).toBe(0);
    expect(setFavouriteScore(data, 99)).toEqual({ score: 10 });
    expect(setFavouriteScore(data, RANKING.favouriteArtist)).toEqual({ score: RANKING.favouriteArtist });
    expect(data.store[FAVOURITE_SCORE_KEY]).toBeNull();
    const writes = data.set.mock.calls.length;
    setFavouriteScore(fakeData(), RANKING.favouriteArtist);
    setFavouriteScore(data, RANKING.favouriteArtist);
    expect(data.set.mock.calls.length).toBe(writes);
    expect(() => setFavouriteScore(data, 'lots')).toThrow(/invalid/);
  });
});

describe('artistRegistration', () => {
  const SOURCES = ['/GVP/EroScripts', '/GVP/FapTap'];
  const data = fakeData({
    'library.sources': SOURCES.map((path) => ({ path })),
    [FAVOURITE_ARTISTS_KEY]: { shayuki: true, ScyllaHMV: false },
    [FAVOURITE_SCORE_KEY]: 3,
  });

  it('carries the favourites and the bonus, and every artist folder when given the videos', () => {
    const videos = [
      '/GVP/EroScripts/Misiv/A/A.mp4', '/GVP/FapTap/misiv/B/B.mp4', '/GVP/EroScripts/Misiv/C/C.mp4',
      '/GVP/EroScripts/Agapilord/D/D.mp4', '/GVP/EroScripts/Loose.mp4', '/GVP/EroScripts/Misc/E/E.mp4',
    ];
    const favourites = [...FAVOURITES.filter((f) => f !== 'ScyllaHMV'), 'shayuki'];
    expect(artistRegistration(data)).toEqual({ favouriteArtists: favourites, favouriteArtistScore: { score: 3, default: RANKING.favouriteArtist } });
    expect(artistRegistration(data, videos).artists).toEqual([
      { name: 'Agapilord', videoPaths: ['/GVP/EroScripts/Agapilord/D/D.mp4'] },
      { name: 'Misiv', videoPaths: ['/GVP/EroScripts/Misiv/A/A.mp4', '/GVP/FapTap/misiv/B/B.mp4', '/GVP/EroScripts/Misiv/C/C.mp4'] },
    ]);
  });

  it('gives the scouts the same favourites and bonus from config.json', () => {
    const config = { settings: { library: { favouriteArtists: { shayuki: true, ScyllaHMV: false }, favouriteArtistScore: 3, tagScores: { pmv: 1 } } } };
    expect(scoutPreferences(config)).toEqual({ favourites: favouritesOf(data), scoring: { tagScores: { pmv: 1 }, favouriteScore: 3 } });
    expect(scoutPreferences({})).toEqual({ favourites: FAVOURITES, scoring: { tagScores: {}, favouriteScore: RANKING.favouriteArtist } });
  });
});
