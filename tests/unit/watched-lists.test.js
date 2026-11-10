// Watched videos (user, 2026-11-10): watched = rated, watched videos sink to
// the end of some lists (ratings.js, watched-lists.js).
import { describe, it, expect } from 'vitest';
import { isWatched, watchedByRecency, watchedLast } from '../../renderer/js/ratings.js';
import {
  LIST_ARTIST_PLAYLIST, LIST_CATEGORY, LIST_COLLECTION, LIST_HAND_PLAYLIST, LIST_LIBRARY, LIST_RATING, LIST_SMART_PLAYLIST, LIST_WATCHED,
  playlistKind, sinksWatched, WATCHED_LAST_KEY, watchedLastOn, watchedRegistration,
} from '../../renderer/js/watched-lists.js';
import { AGE_SCREEN, FAVOURITES_PLAYLIST, SMART_PLAYLISTS } from '../../renderer/js/content-preferences.js';

const ratings = {
  '/v/rated.mp4': { rating: 3, favorite: false, updatedAt: 10 },
  '/v/half.mp4': { rating: 0.5, favorite: true, updatedAt: 30 },
  '/v/fav.mp4': { rating: 0, favorite: true, updatedAt: 40 },
  '/v/new.mp4': { rating: 5, favorite: false, updatedAt: 20 },
  '/v/bad.mp4': { rating: 7, favorite: false, updatedAt: 50 },
};

describe('isWatched', () => {
  it('is any rating above 0', () => {
    expect(isWatched(ratings, '/v/rated.mp4')).toBe(true);
    expect(isWatched(ratings, '/v/half.mp4')).toBe(true);
  });
  it('is not a favourite flag alone, an invalid rating or no entry', () => {
    expect(isWatched(ratings, '/v/fav.mp4')).toBe(false);
    expect(isWatched(ratings, '/v/bad.mp4')).toBe(false);
    expect(isWatched(ratings, '/v/none.mp4')).toBe(false);
    expect(isWatched(null, '/v/rated.mp4')).toBe(false);
  });
});

describe('watchedLast', () => {
  const watched = (p) => isWatched(ratings, p);
  it('moves watched items to the end, keeping the order within each group', () => {
    const items = ['/v/new.mp4', '/v/a.mp4', '/v/fav.mp4', '/v/rated.mp4', '/v/b.mp4', '/v/half.mp4'].map((path) => ({ path }));
    expect(watchedLast(items, (v) => v.path, watched).map((v) => v.path)).toEqual([
      '/v/a.mp4', '/v/fav.mp4', '/v/b.mp4', '/v/new.mp4', '/v/rated.mp4', '/v/half.mp4',
    ]);
  });
  it('returns a new array and leaves the input alone', () => {
    const items = ['/v/rated.mp4', '/v/a.mp4'];
    const out = watchedLast(items, (p) => p, watched);
    expect(out).toEqual(['/v/a.mp4', '/v/rated.mp4']);
    expect(items).toEqual(['/v/rated.mp4', '/v/a.mp4']);
    const unwatched = ['/v/a.mp4'];
    expect(watchedLast(unwatched, (p) => p, watched)).not.toBe(unwatched);
  });
  it('handles empty and all-watched lists', () => {
    expect(watchedLast([], (p) => p, watched)).toEqual([]);
    expect(watchedLast(undefined, (p) => p, watched)).toEqual([]);
    expect(watchedLast(['/v/new.mp4', '/v/rated.mp4'], (p) => p, watched)).toEqual(['/v/new.mp4', '/v/rated.mp4']);
  });
});

describe('watchedByRecency', () => {
  it('lists the rated videos, newest rating first; favourites alone and invalid ratings stay out', () => {
    expect(watchedByRecency(ratings)).toEqual(['/v/half.mp4', '/v/new.mp4', '/v/rated.mp4']);
  });
  it('breaks ties by path and tolerates a missing map', () => {
    const map = { '/v/b.mp4': { rating: 1, updatedAt: 5 }, '/v/a.mp4': { rating: 2, updatedAt: 5 }, '/v/c.mp4': { rating: 2 } };
    expect(watchedByRecency(map)).toEqual(['/v/a.mp4', '/v/b.mp4', '/v/c.mp4']);
    expect(watchedByRecency(undefined)).toEqual([]);
  });
});

describe('where watched videos sink', () => {
  it('sinks in the library, categories and collections', () => {
    for (const kind of [LIST_LIBRARY, LIST_CATEGORY, LIST_COLLECTION]) expect(sinksWatched(kind)).toBe(true);
  });
  it('keeps the order of every playlist (artist, smart, rating, hand-made) and the Watched list', () => {
    for (const kind of [LIST_ARTIST_PLAYLIST, LIST_SMART_PLAYLIST, LIST_RATING, LIST_WATCHED, LIST_HAND_PLAYLIST, undefined]) {
      expect(sinksWatched(kind)).toBe(false);
    }
  });
  it('is on unless set to false', () => {
    expect(watchedLastOn(undefined)).toBe(true);
    expect(watchedLastOn(true)).toBe(true);
    expect(watchedLastOn(false)).toBe(false);
  });
});

describe('playlistKind', () => {
  const state = { created: ['scyllahmv', ':favourites'], added: {} };
  it('knows the artist playlists the app made, whatever the case, and Favourite artists', () => {
    expect(playlistKind({ name: 'ScyllaHMV' }, state)).toBe(LIST_ARTIST_PLAYLIST);
    expect(playlistKind({ name: ' scyllahmv ' }, state)).toBe(LIST_ARTIST_PLAYLIST);
    expect(playlistKind({ name: FAVOURITES_PLAYLIST }, {})).toBe(LIST_ARTIST_PLAYLIST);
  });
  it('knows the age screener review list as a rating list', () => {
    expect(playlistKind({ name: AGE_SCREEN.playlist }, state)).toBe(LIST_RATING);
  });
  it('knows the smart playlists, whatever the case', () => {
    for (const def of SMART_PLAYLISTS) expect(playlistKind({ name: def.name }, state)).toBe(LIST_SMART_PLAYLIST);
    expect(playlistKind({ name: ' vr PICKS ' })).toBe(LIST_SMART_PLAYLIST);
  });
  it('takes everything else as hand-made, a leftover Discovery or Review too', () => {
    for (const name of ['Keep Forever', 'Maybe later?', '12 Days of Scriptsmas', 'Agapilord', 'Discovery', 'Review', '']) {
      expect(playlistKind({ name }, state)).toBe(LIST_HAND_PLAYLIST);
    }
    expect(playlistKind(null)).toBe(LIST_HAND_PLAYLIST);
  });
});

describe('watchedRegistration', () => {
  const settings = (values) => ({ get: (key) => values[key] });
  it('sends the setting only (no playlist sinks)', () => {
    expect(watchedRegistration(settings({}))).toEqual({ watched: { sink: true } });
    expect(watchedRegistration(settings({ [WATCHED_LAST_KEY]: false }))).toEqual({ watched: { sink: false } });
  });
});
