/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Smart playlists (renderer/js/smart-playlists.js): VR picks and Quick ones
// hold exactly the unwatched videos that qualify, Favorites every hearted
// video; best score first; made once, adopted by name (or a former name,
// then renamed), rewritten on every refresh.
import { describe, it, expect } from 'vitest';
import {
  applySmartPlaylists, isSmartPlaylistName, planSmartPlaylists, SMART_PLAYLISTS_KEY, smartPlaylistMembers,
} from '../../renderer/js/smart-playlists.js';
import { SMART_PLAYLISTS } from '../../renderer/js/content-preferences.js';

const FAV = () => ({ rating: 0, favorite: true });
const def = (key) => SMART_PLAYLISTS.find((d) => d.key === key);
const VR = def('vr-picks');
const QUICK = def('quick-ones');
const FAVS = def('favorites');

describe('SMART_PLAYLISTS', () => {
  it('are the three the user asked for, with their thresholds', () => {
    expect(SMART_PLAYLISTS.map((d) => d.name)).toEqual(['VR picks', 'Quick ones', 'Favorites']);
    expect(VR).toMatchObject({ vr: true, minScore: 15, order: 'score' });
    expect(QUICK).toMatchObject({ maxSeconds: 300, minScore: 15, order: 'score' });
    expect(FAVS).toMatchObject({ favorite: true, order: 'score', formerNames: ['Fresh downloads'] });
  });
});

describe('smartPlaylistMembers', () => {
  const v = (name) => ({ path: `/v/${name}.mp4` });
  const p = (name) => `/v/${name}.mp4`;

  it('VR picks: unwatched VR videos scored 15 or more, best first; 14.5 and flat stay out', () => {
    const videos = [v('a'), v('b'), v('c'), v('flat'), v('low'), v('none')];
    const scores = { [p('a')]: 15, [p('b')]: 18, [p('c')]: 15, [p('flat')]: 20, [p('low')]: 14.5 };
    const vr = new Set([p('a'), p('b'), p('c'), p('low'), p('none')]);
    const m = smartPlaylistMembers({ videos, scores, vrOf: (x) => vr.has(x) });
    expect(m.get('vr-picks')).toEqual([p('b'), p('a'), p('c')]);
  });

  it('Quick ones: known duration under 300 s (299 in, 300 and unknown out) and score 15+', () => {
    const videos = [v('s299'), v('s300'), v('unknown'), v('zero'), v('lowscore'), v('s60')];
    const scores = Object.fromEntries(videos.map((x) => [x.path, 16]));
    scores[p('lowscore')] = 14.5;
    scores[p('s60')] = 19;
    const durations = { [p('s299')]: 299, [p('s300')]: 300, [p('zero')]: 0, [p('lowscore')]: 100, [p('s60')]: 60 };
    const m = smartPlaylistMembers({ videos, scores, durations });
    expect(m.get('quick-ones')).toEqual([p('s60'), p('s299')]);
  });

  it('Favorites: every hearted video, watched or not, any score, best first', () => {
    const videos = [v('plain'), v('fav'), v('ratedfav'), v('lowfav'), v('rated')];
    const scores = { [p('fav')]: 10, [p('ratedfav')]: 12, [p('rated')]: 30 };
    const ratings = {
      [p('fav')]: FAV(), [p('ratedfav')]: { rating: 4, favorite: true }, [p('lowfav')]: FAV(), [p('rated')]: { rating: 5 },
    };
    const m = smartPlaylistMembers({ videos, scores, ratings });
    expect(m.get('favorites')).toEqual([p('ratedfav'), p('fav'), p('lowfav')]);
  });

  it('a rated video leaves VR picks and Quick ones; a favourite heart alone does not', () => {
    const videos = [v('rated'), v('fav'), v('free')];
    const scores = Object.fromEntries(videos.map((x) => [x.path, 20]));
    const durations = Object.fromEntries(videos.map((x) => [x.path, 120]));
    const ratings = { [p('rated')]: { rating: 3 }, [p('fav')]: FAV() };
    const m = smartPlaylistMembers({ videos, scores, durations, ratings, vrOf: () => true });
    for (const key of ['vr-picks', 'quick-ones']) expect(m.get(key)).toEqual([p('fav'), p('free')]);
  });

  it('breaks score ties by title, then path', () => {
    const videos = [v('b'), v('a'), v('c')];
    const scores = { [p('a')]: 16, [p('b')]: 16, [p('c')]: 17 };
    const m = smartPlaylistMembers({ videos, scores, vrOf: () => true });
    expect(m.get('vr-picks')).toEqual([p('c'), p('a'), p('b')]);
  });
});

describe('planSmartPlaylists', () => {
  const members = (vr = [], quick = [], favs = []) => new Map([['vr-picks', vr], ['quick-ones', quick], ['favorites', favs]]);

  it('makes a playlist once it would hold a video, and records it', () => {
    const r = planSmartPlaylists({ playlists: [], members: members(['/a.mp4']) });
    expect(r.create).toEqual([{ key: 'vr-picks', name: 'VR picks', videoPaths: ['/a.mp4'] }]);
    expect(r.state.created).toEqual(['vr-picks']);
  });

  it('takes over a playlist of the same name, whatever its case, and rewrites it to the members in order', () => {
    const playlists = [{ id: 'q', name: 'quick ONES', videoPaths: ['/x.mp4', '/b.mp4'] }];
    const r = planSmartPlaylists({ playlists, members: members([], ['/a.mp4', '/b.mp4']) });
    expect(r.create).toEqual([]);
    expect(r.update).toEqual([{ id: 'q', videoPaths: ['/a.mp4', '/b.mp4'] }]);
    expect(r.state.created).toEqual(['quick-ones']);
  });

  it('rewrites an order change, leaves an identical playlist alone, keeps an empty one', () => {
    const playlists = [
      { id: 'v', name: 'VR picks', videoPaths: ['/b.mp4', '/a.mp4'] },
      { id: 'q', name: 'Quick ones', videoPaths: ['/q.mp4'] },
      { id: 'f', name: 'Favorites', videoPaths: ['/f.mp4'] },
    ];
    const r = planSmartPlaylists({ playlists, members: members(['/a.mp4', '/b.mp4'], ['/q.mp4'], []), state: { created: ['vr-picks', 'quick-ones', 'favorites'] } });
    expect(r.update).toEqual([{ id: 'v', videoPaths: ['/a.mp4', '/b.mp4'] }, { id: 'f', videoPaths: [] }]);
    expect(r.rename).toEqual([]);
  });

  it('takes over "Fresh downloads" as Favorites: renamed and rewritten, not made a second time', () => {
    const playlists = [{ id: 'old', name: 'fresh DOWNLOADS', videoPaths: ['/new.mp4'] }];
    const r = planSmartPlaylists({ playlists, members: members([], [], ['/h.mp4']), state: { created: ['fresh-downloads'] } });
    expect(r.create).toEqual([]);
    expect(r.rename).toEqual([{ id: 'old', name: 'Favorites' }]);
    expect(r.update).toEqual([{ id: 'old', videoPaths: ['/h.mp4'] }]);
    expect(r.state.created).toEqual(['fresh-downloads', 'favorites']);
  });

  it('prefers a playlist already called Favorites over a former name', () => {
    const playlists = [{ id: 'old', name: 'Fresh downloads', videoPaths: [] }, { id: 'f', name: 'Favorites', videoPaths: ['/h.mp4'] }];
    const r = planSmartPlaylists({ playlists, members: members([], [], ['/h.mp4']) });
    expect(r).toMatchObject({ create: [], update: [], rename: [] });
  });

  it("doesn't make one again after the user deleted it", () => {
    const r = planSmartPlaylists({ playlists: [], members: members(['/a.mp4'], ['/a.mp4']), state: { created: ['vr-picks'] } });
    expect(r.create.map((c) => c.name)).toEqual(['Quick ones']);
    expect(r.state.created).toEqual(['vr-picks', 'quick-ones']);
  });

  it('makes nothing for an empty list that was never made', () => {
    const r = planSmartPlaylists({ playlists: [], members: members() });
    expect(r).toEqual({ create: [], update: [], rename: [], state: { created: [] } });
  });
});

describe('isSmartPlaylistName', () => {
  it('matches the smart playlist names only', () => {
    expect(isSmartPlaylistName(' favorites ')).toBe(true);
    expect(isSmartPlaylistName('Fresh downloads')).toBe(false);
    expect(isSmartPlaylistName('Discovery')).toBe(false);
    expect(isSmartPlaylistName('')).toBe(false);
  });
});

describe('applySmartPlaylists', () => {
  function data(playlists, store) {
    let n = 0;
    const calls = [];
    return {
      calls,
      get: (k) => store[k],
      set: (k, val) => { store[k] = val; },
      getRatings: () => store['library.ratings'] || {},
      getPlaylists: () => playlists,
      addPlaylist: async (name) => { const pl = { id: `new${++n}`, name, videoPaths: [] }; playlists.push(pl); return pl; },
      addVideosToPlaylist: (id, paths) => {
        const pl = playlists.find((x) => x.id === id);
        const fresh = paths.filter((x) => !pl.videoPaths.includes(x));
        if (fresh.length) { calls.push(['add', id, fresh]); pl.videoPaths.push(...fresh); }
      },
      removeVideoFromPlaylist: (id, x) => {
        calls.push(['remove', id, x]);
        const pl = playlists.find((y) => y.id === id);
        pl.videoPaths = pl.videoPaths.filter((y) => y !== x);
      },
      setPlaylistVideoPaths: (id, paths) => {
        calls.push(['order', id, paths]);
        playlists.find((x) => x.id === id).videoPaths = [...paths];
      },
      renamePlaylist: (id, name) => {
        calls.push(['rename', id, name]);
        playlists.find((x) => x.id === id).name = name;
      },
    };
  }

  it('makes, fills, then follows a rating and a score change; a deleted one stays gone', async () => {
    const videos = [{ path: '/a.mp4' }, { path: '/b.mp4' }];
    const store = {
      'library.preferenceScores': { '/a.mp4': 15, '/b.mp4': 16 },
      'library.durationCache': { '/a.mp4': 100, '/b.mp4': 400 },
      'library.ratings': { '/a.mp4': FAV() },
    };
    const playlists = [];
    const d = data(playlists, store);
    const vrOf = () => true;
    const first = await applySmartPlaylists(d, videos, { vrOf });
    expect(first).toEqual({ created: ['VR picks', 'Quick ones', 'Favorites'], updated: 0 });
    expect(playlists.map((pl) => [pl.name, pl.videoPaths])).toEqual([
      ['VR picks', ['/b.mp4', '/a.mp4']], ['Quick ones', ['/a.mp4']], ['Favorites', ['/a.mp4']],
    ]);
    expect(store[SMART_PLAYLISTS_KEY]).toEqual({ created: ['vr-picks', 'quick-ones', 'favorites'] });

    // /a.mp4 gets rated: it leaves VR picks and Quick ones (empty, kept), stays a favourite.
    store['library.ratings'] = { '/a.mp4': { rating: 4, favorite: true } };
    d.calls.length = 0;
    expect(await applySmartPlaylists(d, videos, { vrOf })).toEqual({ created: [], updated: 2 });
    expect(playlists.map((pl) => pl.videoPaths)).toEqual([['/b.mp4'], [], ['/a.mp4']]);
    expect(d.calls.every((c) => c[0] === 'remove' && c[2] === '/a.mp4')).toBe(true);

    // A new video scored above /b.mp4 goes first: added, then reordered.
    videos.push({ path: '/c.mp4' });
    store['library.preferenceScores']['/c.mp4'] = 19;
    d.calls.length = 0;
    await applySmartPlaylists(d, videos, { vrOf });
    expect(playlists[0].videoPaths).toEqual(['/c.mp4', '/b.mp4']);
    expect(d.calls).toEqual([['add', 'new1', ['/c.mp4']], ['order', 'new1', ['/c.mp4', '/b.mp4']]]);

    // Nothing changed: no writes.
    d.calls.length = 0;
    expect(await applySmartPlaylists(d, videos, { vrOf })).toEqual({ created: [], updated: 0 });
    expect(d.calls).toEqual([]);

    // The user deletes VR picks: it isn't made again.
    playlists.splice(0, 1);
    expect((await applySmartPlaylists(d, videos, { vrOf })).created).toEqual([]);
    expect(playlists.map((pl) => pl.name)).toEqual(['Quick ones', 'Favorites']);
  });

  it('turns an existing Fresh downloads playlist into Favorites', async () => {
    const playlists = [{ id: 'old', name: 'Fresh downloads', videoPaths: ['/b.mp4'] }];
    const store = { 'library.ratings': { '/a.mp4': FAV() }, [SMART_PLAYLISTS_KEY]: { created: ['fresh-downloads'] } };
    const d = data(playlists, store);
    const r = await applySmartPlaylists(d, [{ path: '/a.mp4' }, { path: '/b.mp4' }], { vrOf: () => false });
    expect(r).toEqual({ created: [], updated: 1 });
    expect(playlists).toEqual([{ id: 'old', name: 'Favorites', videoPaths: ['/a.mp4'] }]);
    expect(d.calls[0]).toEqual(['rename', 'old', 'Favorites']);
  });
});
