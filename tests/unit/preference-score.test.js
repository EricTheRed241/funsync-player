/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The preference score: one ranking for the scouts and the library
// (renderer/js/preference-score.js), and what the library feeds it per scene
// (renderer/js/library-scores.js): VR, hand-made axes, post tags, the title,
// favourite / known artists, likes, and the stored map.
import { describe, it, expect } from 'vitest';
import {
  candidateScore, normalizeTagScore, tagKey, tagScoreSum, TAG_SCORE_MAX, TAG_SCORE_STEP,
} from '../../renderer/js/preference-score.js';
import * as scout from '../../scripts/eroscripts-scout-lib.mjs';
import {
  applyPreferenceScores, planPreferenceScores, sameScores, sceneCandidate, PREFERENCE_SCORES_KEY, TAG_SCORES_KEY,
} from '../../renderer/js/library-scores.js';
import { MIN_ARTIST_VIDEOS, RANKING } from '../../renderer/js/content-preferences.js';

const SOURCES = ['/GVP/EroScripts', '/GVP/FapTap', '/GVP/Other'];
const scene = (artist, name, root = '/GVP/EroScripts') => `${root}/${artist}/${name}/${name}.mp4`;
const video = (path, hasFunscript = true) => ({ path, name: path.split('/').pop(), hasFunscript });
const score = (o) => candidateScore(sceneCandidate(o));

describe('one ranking for scouts and library', () => {
  it('is the scout lib\'s own ranking, not a copy', () => {
    expect(scout.candidateScore).toBe(candidateScore);
  });

  it('scores a library scene as the scouts score the same post', () => {
    const post = { title: 'Tracer rides', tags: ['animation', '3d', 'riding', 'creampie', 'pmv'], likes: 40, vr: true, axisType: 'multi', artistInLibrary: true };
    const music = true;
    const preferred = ['riding', 'creampie'];
    expect(score({ title: post.title, tags: post.tags, likes: 40, vr: true, axisKind: 'handmade', artistInLibrary: true }))
      .toBe(candidateScore({ ...post, music, preferred }));
  });
});

describe('scoring boundaries', () => {
  it('gives hand-made axes RANKING.multiAxis; generated and single-axis get nothing for axes', () => {
    const base = { title: 'Scene', tags: ['3d'] };
    expect(score({ ...base, axisKind: 'handmade' }) - score({ ...base, axisKind: 'generated' })).toBe(RANKING.multiAxis);
    expect(score({ ...base, axisKind: 'generated' })).toBe(score({ ...base, axisKind: 'single' }));
    expect(score({ ...base, axisKind: null })).toBe(score({ ...base, axisKind: 'single' }));
  });

  it('gives VR RANKING.vr', () => {
    expect(score({ title: 'Scene', vr: true }) - score({ title: 'Scene' })).toBe(RANKING.vr);
  });

  it('takes RANKING.hentaiOnly off 2D hentai, not off hentai with a 3D tag or title', () => {
    const flat = score({ title: 'Scene', tags: ['hentai', 'animation'] });
    expect(flat).toBe(-RANKING.hentaiOnly);
    expect(score({ title: 'Scene', tags: ['hentai', 'sfm'] })).toBe(0);
    expect(score({ title: '[SFM] Scene', tags: ['hentai'] })).toBe(0);
  });

  it('takes 0.5 off a monster and 0.5 off feet, by tag or title', () => {
    expect(score({ title: 'Scene', tags: ['tentacle'] })).toBe(-RANKING.monster);
    expect(score({ title: 'Werewolf WW scene' })).toBe(-RANKING.monster);
    expect(score({ title: 'Scene - footjob' })).toBe(-RANKING.feet);
    expect(score({ title: 'Orc footjob', tags: ['feet'] })).toBe(-(RANKING.monster + RANKING.feet));
  });

  it('adds music and preferred acts from the tags, whatever their case', () => {
    expect(score({ title: 'Scene', tags: ['PMV'] })).toBe(RANKING.music);
    expect(score({ title: 'Scene', tags: ['Creampie', 'riding', 'riding'] })).toBe(RANKING.preferred + RANKING.preferredExtra);
  });

  it('counts likes from the record, capped', () => {
    expect(score({ title: 'Scene', likes: RANKING.likesPer })).toBe(1);
    expect(score({ title: 'Scene', likes: 'many' })).toBe(0);
    expect(score({ title: 'Scene', likes: 1e6 })).toBe(RANKING.likesMax);
  });
});

describe('planPreferenceScores', () => {
  const plan = (o) => planPreferenceScores({ sourcePaths: SOURCES, ...o });

  it('gives a favourite\'s artist folder the library-creator bonus from one video', () => {
    const fav = scene('ScyllaHMV', 'One');
    const other = scene('Nobody', 'One');
    const s = plan({ videos: [video(fav), video(other)], favourites: ['scyllahmv'], axisFlags: { [fav]: 'single', [other]: 'single' } });
    expect(s[fav] - s[other]).toBe(RANKING.artistInLibrary);
  });

  it('gives a known artist the bonus from MIN_ARTIST_VIDEOS videos, across sources and spellings', () => {
    const known = [
      ...Array.from({ length: MIN_ARTIST_VIDEOS - 1 }, (_, i) => scene('Misiv', `S${i}`)),
      scene('misiv', 'Last', '/GVP/FapTap'),
    ];
    const few = Array.from({ length: MIN_ARTIST_VIDEOS - 1 }, (_, i) => scene('Rare', `S${i}`));
    const videos = [...known, ...few].map((p) => video(p, false));
    const s = plan({ videos });
    expect(known.every((p) => s[p] === RANKING.artistInLibrary)).toBe(true);
    expect(few.every((p) => s[p] === 0)).toBe(true);
    // No artist folder (`<source>/<scene>/<video>`): no bonus.
    expect(plan({ videos: [video('/GVP/Other/Scene/Scene.mp4', false)], favourites: ['Scene'] })['/GVP/Other/Scene/Scene.mp4']).toBe(0);
  });

  it('maps VR, hand-made axes, post tags, likes and the title per video', () => {
    const a = scene('X', 'Tracer rides 3D');
    const b = scene('Y', 'Some hentai');
    const c = scene('Z', 'Orc feet');
    const s = plan({
      videos: [video(a), video(b), video(c, false)],
      scriptTags: { [a]: { tags: ['creampie', 'vr'], likes: 60 }, [b]: { tags: ['hentai'] } },
      vrOf: (p) => p === a,
      axisFlags: { [a]: 'handmade', [b]: 'generated' },
    });
    expect(s[a]).toBe(RANKING.vr + RANKING.multiAxis + RANKING.preferred + 3);
    expect(s[b]).toBe(-RANKING.hentaiOnly);
    // No post tags: still scored from the title.
    expect(s[c]).toBe(-(RANKING.monster + RANKING.feet));
  });

  it('keeps the stored score of a scripted video whose axes the backend has not decided yet', () => {
    const a = scene('X', 'Pending');
    const b = scene('X', 'New');
    const s = plan({ videos: [video(a), video(b)], previous: { [a]: 9 } });
    expect(s).toEqual({ [a]: 9, [b]: 0 });
    expect(plan({ videos: [video(a)], previous: { [a]: 9 }, axisFlags: { [a]: 'generated' } })[a]).toBe(0);
  });

  it('drops videos no longer in the library', () => {
    const a = scene('X', 'Here');
    expect(Object.keys(plan({ videos: [video(a, false)], previous: { '/gone.mp4': 4 } }))).toEqual([a]);
  });
});

describe('applyPreferenceScores', () => {
  const store = (init = {}) => {
    const data = { ...init };
    let writes = 0;
    return {
      get: (k) => data[k],
      set: (k, v) => { data[k] = v; writes++; },
      get writes() { return writes; },
    };
  };

  it('stores the scores only when they change', () => {
    const a = scene('X', 'Scene');
    const data = store({ 'library.sources': SOURCES.map((path) => ({ path })), 'library.scriptTags': { [a]: { tags: ['riding'] } } });
    const first = applyPreferenceScores(data, [video(a)], { axisFlags: { [a]: 'handmade' } });
    expect(first).toEqual({ scores: { [a]: RANKING.multiAxis + RANKING.preferred }, changed: true });
    expect(data.get(PREFERENCE_SCORES_KEY)).toEqual(first.scores);
    expect(applyPreferenceScores(data, [video(a)], { axisFlags: { [a]: 'handmade' } }).changed).toBe(false);
    expect(data.writes).toBe(1);
    expect(applyPreferenceScores(data, [video(a)], { axisFlags: { [a]: 'generated' } }).changed).toBe(true);
  });

  it('compares maps by content, not key order', () => {
    expect(sameScores({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true);
    expect(sameScores({ a: 1 }, { a: 1, b: 2 })).toBe(false);
    expect(sameScores({ a: 1 }, { a: 1.5 })).toBe(false);
  });
});

describe('tag scores', () => {
  it('adds up the scores of a post\'s distinct tags, negative ones too; tags without a score count 0', () => {
    const scores = { riding: 2, monstergirl: -1.5, pmv: 0.5 };
    expect(tagScoreSum(['riding', 'pmv', 'monstergirl', '3d'], scores)).toBe(1);
    expect(tagScoreSum(['riding', 'Riding', ' RIDING '], scores)).toBe(2);
    expect(tagScoreSum(['3d', 'sfm'], scores)).toBe(0);
    expect(tagScoreSum([], scores)).toBe(0);
    expect(tagScoreSum(['riding'], {})).toBe(0);
    expect(tagScoreSum(['riding'], undefined)).toBe(0);
    expect(tagScoreSum(undefined, scores)).toBe(0);
    expect(tagScoreSum(['toString', 'constructor'], {})).toBe(0);
  });

  it('rounds scores to 0.5 steps (halves up) and clamps them to ±TAG_SCORE_MAX; anything else is not a score', () => {
    expect([TAG_SCORE_MAX, TAG_SCORE_STEP]).toEqual([10, 0.5]);
    expect([1.5, 1.2, 1.25, -1.25, -1.3, 0.24, '2.5', 11, -99, 10, -10].map(normalizeTagScore))
      .toEqual([1.5, 1, 1.5, -1, -1.5, 0, 2.5, 10, -10, 10, -10]);
    expect(Object.is(normalizeTagScore(-0.2), 0)).toBe(true);
    expect([NaN, Infinity, '', ' ', 'x', null, undefined, true, {}, []].map(normalizeTagScore)).toEqual(Array(10).fill(null));
    expect(tagScoreSum(['riding', 'pmv'], { riding: 25, pmv: 'abc' })).toBe(10);
  });

  it('keys tags trimmed and lower case', () => {
    expect([tagKey(' Riding '), tagKey('PMV'), tagKey(3), tagKey(null)]).toEqual(['riding', 'pmv', '', '']);
  });

  it('adds the tag scores on top of the RANKING score, and changes nothing without any', () => {
    const post = { title: 'Tracer rides', tags: ['3d', 'riding', 'pmv'], likes: 30, vr: true, music: true, preferred: ['riding'], axisType: 'multi' };
    const base = candidateScore(post);
    expect(candidateScore(post, {})).toBe(base);
    expect(candidateScore(post, { blowjob: 4 })).toBe(base);
    expect(candidateScore(post, { riding: 2.5, pmv: -1 })).toBeCloseTo(base + 1.5, 10);
    expect(candidateScore(post, { riding: -20 })).toBeCloseTo(base - 10, 10);
  });

  it('scores library scenes with library.tagScores', () => {
    const a = scene('X', 'Riding scene');
    const b = scene('Y', 'Other scene');
    const scriptTags = { [a]: { tags: ['riding', 'Monstergirl'] }, [b]: { tags: ['3d'] } };
    const base = { videos: [video(a), video(b)], sourcePaths: SOURCES, scriptTags, axisFlags: { [a]: 'generated', [b]: 'generated' } };
    const without = planPreferenceScores(base);
    const withScores = planPreferenceScores({ ...base, tagScores: { riding: 1, monstergirl: -3, '3d': 0.5 } });
    expect(withScores[a]).toBe(without[a] - 2);
    expect(withScores[b]).toBe(without[b] + 0.5);

    const store = { 'library.sources': SOURCES.map((path) => ({ path })), 'library.scriptTags': scriptTags };
    const data = { get: (k) => store[k], set: (k, v) => { store[k] = v; } };
    applyPreferenceScores(data, base.videos, { axisFlags: base.axisFlags });
    expect(store[PREFERENCE_SCORES_KEY]).toEqual(without);
    store[TAG_SCORES_KEY] = { riding: 1, monstergirl: -3, '3d': 0.5 };
    expect(applyPreferenceScores(data, base.videos, { axisFlags: base.axisFlags })).toEqual({ scores: withScores, changed: true });
  });
});
