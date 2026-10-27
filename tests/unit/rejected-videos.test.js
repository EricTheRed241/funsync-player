/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Triage deletions: deleting a rejected video and its companions
// (electron/rejected-videos.js), and the renderer sweep that decides which
// videos go (renderer/js/reject-rule.js: rated 2 stars or lower).
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { sweepRejected, syncReview, ratingVerdict, REVIEW_PLAYLIST } from '../../renderer/js/reject-rule.js';

const require = createRequire(import.meta.url);
const { deleteRejectedVideo, readRejected, recordRejection, faptapRejectionKey } = require('../../electron/rejected-videos.js');

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
    expect(readRejected(userData).entries).toEqual([expect.objectContaining({ key: '42|3.mp4', topicId: 42, reason: 'low-rating' })]);
  });

  it('records a duplicate and the historic no-thank-you as their own reasons, and refuses an unknown one', () => {
    const video = touch('A/S/S.mp4');
    touch('A/S/eroscripts.json', JSON.stringify({ topicId: 7, video: 'v.mp4' }));
    deleteRejectedVideo(video, { sources: [lib], userDataDir: userData, reason: 'no-thank-you' });
    const copy = touch('C/S/S.mp4');
    touch('C/S/eroscripts.json', JSON.stringify({ topicId: 8, video: 'w.mp4' }));
    deleteRejectedVideo(copy, { sources: [lib], userDataDir: userData, reason: 'duplicate' });
    expect(readRejected(userData).entries.map((e) => [e.key, e.reason])).toEqual([['7|v.mp4', 'no-thank-you'], ['8|w.mp4', 'duplicate']]);
    expect(() => deleteRejectedVideo(touch('B/S/S.mp4'), { sources: [lib], userDataDir: userData, reason: 'short' })).toThrow(/unknown reason/);
  });

  it('deletes a FapTap scene with its companions and sidecar, and remembers it under its FapTap key', () => {
    const stem = 'Zoot - YOGA TIME with Bulma!';
    const video = touch(`Zoot/${stem}/${stem}.mp4`);
    touch(`Zoot/${stem}/${stem}.funscript`);
    touch(`Zoot/${stem}/${stem}.generated.roll.funscript`);
    touch(`Zoot/${stem}/faptap.json`, JSON.stringify({
      faptapId: '2114528228239478785', url: 'https://faptap.net/v/2114528228239478785', title: 'YOGA TIME with Bulma!',
      author: 'Zoot', tags: ['riding'], file: `${stem}.mp4`,
    }));
    touch('Zoot/Other/Other.mp4');

    const r = deleteRejectedVideo(video, { sources: [lib], userDataDir: userData, now: 0, reason: 'low-rating' });

    expect(r.deleted).toHaveLength(3);
    expect(r.removedDirs).toEqual([path.join(lib, 'Zoot', stem)]);
    expect(exists(`Zoot/${stem}`)).toBe(false);
    expect(exists('Zoot/Other/Other.mp4')).toBe(true);
    expect(faptapRejectionKey('2114528228239478785')).toBe('faptap|2114528228239478785');
    expect(readRejected(userData).entries).toEqual([{
      key: 'faptap|2114528228239478785', source: 'faptap', faptapId: '2114528228239478785', title: 'YOGA TIME with Bulma!',
      url: 'https://faptap.net/v/2114528228239478785', reason: 'low-rating', path: video, at: new Date(0).toISOString(),
    }]);
  });

  it('prefers the EroScripts sidecar when a scene has both', () => {
    const video = touch('A/S/S.mp4');
    touch('A/S/eroscripts.json', JSON.stringify({ topicId: 9, video: 'v.mp4' }));
    touch('A/S/faptap.json', JSON.stringify({ faptapId: '5' }));
    deleteRejectedVideo(video, { sources: [lib], userDataDir: userData });
    expect(readRejected(userData).entries.map((e) => e.key)).toEqual(['9|v.mp4']);
    expect(exists('A')).toBe(false);
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

  it('keys FapTap entries by their FapTap id', () => {
    recordRejection(userData, { source: 'faptap', faptapId: '11', reason: 'low-rating' });
    recordRejection(userData, { source: 'faptap', faptapId: '11', reason: 'no-thank-you' });
    recordRejection(userData, { topicId: 11, video: 'a.mp4', reason: 'short' });
    expect(readRejected(userData).entries.map((e) => `${e.key} ${e.reason}`)).toEqual(['faptap|11 no-thank-you', '11|a.mp4 short']);
  });
});

describe('ratingVerdict', () => {
  it('deletes at 2 or lower, reviews 2½–3½, keeps 4 and up; unrated and favourites never go', () => {
    const v = (rating, favorite = false) => ratingVerdict({ rating, favorite });
    expect([0.5, 2, 2.5, 3, 3.5, 4, 5].map((r) => v(r))).toEqual(['delete', 'delete', 'review', 'review', 'review', 'keep', 'keep']);
    expect(v(0)).toBe(null);
    expect(ratingVerdict(null)).toBe(null);
    expect(v(1, true)).toBe('review');
  });
});

function data(playlists, ratings = {}, store = {}) {
  let n = 0;
  return {
    get: (k) => store[k],
    set: (k, v) => { store[k] = v; },
    getRatings: () => ratings,
    setRating: (p) => { delete ratings[p]; },
    getPlaylists: () => playlists,
    addPlaylist: async (name) => { const pl = { id: `new${++n}`, name, videoPaths: [] }; playlists.push(pl); return pl; },
    addVideosToPlaylist: (id, paths) => { playlists.find((x) => x.id === id).videoPaths.push(...paths); },
    removeVideoFromPlaylist: (id, p) => {
      const pl = playlists.find((x) => x.id === id);
      pl.videoPaths = pl.videoPaths.filter((v) => v !== p);
    },
  };
}

describe('sweepRejected', () => {
  it('takes each deleted video out of every playlist and keeps one whose deletion failed', async () => {
    const playlists = [
      { id: 'k', name: 'Keep Forever', videoPaths: ['/a.mp4', '/b.mp4'] },
      { id: 'd', name: 'Discovery', videoPaths: ['/a.mp4', '/b.mp4', '/c.mp4'] },
    ];
    const ratings = { '/a.mp4': { rating: 1 }, '/b.mp4': { rating: 2 } };
    const r = await sweepRejected({ data: data(playlists, ratings), deleteVideo: async (p) => (p === '/b.mp4' ? { success: false, error: 'busy' } : { success: true }) });
    expect(r).toEqual({ removed: [{ path: '/a.mp4', reason: 'low-rating' }], failed: [{ path: '/b.mp4', error: 'busy' }] });
    expect(playlists.map((p) => p.videoPaths)).toEqual([['/b.mp4'], ['/b.mp4', '/c.mp4']]);
  });

  it('deletes videos rated 2 or lower and clears their rating; nothing else', async () => {
    const ratings = { '/low.mp4': { rating: 2 }, '/fav.mp4': { rating: 1, favorite: true }, '/mid.mp4': { rating: 3 }, '/top.mp4': { rating: 4.5 } };
    const calls = [];
    const r = await sweepRejected({ data: data([{ id: 'd', name: 'Discovery', videoPaths: ['/low.mp4'] }], ratings), deleteVideo: async (p, reason) => { calls.push([p, reason]); return { success: true }; } });
    expect(calls).toEqual([['/low.mp4', 'low-rating']]);
    expect(r.removed).toEqual([{ path: '/low.mp4', reason: 'low-rating' }]);
    expect(Object.keys(ratings)).toEqual(['/fav.mp4', '/mid.mp4', '/top.mp4']);
  });

  it('leaves videos in a No thank you playlist alone unless they are rated 2 or lower', async () => {
    const playlists = [{ id: 'n', name: 'No thank you', videoPaths: ['/a.mp4', '/b.mp4'] }];
    const ratings = { '/b.mp4': { rating: 1.5 } };
    const r = await sweepRejected({ data: data(playlists, ratings), deleteVideo: async () => ({ success: true }) });
    expect(r).toEqual({ removed: [{ path: '/b.mp4', reason: 'low-rating' }], failed: [] });
    expect(playlists[0].videoPaths).toEqual(['/a.mp4']);
  });
});

describe('syncReview', () => {
  it('makes Review for 2½–3½-star videos and takes out ones rated 4 or more', async () => {
    const playlists = [];
    const ratings = { '/a.mp4': { rating: 3 }, '/b.mp4': { rating: 2.5 }, '/c.mp4': { rating: 5 } };
    const d = data(playlists, ratings);
    expect(await syncReview(d)).toEqual({ added: 2, removed: 0 });
    expect(playlists).toEqual([{ id: 'new1', name: REVIEW_PLAYLIST, videoPaths: ['/a.mp4', '/b.mp4'] }]);

    ratings['/a.mp4'] = { rating: 4 };
    expect(await syncReview(d, '/a.mp4')).toEqual({ added: 0, removed: 1 });
    expect(playlists[0].videoPaths).toEqual(['/b.mp4']);
  });

  it('at startup leaves out a video taken out by hand; a new 3-star rating puts it back', async () => {
    const playlists = [{ id: 'r', name: 'review', videoPaths: [] }];
    const ratings = { '/a.mp4': { rating: 3 } };
    const d = data(playlists, ratings, { 'library.reviewAdded': ['/a.mp4'] });
    expect(await syncReview(d)).toEqual({ added: 0, removed: 0 });
    expect(await syncReview(d, '/a.mp4')).toEqual({ added: 1, removed: 0 });
    expect(playlists[0].videoPaths).toEqual(['/a.mp4']);
  });
});
