/**
 * @vitest-environment jsdom
 * Web remote favourite artists (backend/web-remote/artists.js): the server
 * state, the order, stars still on their way, what the desktop has applied,
 * waiting for it, and the star on each artist row.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  foldArtist, parseArtists, sortArtists, mergeArtistEdits, appliedArtistEdits, artistsPending,
  fetchArtists, setFavouriteArtist, setFavouriteScore, pollArtists, createFavouriteStar, updateFavouriteStar,
} from '../../backend/web-remote/artists.js';

const jsonResponse = (data, status = 200) => ({ ok: status < 400, status, json: async () => data });
const artist = (name, extra = {}) => ({
  id: foldArtist(name), name, videoCount: 1, totalDuration: 60, videoIds: ['v1'], favourite: false, pending: false, ...extra,
});

describe('artist names', () => {
  it('compare by NFC, trimmed, lower case', () => {
    expect(foldArtist('  Café ')).toBe(foldArtist('cafe\u0301'.replace('cafe', 'CAFE')));
    expect(foldArtist(null)).toBe('');
  });
});

describe('server state', () => {
  it('fills in defaults, drops nameless entries and keeps the bonus numbers', () => {
    const s = parseArtists({
      artists: [
        { id: 'alpha', name: 'Alpha', videoCount: 2, totalDuration: 300, videoIds: ['a', 3, 'b'], favourite: true, pending: true },
        { name: '  Beta  ', videoIds: ['x'] },
        { name: '' }, { id: 'x' }, null,
      ],
      score: 7.5, defaultScore: 5, scorePending: true,
    });
    expect(s.artists).toEqual([
      { id: 'alpha', name: 'Alpha', videoCount: 2, totalDuration: 300, videoIds: ['a', 'b'], favourite: true, pending: true },
      { id: 'beta', name: 'Beta', videoCount: 1, totalDuration: 0, videoIds: ['x'], favourite: false, pending: false },
    ]);
    expect([s.score, s.defaultScore, s.scorePending]).toEqual([7.5, 5, true]);
  });

  it('reads an unset bonus as its default, and nothing as empty', () => {
    expect(parseArtists({ defaultScore: 5, score: 'x' }).score).toBe(5);
    expect(parseArtists(null)).toEqual({ artists: [], score: 0, defaultScore: 0, scorePending: false });
  });

  it('GETs the artists and POSTs a star or the bonus', async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ artists: [artist('Alpha')], score: 5, defaultScore: 5 }));
    expect((await fetchArtists(fetchImpl)).artists.map((a) => a.name)).toEqual(['Alpha']);
    expect(fetchImpl).toHaveBeenLastCalledWith('/api/remote/artists');
    await setFavouriteArtist('Alpha', true, fetchImpl);
    expect(fetchImpl.mock.calls.at(-1)[0]).toBe('/api/remote/favourite-artist');
    expect(JSON.parse(fetchImpl.mock.calls.at(-1)[1].body)).toEqual({ artist: 'Alpha', favourite: true });
    await setFavouriteScore(2.5, fetchImpl);
    expect(fetchImpl.mock.calls.at(-1)[0]).toBe('/api/remote/favourite-score');
    expect(JSON.parse(fetchImpl.mock.calls.at(-1)[1].body)).toEqual({ score: 2.5 });
    await expect(fetchArtists(async () => jsonResponse({}, 500))).rejects.toThrow(/500/);
  });

  it('polls until the desktop is done, past failed fetches, or gives up', async () => {
    const states = [new Error('down'), { artists: [artist('A', { pending: true })] }, { artists: [artist('A')], scorePending: false }];
    const fetchImpl = vi.fn(async () => {
      const next = states.shift();
      if (next instanceof Error) throw next;
      return jsonResponse(next);
    });
    const done = await pollArtists((s) => !artistsPending(s), { fetchImpl, sleep: async () => {}, timeoutMs: 1e9 });
    expect(done.artists[0].pending).toBe(false);
    expect(fetchImpl).toHaveBeenCalledTimes(3);

    let clock = 0;
    const never = await pollArtists(() => false, {
      fetchImpl: async () => jsonResponse({ scorePending: true }), sleep: async () => { clock += 1000; }, now: () => clock, timeoutMs: 3000,
    });
    expect(never).toBeNull();
  });
});

describe('order and stars on their way', () => {
  it('lists favourites first, then by name regardless of case', () => {
    const sorted = sortArtists([artist('zed'), artist('Bob', { favourite: true }), artist('alice'), artist('Amy', { favourite: true })]);
    expect(sorted.map((a) => a.name)).toEqual(['Amy', 'Bob', 'alice', 'zed']);
  });

  it('applies unanswered stars as pending and adds a new favourite without videos', () => {
    const artists = [artist('Alpha', { favourite: true }), artist('Beta')];
    const unsent = new Map([
      ['alpha', { name: 'Alpha', favourite: false }],
      ['beta', { name: 'BETA', favourite: true }],
      ['gamma', { name: 'Gamma', favourite: true }],
      ['delta', { name: 'Delta', favourite: false }],
    ]);
    const shown = mergeArtistEdits(artists, unsent);
    expect(shown.map((a) => [a.name, a.favourite, a.pending])).toEqual([
      ['Beta', true, true], ['Gamma', true, true], ['Alpha', false, true],
    ]);
    expect(shown[1]).toMatchObject({ id: 'gamma', videoCount: 0, videoIds: [] });
    expect(artists[0].favourite).toBe(true);
    expect(mergeArtistEdits(artists).map((a) => a.name)).toEqual(['Alpha', 'Beta']);
  });

  it('names what the desktop has applied since the last state', () => {
    const prev = { artists: [artist('Alpha', { pending: true }), artist('Beta', { pending: true })], scorePending: true };
    const next = { artists: [artist('alpha'), artist('Beta', { pending: true })], scorePending: false };
    expect(appliedArtistEdits(prev, next)).toEqual({ artists: ['alpha'], score: true });
    expect(appliedArtistEdits(next, next)).toEqual({ artists: [], score: false });
    expect([artistsPending(prev), artistsPending({ artists: [artist('A')], scorePending: false })]).toEqual([true, false]);
  });
});

describe('favourite star', () => {
  it('toggles on a tap, never reaches the row, and shows pending', () => {
    const onToggle = vi.fn();
    const rowClick = vi.fn();
    const row = document.createElement('div');
    row.addEventListener('click', rowClick);
    const star = createFavouriteStar({ artist: 'Alpha', favourite: false, onToggle });
    row.appendChild(star);
    expect(star.getAttribute('aria-pressed')).toBe('false');
    star.click();
    expect(onToggle).toHaveBeenLastCalledWith(true);
    expect(rowClick).not.toHaveBeenCalled();

    updateFavouriteStar(star, { favourite: true, pending: true });
    expect(star.getAttribute('aria-pressed')).toBe('true');
    expect(star.classList.contains('artist-star--on')).toBe(true);
    expect(star.classList.contains('artist-star--pending')).toBe(true);
    star.click();
    expect(onToggle).toHaveBeenLastCalledWith(false);
  });
});
