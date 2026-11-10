/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Library overview (renderer/js/library-overview.js): capacity against the
// scouts' cap, ratings and their triage buckets, tag and artist listings over
// the scanned library only.
import { describe, it, expect } from 'vitest';
import {
  artistSummary, capacitySummary, formatBytes, ratingSummary, stepTagScore, tagSummary,
} from '../../renderer/js/library-overview.js';
import { FAVOURITES, LIBRARY_MAX_VIDEOS } from '../../renderer/js/content-preferences.js';
import { tagScoreInfo } from '../../renderer/js/preference-score.js';

const fakeData = (init = {}) => ({ get: (k) => init[k] });
const videos = (n, extra = {}) => Array.from({ length: n }, (_, i) => ({ path: `/lib/v${i}.mp4`, size: 1, ...extra }));

describe('capacitySummary', () => {
  it('counts videos against the scouts\' cap and sums their size per source, most videos first', () => {
    const c = capacitySummary([
      { path: '/a/1.mp4', size: 2e9, sourceName: 'Other' },
      { path: '/b/1.mp4', size: 1e9, sourceName: 'EroScripts' },
      { path: '/b/2.mp4', size: 3e9, sourceName: 'EroScripts' },
      { path: '/b/3.mp4', sourceName: 'EroScripts' },
    ]);
    expect(c).toMatchObject({ count: 4, cap: LIBRARY_MAX_VIDEOS, free: LIBRARY_MAX_VIDEOS - 4, bytes: 6e9, level: 'ok' });
    expect(c.sources).toEqual([
      { name: 'EroScripts', count: 3, bytes: 4e9 },
      { name: 'Other', count: 1, bytes: 2e9 },
    ]);
  });

  it('reads nearly full from 90 % and full at the cap, never past 0 free', () => {
    expect(capacitySummary(videos(449), { cap: 500 }).level).toBe('ok');
    expect(capacitySummary(videos(450), { cap: 500 }).level).toBe('high');
    expect(capacitySummary(videos(499), { cap: 500 })).toMatchObject({ level: 'high', percent: 99, free: 1 });
    expect(capacitySummary(videos(500), { cap: 500 })).toMatchObject({ level: 'full', percent: 100, free: 0 });
    expect(capacitySummary(videos(510), { cap: 500 })).toMatchObject({ level: 'full', percent: 102, free: 0 });
  });
});

describe('formatBytes', () => {
  it('uses decimal units, one decimal below 100', () => {
    expect(formatBytes(0, 'en')).toBe('0 B');
    expect(formatBytes(21.9e9, 'en')).toBe('21.9 GB');
    expect(formatBytes(542.6e9, 'en')).toBe('543 GB');
    expect(formatBytes(1.24e12, 'en')).toBe('1.2 TB');
  });
});

describe('ratingSummary', () => {
  const ratings = {
    '/lib/a.mp4': { rating: 5, favorite: true, updatedAt: 1 },
    '/lib/b.mp4': { rating: 4, favorite: false, updatedAt: 3 },
    '/lib/c.mp4': { rating: 3, favorite: false, updatedAt: 2 },
    '/lib/d.mp4': { rating: 0, favorite: true, updatedAt: 4 },
    '/lib/e.mp4': { rating: 2, favorite: true, updatedAt: 5 },
    '/lib/f.mp4': { rating: 1.5, favorite: false, updatedAt: 6 },
    '/gone/x.mp4': { rating: 5, favorite: true, updatedAt: 9 },
  };
  const paths = ['/lib/a.mp4', '/lib/b.mp4', '/lib/c.mp4', '/lib/d.mp4', '/lib/e.mp4', '/lib/f.mp4', '/lib/g.mp4'];

  it('counts only library videos and sorts them into the triage buckets', () => {
    const r = ratingSummary(ratings, paths);
    expect(r).toMatchObject({ total: 7, rated: 5, unrated: 2, favourites: 3, keep: 2, toDelete: 1 });
    expect(r).not.toHaveProperty('review');
  });

  it('lists every half-star step and the rated videos, favourites first', () => {
    const r = ratingSummary(ratings, paths);
    expect(r.stars.map((s) => s.rating)).toEqual([5, 4.5, 4, 3.5, 3, 2.5, 2, 1.5, 1, 0.5]);
    expect(r.stars.find((s) => s.rating === 4).count).toBe(1);
    expect(r.entries.map((e) => e.path)).toEqual(['/lib/a.mp4', '/lib/e.mp4', '/lib/d.mp4', '/lib/b.mp4', '/lib/c.mp4', '/lib/f.mp4']);
  });
});

describe('tagSummary', () => {
  it('counts each library video once per tag, without case, and keeps scored tags no video has', () => {
    const data = fakeData({
      'library.scriptTags': {
        '/lib/a.mp4': { tags: ['Creampie', 'creampie', 'animation'] },
        '/lib/b.mp4': { tags: ['animation'] },
        '/gone/x.mp4': { tags: ['animation', 'feet'] },
      },
      'library.tagScores': { animation: 2, 'Rare-Tag': -1, riding: 0 },
    });
    const rows = tagSummary(data, ['/lib/a.mp4', '/lib/b.mp4'], tagScoreInfo().groups);
    const byTag = Object.fromEntries(rows.map((r) => [r.tag, r]));
    expect(rows[0]).toEqual({ tag: 'animation', videos: 2, score: 2, group: null, changed: true });
    expect(byTag.creampie).toEqual({ tag: 'creampie', videos: 1, score: 2, group: 'Creampie', changed: false });
    expect(byTag['rare-tag']).toMatchObject({ videos: 0, score: -1, changed: true });
    // A grouped tag set to 0 by the user is listed at 0, changed.
    expect(byTag.riding).toMatchObject({ videos: 0, score: 0, group: 'Riding', changed: true });
    expect(byTag.feet?.videos ?? 0).toBe(0);
  });

  it('leaves out device, length, VR and bookkeeping tags, as the web remote does, unless scored', () => {
    const data = fakeData({
      'library.scriptTags': { '/lib/a.mp4': { tags: ['osr2', 'multi-axis', 'len-5-10', 'non-vr', 'VR', 'straight', 'riding'] } },
      'library.tagScores': { 'non-vr': -1 },
    });
    const tags = tagSummary(data, ['/lib/a.mp4'], tagScoreInfo().groups).filter((r) => r.videos > 0).map((r) => r.tag);
    expect(tags.sort()).toEqual(['non-vr', 'riding']);
  });
});

describe('artistSummary', () => {
  it('lists artist folders with their videos, favourites first, plus favourites without videos', () => {
    const data = fakeData({
      'library.sources': [{ path: '/lib' }],
      'library.favouriteArtists': { Wezzam: true, [FAVOURITES[1]]: false },
    });
    const rows = artistSummary(data, [
      '/lib/Wezzam/Scene 1/a.mp4',
      '/lib/Somebody/Scene/b.mp4',
      '/lib/Somebody/Scene 2/c.mp4',
      `/lib/${FAVOURITES[0]}/Scene/d.mp4`,
      `/lib/${FAVOURITES[1]}/Scene/e.mp4`,
    ]);
    const favourites = rows.filter((r) => r.favourite).map((r) => r.name);
    expect(rows.slice(0, 2).map((r) => r.name).sort()).toEqual([FAVOURITES[0], 'Wezzam'].sort());
    expect(favourites).not.toContain(FAVOURITES[1]);
    expect(rows.find((r) => r.name === 'Somebody')).toEqual({ name: 'Somebody', videos: 2, favourite: false });
    expect(rows.find((r) => r.name === FAVOURITES[2])).toEqual({ name: FAVOURITES[2], videos: 0, favourite: true });
  });
});

describe('stepTagScore', () => {
  it('steps by 0.5 within ±10', () => {
    expect(stepTagScore(0, 1)).toBe(0.5);
    expect(stepTagScore(-0.5, 1)).toBe(0);
    expect(stepTagScore(10, 1)).toBe(10);
    expect(stepTagScore(-10, -1)).toBe(-10);
  });
});
