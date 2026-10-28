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
import { candidateScore } from '../../renderer/js/preference-score.js';
import * as scout from '../../scripts/eroscripts-scout-lib.mjs';
import {
  applyPreferenceScores, planPreferenceScores, sameScores, sceneCandidate, PREFERENCE_SCORES_KEY,
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
