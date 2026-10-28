/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Playlist cleanup after a scan (renderer/js/playlist-cleanup.js): empty
// playlists go, app-made artist playlists below the threshold go and can come
// back, the user's small playlists stay.
import { describe, it, expect } from 'vitest';
import { planPlaylistCleanup } from '../../renderer/js/playlist-cleanup.js';
import { planArtistPlaylists } from '../../renderer/js/artist-playlists.js';
import { FAVOURITES_PLAYLIST, MIN_ARTIST_VIDEOS } from '../../renderer/js/content-preferences.js';

const vids = (artist, n, from = 1) => Array.from({ length: n }, (_, i) => `/GVP/EroScripts/${artist}/S${i + from}/S${i + from}.mp4`);

describe('planPlaylistCleanup', () => {
  it('removes playlists with no video left in the library, deleted paths not counting', () => {
    const r = planPlaylistCleanup({
      playlists: [
        { id: 'k', name: 'Keep Forever', videoPaths: [] },
        { id: 'g', name: 'Gone', videoPaths: ['/deleted.mp4'] },
        { id: 'm', name: 'My two', videoPaths: ['/a.mp4', '/b.mp4'] },
      ],
      videoPaths: ['/a.mp4', '/b.mp4'],
    });
    expect(r.remove.map((x) => x.id)).toEqual(['k', 'g']);
  });

  it('removes an app-made artist playlist below the threshold and forgets it, a favourite only when empty', () => {
    const lib = [...vids('Misiv', MIN_ARTIST_VIDEOS - 1), ...vids('Agapilord', 1)];
    const state = { created: ['misiv', 'agapilord', ':favourites'], added: { [vids('Misiv', 1)[0]]: ['misiv'], [vids('Agapilord', 1)[0]]: ['agapilord', ':favourites'] } };
    const r = planPlaylistCleanup({
      playlists: [
        { id: 'mi', name: 'Misiv', videoPaths: vids('Misiv', MIN_ARTIST_VIDEOS) },
        { id: 'ag', name: 'Agapilord', videoPaths: vids('Agapilord', 1) },
        { id: 'fa', name: FAVOURITES_PLAYLIST, videoPaths: vids('Agapilord', 1) },
      ],
      videoPaths: lib,
      state,
      favourites: ['Agapilord'],
    });
    expect(r.remove).toEqual([{ id: 'mi', name: 'Misiv', videos: MIN_ARTIST_VIDEOS - 1 }]);
    expect(r.state.created).toEqual(['agapilord', ':favourites']);
    expect(r.state.added).toEqual({ [vids('Agapilord', 1)[0]]: ['agapilord', ':favourites'] });
  });

  it('lets a removed artist playlist come back, with every video, once the artist reaches the threshold', () => {
    const removed = planPlaylistCleanup({
      playlists: [{ id: 'mi', name: 'Misiv', videoPaths: vids('Misiv', MIN_ARTIST_VIDEOS) }],
      videoPaths: vids('Misiv', 2),
      state: { created: ['misiv'], added: Object.fromEntries(vids('Misiv', MIN_ARTIST_VIDEOS).map((p) => [p, ['misiv']])) },
    });
    const later = planArtistPlaylists({ videoPaths: vids('Misiv', MIN_ARTIST_VIDEOS), sourcePaths: ['/GVP/EroScripts'], playlists: [], state: removed.state });
    expect(later.create).toEqual([{ name: 'Misiv', videoPaths: vids('Misiv', MIN_ARTIST_VIDEOS) }]);
  });

  it("keeps the user's own small playlists, even one named like an artist the app never made", () => {
    const r = planPlaylistCleanup({
      playlists: [{ id: 'o', name: 'Misiv', videoPaths: vids('Misiv', 1) }],
      videoPaths: vids('Misiv', 1),
      state: { created: [] },
    });
    expect(r.remove).toEqual([]);
    expect(r.state).toEqual({ created: [] });
  });
});
