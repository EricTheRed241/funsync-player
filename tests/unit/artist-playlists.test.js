/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// One playlist per artist with at least ARTIST_PLAYLIST_MIN videos, per favourite from one
// video, and the combined Favourite artists playlist
// (renderer/js/artist-playlists.js): who counts, when a playlist is made,
// and that the user's own edits stick.
import { describe, it, expect } from 'vitest';
import {
  artistOf, planArtistPlaylists, applyArtistPlaylists, ARTIST_PLAYLISTS_KEY, FAVOURITES_STATE_KEY,
} from '../../renderer/js/artist-playlists.js';
import { ARTIST_PLAYLIST_MIN, FAVOURITES_PLAYLIST } from '../../renderer/js/content-preferences.js';

const SOURCES = ['/GVP/Multi-Axis', '/GVP/Multi Generated/', '/GVP/_incoming'];
const vids = (root, artist, n, from = 1) => Array.from({ length: n }, (_, i) => `${root}/${artist}/Scene ${i + from}/Scene ${i + from}.mp4`);

describe('artistOf', () => {
  it('is the first folder under the source, only for <artist>/<scene>/<video>', () => {
    expect(artistOf('/GVP/Multi Generated/Agapilord/Scene/Scene.mp4', SOURCES)).toBe('Agapilord');
    expect(artistOf('/GVP/_incoming/Scene/Scene.mp4', SOURCES)).toBe(null);
    expect(artistOf('/GVP/Multi Generated/Unknown/Scene/Scene.mp4', SOURCES)).toBe(null);
    expect(artistOf('/Elsewhere/Agapilord/Scene/Scene.mp4', SOURCES)).toBe(null);
  });
});

describe('planArtistPlaylists', () => {
  it('makes a playlist only from ARTIST_PLAYLIST_MIN videos, counting every source and spelling', () => {
    const videoPaths = [
      ...vids('/GVP/Multi-Axis', 'Misiv', ARTIST_PLAYLIST_MIN - 3), ...vids('/GVP/_incoming', 'misiv', 3, ARTIST_PLAYLIST_MIN - 2),
      ...vids('/GVP/Multi Generated', 'Eternal Enigma', ARTIST_PLAYLIST_MIN - 1),
    ];
    const r = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES });
    expect(r.create.map((c) => [c.name, c.videoPaths.length])).toEqual([['Misiv', ARTIST_PLAYLIST_MIN]]);
    expect(r.create[0].videoPaths.map((p) => p.split('/').pop())).toEqual(Array.from({ length: ARTIST_PLAYLIST_MIN }, (_, i) => `Scene ${i + 1}.mp4`));
  });

  it('adds new videos to an existing playlist of that name, not ones taken out by hand', () => {
    const n = ARTIST_PLAYLIST_MIN + 1;
    const videoPaths = vids('/GVP/Multi Generated', 'ScyllaHMV', n);
    const first = planArtistPlaylists({ videoPaths: videoPaths.slice(0, n - 1), sourcePaths: SOURCES, playlists: [{ id: 'p', name: 'scyllahmv', videoPaths: [] }] });
    expect(first.extend).toEqual([{ id: 'p', videoPaths: videoPaths.slice(0, n - 1) }]);

    // The user took Scene 1 out; the last scene is new.
    const kept = videoPaths.slice(1, n - 1);
    const next = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, playlists: [{ id: 'p', name: 'ScyllaHMV', videoPaths: kept }], state: first.state });
    expect(next.extend).toEqual([{ id: 'p', videoPaths: [videoPaths[n - 1]] }]);
  });

  it('never makes a deleted playlist again', () => {
    const videoPaths = vids('/GVP/Multi Generated', 'Agapilord', ARTIST_PLAYLIST_MIN);
    const first = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES });
    const again = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, playlists: [], state: first.state });
    expect(again).toMatchObject({ create: [], extend: [] });
  });

  it('gives a favourite its own playlist only from ARTIST_PLAYLIST_MIN, but Favourite artists from one video, matched whatever the case or Unicode form', () => {
    const videoPaths = [
      ...vids('/GVP/Multi Generated', 'Eternal Enigma', ARTIST_PLAYLIST_MIN - 1),
      ...vids('/GVP/_incoming', 'Cafe\u0301 Girl', ARTIST_PLAYLIST_MIN),
    ];
    const r = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, favourites: ['eternal enigma', 'Café girl', 'Not Here'] });
    expect(r.create.map((c) => [c.name, c.videoPaths.length])).toEqual([[FAVOURITES_PLAYLIST, 2 * ARTIST_PLAYLIST_MIN - 1], ['Cafe\u0301 Girl', ARTIST_PLAYLIST_MIN]]);
  });

  it('puts every favourite\'s videos in Favourite artists, grouped by artist, and adds new ones once', () => {
    const agapi = vids('/GVP/Multi Generated', 'Agapilord', 3);
    const scylla = vids('/GVP/Multi-Axis', 'ScyllaHMV', 2);
    const other = vids('/GVP/Multi-Axis', 'Misiv', ARTIST_PLAYLIST_MIN);
    const favourites = ['ScyllaHMV', 'Agapilord'];
    const first = planArtistPlaylists({ videoPaths: [...scylla, ...other, ...agapi.slice(0, 2)], sourcePaths: SOURCES, favourites });
    expect(first.create[0]).toEqual({ name: FAVOURITES_PLAYLIST, videoPaths: [...agapi.slice(0, 2), ...scylla] });
    expect(first.state.created).toContain(FAVOURITES_STATE_KEY);

    // The user took the first Agapilord video out; a third one is new.
    const playlists = [{ id: 'f', name: 'favourite ARTISTS', videoPaths: [agapi[1], ...scylla] }];
    const next = planArtistPlaylists({ videoPaths: [...scylla, ...other, ...agapi], sourcePaths: SOURCES, favourites, playlists, state: first.state });
    expect(next.extend).toEqual([{ id: 'f', videoPaths: [agapi[2]] }]);

    // Deleted: not made again.
    const gone = planArtistPlaylists({ videoPaths: [...scylla, ...agapi], sourcePaths: SOURCES, favourites, state: next.state });
    expect(gone.create.map((c) => c.name)).not.toContain(FAVOURITES_PLAYLIST);
  });

  it('takes a former favourite\'s videos out of Favourite artists and puts them back when favourited again', () => {
    const agapi = vids('/GVP/Multi Generated', 'Agapilord', 2);
    const scylla = vids('/GVP/Multi-Axis', 'ScyllaHMV', 2);
    const videoPaths = [...agapi, ...scylla];
    const first = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, favourites: ['Agapilord', 'ScyllaHMV'] });
    const playlists = [{ id: 'f', name: FAVOURITES_PLAYLIST, videoPaths: [...agapi, ...scylla] }];

    const dropped = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, favourites: ['ScyllaHMV'], playlists, state: first.state });
    expect(dropped.withdraw).toEqual([{ id: 'f', videoPaths: agapi }]);
    expect(dropped.extend).toEqual([]);

    const after = [{ id: 'f', name: FAVOURITES_PLAYLIST, videoPaths: scylla }];
    const again = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, favourites: ['Agapilord', 'ScyllaHMV'], playlists: after, state: dropped.state });
    expect(again.extend).toEqual([{ id: 'f', videoPaths: agapi }]);
    expect(again.withdraw).toEqual([]);
  });
});

describe('applyArtistPlaylists', () => {
  it('creates and fills the playlists, a second run changes nothing, and a former favourite leaves Favourite artists', async () => {
    const store = { 'library.sources': SOURCES.map((path) => ({ path })) };
    const playlists = [];
    const data = {
      get: (k) => store[k],
      set: (k, v) => { store[k] = v; },
      getPlaylists: () => playlists,
      addPlaylist: async (name) => { const pl = { id: `id${playlists.length}`, name, videoPaths: [] }; playlists.push(pl); return pl; },
      addVideosToPlaylist: (id, paths) => { playlists.find((p) => p.id === id).videoPaths.push(...paths); },
      removeVideoFromPlaylist: (id, path) => { const pl = playlists.find((p) => p.id === id); pl.videoPaths = pl.videoPaths.filter((p) => p !== path); },
    };
    const videoPaths = [...vids('/GVP/Multi Generated', 'Cuddle Mocap', ARTIST_PLAYLIST_MIN + 2), ...vids('/GVP/Multi-Axis', 'Shayuki', 4)];
    expect(await applyArtistPlaylists(data, videoPaths)).toEqual({ created: ['Cuddle Mocap'], extended: 0, withdrawn: 0 });
    expect(playlists[0].videoPaths).toHaveLength(ARTIST_PLAYLIST_MIN + 2);
    expect(store[ARTIST_PLAYLISTS_KEY].created).toEqual(['cuddle mocap']);
    expect(await applyArtistPlaylists(data, videoPaths)).toEqual({ created: [], extended: 0, withdrawn: 0 });
    expect(await applyArtistPlaylists(data, videoPaths, ['Shayuki'])).toEqual({ created: [FAVOURITES_PLAYLIST], extended: 0, withdrawn: 0 });
    expect(playlists.map((p) => [p.name, p.videoPaths.length])).toEqual([['Cuddle Mocap', ARTIST_PLAYLIST_MIN + 2], [FAVOURITES_PLAYLIST, 4]]);
    expect(await applyArtistPlaylists(data, videoPaths)).toEqual({ created: [], extended: 0, withdrawn: 4 });
    expect(playlists.map((p) => [p.name, p.videoPaths.length])).toEqual([['Cuddle Mocap', ARTIST_PLAYLIST_MIN + 2], [FAVOURITES_PLAYLIST, 0]]);
    expect(await applyArtistPlaylists(data, videoPaths)).toEqual({ created: [], extended: 0, withdrawn: 0 });
  });
});
