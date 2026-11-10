/**
 * @vitest-environment jsdom
 * Web remote watched videos (backend/web-remote/watched.js): watched
 * (rated) videos go last in the lists that sink them, the rating views
 * keep their order, and the virtual Watched collection.
 */
import { describe, it, expect, afterEach } from 'vitest';
import {
  WATCHED_COLLECTION_ID, watchedLast, shouldSink, isWatchedCollection, localizeWatchedCollection,
  videosInOrder, createWatchedBadge,
} from '../../backend/web-remote/watched.js';
import { groupingDetailHash, parseGroupingDetailHash } from '../../backend/web-remote/grouping-routes.js';
import { _resetForTests, _setBundlesForTests } from '../../backend/web-remote/i18n.js';

const v = (id, watched, rating = watched ? 3 : 0) => ({ id, watched, rating });

describe('watchedLast', () => {
  it('moves watched videos to the end, each group in its order', () => {
    const list = [v('a', true), v('b', false), v('c', true), v('d', false), v('e', false)];
    expect(watchedLast(list).map((x) => x.id)).toEqual(['b', 'd', 'e', 'a', 'c']);
    expect(list.map((x) => x.id)).toEqual(['a', 'b', 'c', 'd', 'e']); // input untouched
  });

  it('keeps a search ranking within each group', () => {
    const ranked = [v('exact', true), v('prefix', false), v('fuzzy', true), v('word', false)];
    expect(watchedLast(ranked).map((x) => x.id)).toEqual(['prefix', 'word', 'exact', 'fuzzy']);
  });

  it('treats a missing flag as unwatched and copes with nothing', () => {
    expect(watchedLast([{ id: 'x' }, v('y', true)]).map((x) => x.id)).toEqual(['x', 'y']);
    expect(watchedLast(undefined)).toEqual([]);
  });
});

describe('shouldSink', () => {
  it('sinks when the list sinks and the view is no rating list', () => {
    expect(shouldSink({ sink: true, rating: 'all', sort: 'name:asc' })).toBe(true);
    expect(shouldSink({ sink: true, rating: 'unrated', sort: 'dateAdded:desc' })).toBe(true);
    expect(shouldSink({ sink: true })).toBe(true);
  });

  it('keeps the order when the setting / grouping says no', () => {
    expect(shouldSink({ sink: false, rating: 'all', sort: 'name:asc' })).toBe(false);
    expect(shouldSink({ sink: undefined, rating: 'all', sort: 'name:asc' })).toBe(false);
    expect(shouldSink()).toBe(false);
  });

  it.each([
    [{ rating: 'favorites', sort: 'name:asc' }],
    [{ rating: 'top', sort: 'name:asc' }],
    [{ rating: 'all', sort: 'rating:desc' }],
    [{ rating: 'all', sort: 'rating:asc' }],
  ])('keeps the order in rating views %o', (view) => {
    expect(shouldSink({ sink: true, ...view })).toBe(false);
  });
});

describe('the Watched collection', () => {
  const collections = [
    { id: 'c1', name: 'Gym', videoIds: ['a'] },
    { id: WATCHED_COLLECTION_ID, name: 'Watched', kind: 'watched', videoIds: ['c', 'a'] },
  ];

  it('is recognised by its kind', () => {
    expect(isWatchedCollection(collections[1])).toBe(true);
    expect(isWatchedCollection(collections[0])).toBe(false);
    expect(isWatchedCollection(null)).toBe(false);
  });

  it('gets its localized name; the others keep theirs', () => {
    const out = localizeWatchedCollection(collections, 'Gesehen');
    expect(out.map((c) => c.name)).toEqual(['Gym', 'Gesehen']);
    expect(collections[1].name).toBe('Watched'); // cached server copy untouched
    expect(localizeWatchedCollection(undefined, 'x')).toEqual([]);
  });

  it('lists its videos in its own order (newest rating first)', () => {
    const library = [v('a', true), v('b', false), v('c', true)];
    expect(videosInOrder(library, collections[1].videoIds).map((x) => x.id)).toEqual(['c', 'a']);
    expect(videosInOrder(library, ['gone', 'b']).map((x) => x.id)).toEqual(['b']);
  });

  it('has a hash route that survives the colon', () => {
    const hash = groupingDetailHash('collections', WATCHED_COLLECTION_ID);
    expect(hash).toBe('collection/%3Awatched');
    expect(parseGroupingDetailHash(hash)).toEqual({ kind: 'collections', id: ':watched' });
  });
});

describe('createWatchedBadge', () => {
  afterEach(() => _resetForTests());

  it('badges watched videos with the rating in its title', () => {
    const en = {
      webRemote: {
        triage: {
          watched: 'Watched',
          watchedTitle: 'Watched: {n, plural, one {# star} other {# stars}}',
        },
      },
    };
    _setBundlesForTests('en', en, en);
    const badge = createWatchedBadge({ watched: true, rating: 4.5 }, 'list__badge list__badge--watched');
    expect(badge.className).toBe('list__badge list__badge--watched');
    expect(badge.textContent).toBe('Watched');
    expect(badge.title).toBe('Watched: 4.5 stars');
    expect(badge.getAttribute('aria-label')).toBe('Watched: 4.5 stars');
    expect(createWatchedBadge({ watched: true, rating: 1 }, 'x').title).toBe('Watched: 1 star');
  });

  it('is null for unwatched videos', () => {
    expect(createWatchedBadge({ watched: false, rating: 0 }, 'x')).toBeNull();
    expect(createWatchedBadge(null, 'x')).toBeNull();
  });
});
