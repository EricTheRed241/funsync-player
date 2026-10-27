/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// One playlist per artist with more than six videos, per favourite from one
// video, and the combined Favourite artists playlist
// (renderer/js/artist-playlists.js): who counts, when a playlist is made,
// and that the user's own edits stick.
import { describe, it, expect } from 'vitest';
import {
  artistOf, planArtistPlaylists, applyArtistPlaylists, ARTIST_PLAYLISTS_KEY, FAVOURITES_STATE_KEY,
} from '../../renderer/js/artist-playlists.js';
import { MIN_ARTIST_VIDEOS, FAVOURITES_PLAYLIST } from '../../renderer/js/content-preferences.js';

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
  it('makes a playlist only past six videos, counting every source and spelling', () => {
    const videoPaths = [
      ...vids('/GVP/Multi-Axis', 'Misiv', 4), ...vids('/GVP/_incoming', 'misiv', 3, 5),
      ...vids('/GVP/Multi Generated', 'Eternal Enigma', MIN_ARTIST_VIDEOS - 1),
    ];
    const r = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES });
    expect(r.create.map((c) => [c.name, c.videoPaths.length])).toEqual([['Misiv', 7]]);
    expect(r.create[0].videoPaths.map((p) => p.split('/').pop())).toEqual([1, 2, 3, 4, 5, 6, 7].map((n) => `Scene ${n}.mp4`));
  });

  it('adds new videos to an existing playlist of that name, not ones taken out by hand', () => {
    const videoPaths = vids('/GVP/Multi Generated', 'ScyllaHMV', 9);
    const first = planArtistPlaylists({ videoPaths: videoPaths.slice(0, 8), sourcePaths: SOURCES, playlists: [{ id: 'p', name: 'scyllahmv', videoPaths: [] }] });
    expect(first.extend).toEqual([{ id: 'p', videoPaths: videoPaths.slice(0, 8) }]);

    // The user took Scene 1 out; Scene 9 is new.
    const kept = videoPaths.slice(1, 8);
    const next = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, playlists: [{ id: 'p', name: 'ScyllaHMV', videoPaths: kept }], state: first.state });
    expect(next.extend).toEqual([{ id: 'p', videoPaths: [videoPaths[8]] }]);
  });

  it('never makes a deleted playlist again', () => {
    const videoPaths = vids('/GVP/Multi Generated', 'Agapilord', 8);
    const first = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES });
    const again = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, playlists: [], state: first.state });
    expect(again).toMatchObject({ create: [], extend: [] });
  });

  it('gives a favourite a playlist from one video, matched whatever the case or Unicode form; others still need seven', () => {
    const videoPaths = [
      ...vids('/GVP/Multi Generated', 'Eternal Enigma', 1),
      ...vids('/GVP/_incoming', 'Cafe\u0301 Girl', 2),
      ...vids('/GVP/Multi-Axis', 'Shayuki', MIN_ARTIST_VIDEOS - 1),
    ];
    const r = planArtistPlaylists({ videoPaths, sourcePaths: SOURCES, favourites: ['eternal enigma', 'Café girl', 'Not Here'] });
    expect(r.create.map((c) => [c.name, c.videoPaths.length])).toEqual([[FAVOURITES_PLAYLIST, 3], ['Cafe\u0301 Girl', 2], ['Eternal Enigma', 1]]);
  });

  it('puts every favourite\'s videos in Favourite artists, grouped by artist, and adds new ones once', () => {
    const agapi = vids('/GVP/Multi Generated', 'Agapilord', 3);
    const scylla = vids('/GVP/Multi-Axis', 'ScyllaHMV', 2);
    const other = vids('/GVP/Multi-Axis', 'Misiv', 8);
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
});

describe('applyArtistPlaylists', () => {
  it('creates and fills the playlists, and a second run changes nothing', async () => {
    const store = { 'library.sources': SOURCES.map((path) => ({ path })) };
    const playlists = [];
    const data = {
      get: (k) => store[k],
      set: (k, v) => { store[k] = v; },
      getPlaylists: () => playlists,
      addPlaylist: async (name) => { const pl = { id: `id${playlists.length}`, name, videoPaths: [] }; playlists.push(pl); return pl; },
      addVideosToPlaylist: (id, paths) => { playlists.find((p) => p.id === id).videoPaths.push(...paths); },
    };
    const videoPaths = [...vids('/GVP/Multi Generated', 'Cuddle Mocap', 12), ...vids('/GVP/Multi-Axis', 'Shayuki', 4)];
    expect(await applyArtistPlaylists(data, videoPaths)).toEqual({ created: ['Cuddle Mocap'], extended: 0 });
    expect(playlists[0].videoPaths).toHaveLength(12);
    expect(store[ARTIST_PLAYLISTS_KEY].created).toEqual(['cuddle mocap']);
    expect(await applyArtistPlaylists(data, videoPaths)).toEqual({ created: [], extended: 0 });
    expect(await applyArtistPlaylists(data, videoPaths, ['Shayuki'])).toEqual({ created: [FAVOURITES_PLAYLIST, 'Shayuki'], extended: 0 });
    expect(playlists.map((p) => [p.name, p.videoPaths.length])).toEqual([['Cuddle Mocap', 12], [FAVOURITES_PLAYLIST, 4], ['Shayuki', 4]]);
    expect(await applyArtistPlaylists(data, videoPaths)).toEqual({ created: [], extended: 0 });
  });
});
