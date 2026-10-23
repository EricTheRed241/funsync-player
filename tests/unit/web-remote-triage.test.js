/**
 * @vitest-environment jsdom
 * Web remote triage (backend/web-remote/triage.js) and the rating sort in
 * its library-search; plus the desktop's shared star widget and the rating
 * sort / filter helpers it uses.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  ratingFromTap, starFill, filterByRating, ratingBody, playlistBody, postJson,
  setRating, addToPlaylist, removeFromPlaylist, createRatingControl, updateRatingControl,
} from '../../backend/web-remote/triage.js';
import { sortVideos as remoteSort } from '../../backend/web-remote/library-search.js';
import { sortVideos } from '../../renderer/js/library-search.js';
import { matchesRatingFilter } from '../../renderer/js/ratings.js';
import { ratingFromPointer, starFill as desktopStarFill, createRatingControl as desktopControl } from '../../renderer/js/rating-stars.js';

const VIDEOS = [
  { id: 'a', name: 'Alpha', path: '/a', rating: 3, favorite: false },
  { id: 'b', name: 'Bravo', path: '/b', rating: 0, favorite: true },
  { id: 'c', name: 'Charlie', path: '/c', rating: 4.5, favorite: true },
  { id: 'd', name: 'Delta', path: '/d' },
  { id: 'e', name: 'Echo', path: '/e', rating: 4.5 },
];

describe('star value from tap / pointer position', () => {
  it.each([
    [0, 0.2, 0, 0.5], [0, 0.8, 0, 1], [3, 0.49, 0, 3.5], [4, 0.9, 0, 5],
    [4, 0.1, 0, 4.5], [3, 0.3, 3.5, 0], [4, 0.9, 5, 0], [9, 0.9, 0, 5], [-1, 0.9, 0, 1],
  ])('star %i at %f with current %f → %f', (i, f, cur, out) => {
    expect(ratingFromTap(i, f, cur)).toBe(out);
    expect(ratingFromPointer(i, f, cur)).toBe(out);
  });

  it('fills full / half / empty', () => {
    expect([0, 1, 2, 3, 4].map((i) => starFill(i, 2.5))).toEqual(['full', 'full', 'half', 'empty', 'empty']);
    expect([0, 1, 2, 3, 4].map((i) => desktopStarFill(i, 5))).toEqual(Array(5).fill('full'));
    expect(desktopStarFill(0, 0)).toBe('empty');
  });
});

describe('rating sort', () => {
  it('web remote: highest first, unrated last, ties by name', () => {
    expect(remoteSort(VIDEOS, 'rating', 'desc').map((v) => v.id)).toEqual(['c', 'e', 'a', 'b', 'd']);
    expect(remoteSort(VIDEOS, 'rating', 'asc').map((v) => v.id)).toEqual(['a', 'c', 'e', 'b', 'd']);
  });

  it('desktop: same rule through the rating lookup', () => {
    const ratings = Object.fromEntries(VIDEOS.map((v) => [v.path, v.rating || 0]));
    const opts = { ratingOf: (p) => ratings[p] };
    expect(sortVideos(VIDEOS, 'rating', 'desc', opts).map((v) => v.id)).toEqual(['c', 'e', 'a', 'b', 'd']);
    expect(sortVideos(VIDEOS, 'rating', 'asc', opts).map((v) => v.id)).toEqual(['a', 'c', 'e', 'b', 'd']);
    // Without a lookup nothing is rated: order by name.
    expect(sortVideos(VIDEOS, 'rating', 'desc').map((v) => v.id)).toEqual(['a', 'b', 'c', 'd', 'e']);
  });
});

describe('rating filters', () => {
  it('web remote', () => {
    expect(filterByRating(VIDEOS, 'favorites').map((v) => v.id)).toEqual(['b', 'c']);
    expect(filterByRating(VIDEOS, 'top').map((v) => v.id)).toEqual(['c', 'e']);
    expect(filterByRating(VIDEOS, 'unrated').map((v) => v.id)).toEqual(['b', 'd']);
    expect(filterByRating(VIDEOS, 'all')).toHaveLength(5);
    expect(filterByRating(VIDEOS, undefined)).toHaveLength(5);
  });

  it('desktop', () => {
    const map = {
      '/a': { rating: 3 }, '/b': { favorite: true }, '/c': { rating: 4.5, favorite: true }, '/e': { rating: 4 },
    };
    const pick = (f) => VIDEOS.filter((v) => matchesRatingFilter(map, v.path, f)).map((v) => v.id);
    expect(pick('favorites')).toEqual(['b', 'c']);
    expect(pick('top')).toEqual(['c', 'e']);
    expect(pick('unrated')).toEqual(['b', 'd']);
    expect(pick('all')).toHaveLength(5);
  });
});

describe('request bodies + endpoints', () => {
  it('builds rating bodies with only the given fields', () => {
    expect(ratingBody('v1', { rating: 4.5 })).toEqual({ videoId: 'v1', rating: 4.5 });
    expect(ratingBody('v1', { favorite: 1 })).toEqual({ videoId: 'v1', favorite: true });
    expect(ratingBody('v1', { rating: 0, favorite: false })).toEqual({ videoId: 'v1', rating: 0, favorite: false });
  });

  it('builds playlist bodies by id and/or trimmed name', () => {
    expect(playlistBody('v1', { playlistId: 'pl-1', name: 'Evening' })).toEqual({ videoId: 'v1', playlistId: 'pl-1', name: 'Evening' });
    expect(playlistBody('v1', { name: '  Triage ' })).toEqual({ videoId: 'v1', name: 'Triage' });
    expect(playlistBody('v1', { name: '  ' })).toEqual({ videoId: 'v1' });
  });

  it('posts to the right endpoints', async () => {
    const fetchImpl = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ ok: true, delivered: false }) }));
    await setRating('v1', { rating: 4.5 }, fetchImpl);
    await addToPlaylist('v1', { name: 'Triage' }, fetchImpl);
    await removeFromPlaylist('v1', { playlistId: 'pl-1' }, fetchImpl);
    expect(fetchImpl.mock.calls.map(([url, init]) => [url, JSON.parse(init.body)])).toEqual([
      ['/api/remote/rating', { videoId: 'v1', rating: 4.5 }],
      ['/api/remote/playlists/add-video', { videoId: 'v1', name: 'Triage' }],
      ['/api/remote/playlists/remove-video', { videoId: 'v1', playlistId: 'pl-1' }],
    ]);
  });

  it('never throws: HTTP errors and network failures come back as results', async () => {
    const bad = await postJson('/x', {}, async () => ({ ok: false, status: 400, json: async () => ({ detail: 'invalid rating' }) }));
    expect(bad).toEqual({ ok: false, status: 400, data: { detail: 'invalid rating' } });
    const down = await postJson('/x', {}, async () => { throw new Error('offline'); });
    expect(down).toMatchObject({ ok: false, status: 0, data: { detail: 'offline' } });
  });
});

describe('rating controls', () => {
  it('web remote control reports taps and renders state', () => {
    const onRate = vi.fn();
    const onFavorite = vi.fn();
    const el = createRatingControl({ rating: 2.5, favorite: false, onRate, onFavorite });
    const stars = el.querySelectorAll('.triage-rating__star');
    expect([...stars].map((s) => s.dataset.fill)).toEqual(['full', 'full', 'half', 'empty', 'empty']);
    // Keyboard-style click (detail 0) counts as the whole star.
    stars[3].dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 0 }));
    expect(onRate).toHaveBeenCalledWith(4);
    el.querySelector('.triage-rating__heart').click();
    expect(onFavorite).toHaveBeenCalledWith(true);
    updateRatingControl(el, { rating: 4, favorite: true });
    expect(el.classList.contains('triage-rating--favorite')).toBe(true);
    expect(el.querySelector('.triage-rating__heart').getAttribute('aria-pressed')).toBe('true');
  });

  it('desktop control does not let clicks reach the card', () => {
    const cardClick = vi.fn();
    const card = document.createElement('div');
    card.addEventListener('click', cardClick);
    const onRate = vi.fn();
    const el = desktopControl({ rating: 4.5, favorite: true, onRate, onFavorite: vi.fn() });
    card.appendChild(el);
    // Keyboard activation of the 5th star = 5 (whole star).
    el.querySelectorAll('.rating-control__star')[4].dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 0 }));
    expect(onRate).toHaveBeenCalledWith(5);
    expect(cardClick).not.toHaveBeenCalled();
    expect(el.classList.contains('rating-control--favorite')).toBe(true);
  });
});
