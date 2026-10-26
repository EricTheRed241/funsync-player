/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// "No thank you" rule: deleting a rejected video and its companions
// (electron/rejected-videos.js), and the renderer sweep that decides which
// videos go (renderer/js/reject-rule.js).
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { sweepRejected } from '../../renderer/js/reject-rule.js';

const require = createRequire(import.meta.url);
const { deleteRejectedVideo, readRejected, recordRejection } = require('../../electron/rejected-videos.js');

let tmp;
let lib;
let userData;
beforeEach(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'rejected-'));
  lib = path.join(tmp, '_incoming');
  userData = path.join(tmp, 'userData');
  fs.mkdirSync(userData);
});
afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

function touch(rel, content = '{}') {
  const p = path.join(lib, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
  return p;
}
const exists = (rel) => fs.existsSync(path.join(lib, rel));

describe('deleteRejectedVideo', () => {
  it('deletes the scene with its scripts, axes and subtitles, then its empty folders, and remembers the post', () => {
    const video = touch('Artist/Artist - Scene/Artist - Scene.mp4');
    for (const n of ['Artist - Scene.funscript', 'Artist - Scene.twist.funscript', 'Artist - Scene.generated.roll.funscript', 'Artist - Scene.srt']) {
      touch(`Artist/Artist - Scene/${n}`);
    }
    touch('Artist/Artist - Scene/eroscripts.json', JSON.stringify({ topicId: 42, video: '3.mp4', title: '[Artist] Scene' }));

    const r = deleteRejectedVideo(video, { sources: [lib], userDataDir: userData, now: 0 });

    expect(r.deleted).toHaveLength(5);
    expect(exists('Artist')).toBe(false);
    expect(fs.existsSync(lib)).toBe(true);
    expect(readRejected(userData).entries).toEqual([expect.objectContaining({ key: '42|3.mp4', topicId: 42, reason: 'no-thank-you' })]);
  });

  it('leaves another video of the folder, its scripts, and a folder with other files', () => {
    const video = touch('Pack/Title.mp4');
    touch('Pack/Title.funscript');
    touch('Pack/Title.Part2.mp4');
    touch('Pack/Title.Part2.funscript');
    touch('Pack/cover.jpg');

    deleteRejectedVideo(video, { sources: [lib], userDataDir: userData });

    expect(fs.readdirSync(path.join(lib, 'Pack')).sort()).toEqual(['Title.Part2.funscript', 'Title.Part2.mp4', 'cover.jpg']);
    expect(readRejected(userData).entries).toEqual([]);
  });

  it('refuses anything outside a library source, and a video already gone is not an error', () => {
    const outside = path.join(tmp, 'elsewhere', 'x.mp4');
    fs.mkdirSync(path.dirname(outside));
    fs.writeFileSync(outside, '');
    expect(() => deleteRejectedVideo(outside, { sources: [lib], userDataDir: userData })).toThrow(/not inside a library source/);
    expect(() => deleteRejectedVideo(path.join(lib, '..', 'elsewhere', 'x.mp4'), { sources: [lib], userDataDir: userData })).toThrow();
    expect(fs.existsSync(outside)).toBe(true);
    expect(deleteRejectedVideo(path.join(lib, 'gone.mp4'), { sources: [lib], userDataDir: userData })).toEqual({ deleted: [], removedDirs: [] });
  });
});

describe('recordRejection', () => {
  it('keeps one entry per post video, the latest', () => {
    recordRejection(userData, { topicId: 1, video: 'a.mp4', reason: 'short' });
    recordRejection(userData, { topicId: 1, video: 'a.mp4', reason: 'no-thank-you' });
    recordRejection(userData, { topicId: 1, video: 'b.mp4', reason: 'short' });
    expect(readRejected(userData).entries.map((e) => `${e.key} ${e.reason}`)).toEqual(['1|a.mp4 no-thank-you', '1|b.mp4 short']);
  });
});

describe('sweepRejected', () => {
  function data(playlists) {
    return {
      getPlaylists: () => playlists,
      removeVideoFromPlaylist: (id, p) => {
        const pl = playlists.find((x) => x.id === id);
        pl.videoPaths = pl.videoPaths.filter((v) => v !== p);
      },
    };
  }

  it('takes each deleted video out of every playlist and keeps one whose deletion failed', async () => {
    const playlists = [
      { id: 'n', name: ' no THANK you ', videoPaths: ['/a.mp4', '/b.mp4'] },
      { id: 'd', name: 'Discovery', videoPaths: ['/a.mp4', '/b.mp4', '/c.mp4'] },
    ];
    const r = await sweepRejected({ data: data(playlists), deleteVideo: async (p) => (p === '/b.mp4' ? { success: false, error: 'busy' } : { success: true }) });
    expect(r).toEqual({ removed: ['/a.mp4'], failed: [{ path: '/b.mp4', error: 'busy' }] });
    expect(playlists.map((p) => p.videoPaths)).toEqual([['/b.mp4'], ['/b.mp4', '/c.mp4']]);
  });

  it('does nothing without a No thank you playlist', async () => {
    const r = await sweepRejected({ data: data([{ id: 'd', name: 'Discovery', videoPaths: ['/a.mp4'] }]), deleteVideo: async () => { throw new Error('called'); } });
    expect(r).toEqual({ removed: [], failed: [] });
  });
});
