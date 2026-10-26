/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// One playlist per artist with more than six videos
// (renderer/js/artist-playlists.js): who counts, when a playlist is made,
// and that the user's own edits stick.
import { describe, it, expect } from 'vitest';
import { artistOf, planArtistPlaylists, applyArtistPlaylists, ARTIST_PLAYLISTS_KEY, MIN_ARTIST_VIDEOS } from '../../renderer/js/artist-playlists.js';

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
  });
});
