/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Score lists: one collection per preference score band
// (renderer/js/score-bands.js, SCORE_BANDS in content-preferences.js).
import { describe, it, expect } from 'vitest';
import {
  applyScoreBandCollections, byScore, planScoreBandCollections, scoreBandsOf,
} from '../../renderer/js/score-bands.js';
import { AUTO_COLLECTIONS_KEY, COLLECTIONS_KEY, planAutoCollections } from '../../renderer/js/auto-collections.js';
import { PREFERENCE_SCORES_KEY } from '../../renderer/js/library-scores.js';

const keysOf = (score) => scoreBandsOf(score).map((b) => b.key);

function store(init = {}) {
  const data = { ...init };
  const writes = [];
  return {
    data,
    writes,
    get: (k) => data[k],
    set: (k, val) => { data[k] = val; writes.push(k); },
  };
}

describe('scoreBandsOf', () => {
  it('puts the boundaries in the right lists; 20+ is also in 15+', () => {
    expect(keysOf(20)).toEqual(['score-20', 'score-15']);
    expect(keysOf(24)).toEqual(['score-20', 'score-15']);
    expect(keysOf(19.5)).toEqual(['score-15']);
    expect(keysOf(15)).toEqual(['score-15']);
    expect(keysOf(14.5)).toEqual(['score-10']);
    expect(keysOf(14.99)).toEqual(['score-10']);
    expect(keysOf(10)).toEqual(['score-10']);
    expect(keysOf(9.5)).toEqual(['score-low']);
    expect(keysOf(0)).toEqual(['score-low']);
    expect(keysOf(-3.5)).toEqual(['score-low']);
  });

  it('leaves unscored videos out of every list', () => {
    for (const s of [undefined, null, NaN, Infinity, '10', true]) expect(keysOf(s)).toEqual([]);
  });
});

describe('planScoreBandCollections', () => {
  const scores = { '/v/a.mp4': 9.5, '/v/b.mp4': 15, '/v/c.mp4': 14.5, '/v/d.mp4': 10, '/v/e.mp4': 17, '/v/f.mp4': 15 };
  const videoPaths = ['/v/a.mp4', '/v/b.mp4', '/v/c.mp4', '/v/d.mp4', '/v/e.mp4', '/v/f.mp4', '/v/unscored.mp4'];

  it('makes each band with its videos, highest score first, name breaking ties', () => {
    const r = planScoreBandCollections({ videoPaths, scores });
    expect(r.create).toEqual([
      { key: 'score-15', name: 'Score 15+', videoPaths: ['/v/e.mp4', '/v/b.mp4', '/v/f.mp4'] },
      { key: 'score-10', name: 'Score 10–14.5', videoPaths: ['/v/c.mp4', '/v/d.mp4'] },
      { key: 'score-low', name: 'Score below 10', videoPaths: ['/v/a.mp4'] },
    ]);
    expect(r.update).toEqual([]);
    expect(r.state.created).toEqual(['score-15', 'score-10', 'score-low']);
  });

  it('puts a 20+ video in both Score 20+ and Score 15+', () => {
    const r = planScoreBandCollections({ videoPaths: ['/v/top.mp4', '/v/b.mp4'], scores: { '/v/top.mp4': 21, '/v/b.mp4': 16 } });
    expect(r.create.map((c) => [c.name, c.videoPaths])).toEqual([
      ['Score 20+', ['/v/top.mp4']],
      ['Score 15+', ['/v/top.mp4', '/v/b.mp4']],
    ]);
  });

  it('makes no empty band', () => {
    const r = planScoreBandCollections({ videoPaths: ['/v/b.mp4'], scores });
    expect(r.create.map((c) => c.name)).toEqual(['Score 15+']);
    expect(r.state.created).toEqual(['score-15']);
  });

  it('follows the scores: a video that moved band moves collection, removed by hand comes back', () => {
    const collections = [
      { id: 'top', name: 'Score 15+', videoPaths: ['/v/e.mp4', '/v/b.mp4', '/v/f.mp4', '/v/c.mp4'] },
      { id: 'mid', name: 'score 10–14.5', videoPaths: ['/v/d.mp4'] },
      { id: 'low', name: 'Score below 10', videoPaths: ['/v/a.mp4'] },
    ];
    const r = planScoreBandCollections({ collections, videoPaths, scores, state: { created: ['score-15', 'score-10', 'score-low'] } });
    expect(r.create).toEqual([]);
    expect(r.update).toEqual([
      { id: 'top', videoPaths: ['/v/e.mp4', '/v/b.mp4', '/v/f.mp4'] },
      { id: 'mid', videoPaths: ['/v/c.mp4', '/v/d.mp4'] },
    ]);
  });

  it('rewrites nothing when the collections already match', () => {
    const first = planScoreBandCollections({ videoPaths, scores });
    const collections = first.create.map((c) => ({ id: c.key, name: c.name, videoPaths: c.videoPaths }));
    const r = planScoreBandCollections({ collections, videoPaths, scores, state: first.state });
    expect(r.create).toEqual([]);
    expect(r.update).toEqual([]);
  });

  it('reorders when only the order changed', () => {
    const collections = [{ id: 'top', name: 'Score 15+', videoPaths: ['/v/b.mp4', '/v/e.mp4', '/v/f.mp4'] }];
    const r = planScoreBandCollections({ collections, videoPaths, scores, state: { created: ['score-15', 'score-10', 'score-low'] } });
    expect(r.update).toEqual([{ id: 'top', videoPaths: ['/v/e.mp4', '/v/b.mp4', '/v/f.mp4'] }]);
  });

  it("doesn't make a band again once the user deleted it", () => {
    const r = planScoreBandCollections({ videoPaths, scores, state: { created: ['score-15'] } });
    expect(r.create.map((c) => c.name)).toEqual(['Score 10–14.5', 'Score below 10']);
  });

  it("keeps the other auto collections' history", () => {
    const state = { created: ['hentai'], added: { '/v/a.mp4': ['hentai'] } };
    const r = planScoreBandCollections({ videoPaths, scores, state });
    expect(r.state.added).toEqual(state.added);
    expect(r.state.created).toEqual(['hentai', 'score-15', 'score-10', 'score-low']);
    // And the other way round: the tag collections keep the band keys.
    expect(planAutoCollections({ videos: [], state: r.state }).state.created).toEqual(r.state.created);
  });
});

describe('applyScoreBandCollections', () => {
  it('creates, then follows score changes, writing only on a change', () => {
    const s = store({
      [PREFERENCE_SCORES_KEY]: { '/v/a.mp4': 16, '/v/b.mp4': 11, '/v/c.mp4': 2 },
      [COLLECTIONS_KEY]: [{ id: 'mine', name: 'Mine', videoPaths: ['/v/c.mp4'] }],
    });
    let n = 0;
    const paths = ['/v/a.mp4', '/v/b.mp4', '/v/c.mp4'];
    expect(applyScoreBandCollections(s, paths, () => `id${++n}`)).toEqual({ created: ['Score 15+', 'Score 10–14.5', 'Score below 10'], updated: 0 });
    expect(s.data[COLLECTIONS_KEY]).toEqual([
      { id: 'mine', name: 'Mine', videoPaths: ['/v/c.mp4'] },
      { id: 'id1', name: 'Score 15+', videoPaths: ['/v/a.mp4'] },
      { id: 'id2', name: 'Score 10–14.5', videoPaths: ['/v/b.mp4'] },
      { id: 'id3', name: 'Score below 10', videoPaths: ['/v/c.mp4'] },
    ]);
    expect(s.data[AUTO_COLLECTIONS_KEY]).toEqual({ created: ['score-15', 'score-10', 'score-low'] });

    s.writes.length = 0;
    expect(applyScoreBandCollections(s, paths)).toEqual({ created: [], updated: 0 });
    expect(s.writes).toEqual([]);

    // A tag score raised b to 15: it moves to Score 15+.
    s.data[PREFERENCE_SCORES_KEY] = { '/v/a.mp4': 16, '/v/b.mp4': 15, '/v/c.mp4': 2 };
    expect(applyScoreBandCollections(s, paths)).toEqual({ created: [], updated: 2 });
    const byName = Object.fromEntries(s.data[COLLECTIONS_KEY].map((c) => [c.name, c.videoPaths]));
    expect(byName['Score 15+']).toEqual(['/v/a.mp4', '/v/b.mp4']);
    expect(byName['Score 10–14.5']).toEqual([]);
    expect(byName.Mine).toEqual(['/v/c.mp4']);
    expect(s.writes).toEqual([COLLECTIONS_KEY]);
  });

  it('leaves out videos that are no longer in the library', () => {
    const s = store({ [PREFERENCE_SCORES_KEY]: { '/v/gone.mp4': 17, '/v/a.mp4': 17 } });
    applyScoreBandCollections(s, ['/v/a.mp4'], () => 'x');
    expect(s.data[COLLECTIONS_KEY]).toEqual([{ id: 'x', name: 'Score 15+', videoPaths: ['/v/a.mp4'] }]);
  });
});

describe('byScore', () => {
  it('orders highest first, unscored last, ties keep their order', () => {
    const scores = { '/b': 5, '/c': 10, '/d': 5, '/e': -1 };
    expect(byScore(['/a', '/b', '/c', '/d', '/e'], (p) => p, scores)).toEqual(['/c', '/b', '/d', '/e', '/a']);
    expect(byScore(undefined, (p) => p, scores)).toEqual([]);
  });
});
