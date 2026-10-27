/**
 * @vitest-environment node
 * Web remote VR decision, tag filter and Tags view list
 * (backend/web-remote/video-tags.js).
 */
import { describe, it, expect } from 'vitest';
import { videoIsVR, hasTag, filterByTag, tagGroupings } from '../../backend/web-remote/video-tags.js';

const VIDEOS = [
  { id: 'a', name: 'Alpha', duration: 100, tags: ['creampie', 'riding'] },
  { id: 'b', name: 'Bravo', duration: 50, tags: ['riding'] },
  { id: 'c', name: 'Charlie', duration: 0, tags: ['Riding', 'riding', 'anal'] },
  { id: 'd', name: 'Delta' },
];

describe('videoIsVR', () => {
  it("uses the backend's decision over the filename", () => {
    expect(videoIsVR({ name: 'Scene_180_LR', isVR: false })).toBe(false);
    expect(videoIsVR({ name: 'CuddleMocap 031', isVR: true })).toBe(true);
  });

  it('falls back to the name heuristic when the field is missing', () => {
    expect(videoIsVR({ name: 'Scene_180_LR' })).toBe(true);
    expect(videoIsVR({ name: 'Plain scene' })).toBe(false);
    expect(videoIsVR(null)).toBe(false);
  });
});

describe('tag filter', () => {
  it('matches case-insensitively', () => {
    expect(hasTag(VIDEOS[2], 'riding')).toBe(true);
    expect(hasTag(VIDEOS[0], ' Creampie ')).toBe(true);
    expect(hasTag(VIDEOS[3], 'riding')).toBe(false);
  });

  it('keeps only videos carrying the tag', () => {
    expect(filterByTag(VIDEOS, 'riding').map((v) => v.id)).toEqual(['a', 'b', 'c']);
    expect(filterByTag(VIDEOS, 'anal').map((v) => v.id)).toEqual(['c']);
    expect(filterByTag(VIDEOS, 'missing')).toEqual([]);
  });

  it('an empty tag filters nothing', () => {
    expect(filterByTag(VIDEOS, '')).toBe(VIDEOS);
    expect(filterByTag(VIDEOS, undefined)).toBe(VIDEOS);
  });
});

describe('Tags view list', () => {
  it('counts each video once per tag, most used first, then by name', () => {
    expect(tagGroupings(VIDEOS)).toEqual([
      { id: 'riding', name: 'riding', videoCount: 3, totalDuration: 150, videoIds: ['a', 'b', 'c'] },
      { id: 'anal', name: 'anal', videoCount: 1, totalDuration: 0, videoIds: ['c'] },
      { id: 'creampie', name: 'creampie', videoCount: 1, totalDuration: 100, videoIds: ['a'] },
    ]);
  });

  it('is empty without tags', () => {
    expect(tagGroupings([{ id: 'x' }])).toEqual([]);
    expect(tagGroupings(null)).toEqual([]);
  });
});
