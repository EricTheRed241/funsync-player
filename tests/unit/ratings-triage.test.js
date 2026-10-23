// Ratings + favourites (ratings.js), their storage through DataService, and
// the desktop side of the library-edit channel (library-edits.js).
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  normalizeRating, getRatingEntry, ratingOf, isFavorite, applyRatingChange, rehomeRatings,
} from '../../renderer/js/ratings.js';
import { applyLibraryEdit, findPlaylist, LibraryEditApplier } from '../../renderer/js/library-edits.js';
import { DataService } from '../../renderer/js/data-service.js';
import { eventBus } from '../../renderer/js/event-bus.js';

describe('normalizeRating', () => {
  it.each([[0, 0], [0.5, 0.5], [4.5, 4.5], [5, 5], ['3.5', 3.5], [' 2 ', 2]])('accepts %j', (v, out) => {
    expect(normalizeRating(v)).toBe(out);
  });
  it.each([[-0.5], [5.5], [4.3], [0.25], ['x'], [''], [null], [undefined], [true], [NaN], [Infinity], [{}]])(
    'rejects %j', (v) => {
      expect(normalizeRating(v)).toBeNull();
    },
  );
});

describe('rating entries', () => {
  const map = {
    '/a.mp4': { rating: 4.5, favorite: true, updatedAt: 3 },
    '/bad.mp4': { rating: 7, favorite: 'yes' },
  };
  it('reads normalised values', () => {
    expect(getRatingEntry(map, '/a.mp4')).toEqual({ rating: 4.5, favorite: true, updatedAt: 3 });
    expect(getRatingEntry(map, '/bad.mp4')).toEqual({ rating: 0, favorite: false, updatedAt: 0 });
    expect(getRatingEntry(map, '/none.mp4')).toBeNull();
    expect(ratingOf(map, '/a.mp4')).toBe(4.5);
    expect(ratingOf(map, '/none.mp4')).toBe(0);
    expect(isFavorite(map, '/a.mp4')).toBe(true);
    expect(isFavorite(null, '/a.mp4')).toBe(false);
  });

  it('applies partial changes without mutating the input', () => {
    const before = { '/a.mp4': { rating: 2, favorite: false, updatedAt: 1 } };
    const { map: next, entry } = applyRatingChange(before, '/a.mp4', { favorite: true }, 10);
    expect(entry).toEqual({ rating: 2, favorite: true, updatedAt: 10 });
    expect(before['/a.mp4'].favorite).toBe(false);
    expect(applyRatingChange(next, '/a.mp4', { rating: 3.5 }, 11).entry).toEqual({ rating: 3.5, favorite: true, updatedAt: 11 });
  });

  it('removes an entry that is unrated and not a favourite', () => {
    const { map: next, entry } = applyRatingChange({ '/a.mp4': { rating: 2, favorite: false } }, '/a.mp4', { rating: 0 });
    expect(entry).toBeNull();
    expect(next).toEqual({});
  });

  it('throws on invalid input', () => {
    expect(() => applyRatingChange({}, '/a.mp4', { rating: 4.3 })).toThrow(/invalid rating/);
    expect(() => applyRatingChange({}, '/a.mp4', {})).toThrow(/nothing to change/);
    expect(() => applyRatingChange({}, '', { rating: 1 })).toThrow(/path/);
  });

  it('follows a moved video, with the association rules', () => {
    const map = { '/old/a.mp4': { rating: 5, favorite: true }, '/old/dup.mp4': { rating: 1 }, '/x/dup.mp4': { rating: 2 } };
    const { map: next, moves } = rehomeRatings(map, ['/new/a.mp4', '/new/dup.mp4']);
    expect(moves).toEqual([{ from: '/old/a.mp4', to: '/new/a.mp4' }]); // dup is ambiguous
    expect(next['/new/a.mp4']).toEqual({ rating: 5, favorite: true });
    expect(next['/old/a.mp4']).toBeUndefined();
    expect(next['/old/dup.mp4']).toEqual({ rating: 1 });
  });

  it('leaves entries on an offline source alone', () => {
    const map = { '/drive/a.mp4': { rating: 5 } };
    const { moves } = rehomeRatings(map, ['/new/a.mp4'], (p) => p.startsWith('/drive'));
    expect(moves).toEqual([]);
  });
});

describe('DataService.setRating', () => {
  let ds;
  beforeEach(() => {
    window.funsync.setSetting.mockClear?.();
    ds = new DataService();
  });

  it('stores under library.ratings, persists via IPC and emits rating:changed', () => {
    const seen = [];
    const off = eventBus.on('rating:changed', (e) => seen.push(e));
    const entry = ds.setRating('/v/a.mp4', { rating: 4.5, favorite: true });
    expect(entry.rating).toBe(4.5);
    expect(ds.getRating('/v/a.mp4')).toMatchObject({ rating: 4.5, favorite: true });
    expect(ds.get('library.ratings')['/v/a.mp4']).toMatchObject({ rating: 4.5, favorite: true });
    expect(window.funsync.setSetting).toHaveBeenCalledWith('library.ratings', expect.any(Object));
    expect(seen).toEqual([{ path: '/v/a.mp4', entry }]);
    if (typeof off === 'function') off();
    else eventBus.off?.('rating:changed');
  });

  it('rejects an invalid rating without storing anything', () => {
    expect(() => ds.setRating('/v/a.mp4', { rating: 9 })).toThrow();
    expect(ds.getRatings()).toEqual({});
  });
});

function fakeData(playlists = []) {
  const data = {
    playlists,
    ratings: {},
    getPlaylists: () => data.playlists,
    addPlaylist: vi.fn(async (name) => {
      const pl = { id: `pl-${data.playlists.length + 1}`, name, videoPaths: [] };
      data.playlists.push(pl);
      return pl;
    }),
    addVideoToPlaylist: vi.fn((id, path) => {
      const pl = data.playlists.find((p) => p.id === id);
      if (!pl.videoPaths.includes(path)) pl.videoPaths.push(path);
    }),
    removeVideoFromPlaylist: vi.fn((id, path) => {
      const pl = data.playlists.find((p) => p.id === id);
      pl.videoPaths = pl.videoPaths.filter((p) => p !== path);
    }),
    setRating: vi.fn((path, change) => {
      data.ratings[path] = applyRatingChange(data.ratings, path, change).entry;
    }),
  };
  return data;
}

describe('applyLibraryEdit', () => {
  it('applies a rating edit with only the fields present', async () => {
    const data = fakeData();
    await applyLibraryEdit({ kind: 'rating', path: '/a.mp4', rating: 4.5, favorite: null }, data);
    expect(data.setRating).toHaveBeenCalledWith('/a.mp4', { rating: 4.5 });
    await applyLibraryEdit({ kind: 'rating', path: '/a.mp4', rating: null, favorite: true }, data);
    expect(data.setRating).toHaveBeenLastCalledWith('/a.mp4', { favorite: true });
  });

  it('adds to a playlist by id, by name, or creates it', async () => {
    const data = fakeData([{ id: 'pl-1', name: 'Evening', videoPaths: [] }]);
    await applyLibraryEdit({ kind: 'playlist-add', path: '/a.mp4', playlistId: 'pl-1', name: 'Evening' }, data);
    await applyLibraryEdit({ kind: 'playlist-add', path: '/b.mp4', playlistId: null, name: 'evening' }, data);
    await applyLibraryEdit({ kind: 'playlist-add', path: '/a.mp4', playlistId: 'pending:Triage', name: null }, data);
    expect(data.playlists.map((p) => [p.name, p.videoPaths])).toEqual([
      ['Evening', ['/a.mp4', '/b.mp4']],
      ['Triage', ['/a.mp4']],
    ]);
    // Replayed edit: idempotent.
    await applyLibraryEdit({ kind: 'playlist-add', path: '/a.mp4', playlistId: null, name: 'Triage' }, data);
    expect(data.addPlaylist).toHaveBeenCalledTimes(1);
    expect(data.playlists[1].videoPaths).toEqual(['/a.mp4']);
  });

  it('removes from a playlist; a missing one is a no-op', async () => {
    const data = fakeData([{ id: 'pl-1', name: 'Evening', videoPaths: ['/a.mp4'] }]);
    await applyLibraryEdit({ kind: 'playlist-remove', path: '/a.mp4', playlistId: null, name: 'EVENING' }, data);
    expect(data.playlists[0].videoPaths).toEqual([]);
    await applyLibraryEdit({ kind: 'playlist-remove', path: '/a.mp4', playlistId: 'nope', name: 'Nope' }, data);
    expect(data.removeVideoFromPlaylist).toHaveBeenCalledTimes(1);
  });

  it('rejects malformed edits', async () => {
    const data = fakeData();
    await expect(applyLibraryEdit({ kind: 'zap', path: '/a.mp4' }, data)).rejects.toThrow(/unknown/);
    await expect(applyLibraryEdit({ kind: 'rating', rating: 1 }, data)).rejects.toThrow(/path/);
    await expect(applyLibraryEdit({ kind: 'playlist-add', path: '/a.mp4', playlistId: 'x' }, data)).rejects.toThrow();
  });

  it('findPlaylist prefers id, then name', () => {
    const lists = [{ id: 'a', name: 'One' }, { id: 'b', name: 'Two' }];
    expect(findPlaylist(lists, 'b', 'One').id).toBe('b');
    expect(findPlaylist(lists, 'zz', 'one').id).toBe('a');
    expect(findPlaylist(lists, 'pending:TWO', null).id).toBe('b');
    expect(findPlaylist(lists, null, null)).toBeNull();
  });
});

describe('LibraryEditApplier', () => {
  it('applies in arrival order, publishes, then acks each', async () => {
    const data = fakeData();
    const log = [];
    // Make playlist creation slow: the next edit must still wait for it.
    data.addPlaylist = vi.fn(async (name) => {
      await new Promise((r) => setTimeout(r, 20));
      const pl = { id: 'pl-new', name, videoPaths: [] };
      data.playlists.push(pl);
      log.push(`create ${name}`);
      return pl;
    });
    const applier = new LibraryEditApplier({
      data,
      publish: async () => { log.push('publish'); },
      ack: (seq, error) => log.push(error ? `ack ${seq} ${error}` : `ack ${seq}`),
    });
    applier.apply({ seq: 1, kind: 'playlist-add', path: '/a.mp4', name: 'Triage' });
    applier.apply({ seq: 2, kind: 'playlist-add', path: '/b.mp4', name: 'triage' });
    applier.apply({ seq: 3, kind: 'nope', path: '/b.mp4' });
    await applier.apply({ seq: 4, kind: 'rating', path: '/a.mp4', rating: 4.5 });
    expect(log).toEqual([
      'create Triage', 'publish', 'ack 1',
      'publish', 'ack 2',
      'publish', 'ack 3 unknown edit kind "nope"',
      'publish', 'ack 4',
    ]);
    expect(data.playlists[0].videoPaths).toEqual(['/a.mp4', '/b.mp4']);
    expect(data.ratings['/a.mp4'].rating).toBe(4.5);
  });

  it('still acks when publishing fails', async () => {
    const ack = vi.fn();
    const applier = new LibraryEditApplier({
      data: fakeData(),
      publish: async () => { throw new Error('backend down'); },
      ack,
    });
    await applier.apply({ seq: 9, kind: 'rating', path: '/a.mp4', favorite: true });
    expect(ack).toHaveBeenCalledWith(9, undefined);
  });
});
