/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Collections by medium, length, franchise and script style
// (renderer/js/auto-collections.js).
import { describe, it, expect } from 'vitest';
import { planAutoCollections, applyAutoCollections, AUTO_COLLECTIONS_KEY, COLLECTIONS_KEY } from '../../renderer/js/auto-collections.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';

const v = (path, tags = [], duration = 0) => ({ path, tags: new Set(tags), duration });
const names = (r) => r.create.map((c) => [c.name, c.videoPaths]);

describe('planAutoCollections', () => {
  it('sorts by tags and length, makes only collections something matches', () => {
    const r = planAutoCollections({
      videos: [
        v('/a', ['sfm', 'overwatch', 'beat-based'], 120),
        v('/b', ['vam', 'hybrid-based'], 4000),
        v('/c', ['len-60-plus']),
      ],
    });
    expect(names(r)).toEqual([
      ['SFM', ['/a']], ['VaM', ['/b']],
      ['Quick (under 5 min)', ['/a']], ['Sessions (15 min+)', ['/b', '/c']], ['Marathons (1 hr+)', ['/b', '/c']],
      ['Overwatch', ['/a']], ['Music / beat', ['/a', '/b']], ['Action', ['/b']],
    ]);
  });

  it('a known duration wins over the post\'s length tag', () => {
    const r = planAutoCollections({ videos: [v('/a', ['len-60-plus'], 200)] });
    expect(names(r)).toEqual([['Quick (under 5 min)', ['/a']]]);
  });

  it('reuses a collection of the same name, and keeps out a video taken out by hand', () => {
    const first = planAutoCollections({ collections: [{ id: 'c', name: 'sfm', videoPaths: [] }], videos: [v('/a', ['sfm'])] });
    expect(first.extend).toEqual([{ id: 'c', videoPaths: ['/a'] }]);
    const again = planAutoCollections({ collections: [{ id: 'c', name: 'SFM', videoPaths: [] }], videos: [v('/a', ['sfm']), v('/b', ['sfm'])], state: first.state });
    expect(again.extend).toEqual([{ id: 'c', videoPaths: ['/b'] }]);
  });

  it('never makes a deleted collection again', () => {
    const first = planAutoCollections({ videos: [v('/a', ['mmd'])] });
    expect(planAutoCollections({ videos: [v('/a', ['mmd']), v('/b', ['mmd'])], state: first.state })).toMatchObject({ create: [], extend: [] });
  });
});

describe('applyAutoCollections', () => {
  it('writes new collections from tags and durations once; a second run changes nothing', () => {
    const store = {
      [SCRIPT_TAGS_KEY]: { '/a': { tags: ['hentai'] } },
      'library.durationCache': { '/a': 1000 },
      [COLLECTIONS_KEY]: [{ id: 'mine', name: 'Mine', videoPaths: ['/x'] }],
    };
    const data = { get: (k) => store[k], set: (k, val) => { store[k] = val; } };
    let n = 0;
    expect(applyAutoCollections(data, ['/a'], () => `id${++n}`)).toEqual({ created: ['Hentai', 'Sessions (15 min+)'], extended: 0 });
    expect(store[COLLECTIONS_KEY]).toEqual([
      { id: 'mine', name: 'Mine', videoPaths: ['/x'] },
      { id: 'id1', name: 'Hentai', videoPaths: ['/a'] },
      { id: 'id2', name: 'Sessions (15 min+)', videoPaths: ['/a'] },
    ]);
    expect(store[AUTO_COLLECTIONS_KEY].added).toEqual({ '/a': ['hentai', 'sessions'] });
    expect(applyAutoCollections(data, ['/a'])).toEqual({ created: [], extended: 0 });
  });
});
