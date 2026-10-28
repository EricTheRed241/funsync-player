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
  candidateScore, DEFAULT_TAG_SCORES as D, effectiveTagScores, normalizeTagScore, tagKey, tagScoreSum, TAG_SCORE_MAX, TAG_SCORE_STEP,
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
    expect(score({ title: post.title, tags: post.tags, likes: 40, vr: true, axisKind: 'handmade', artistInLibrary: true }))
      .toBe(candidateScore(post));
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

  it('takes the monster and feet scores off, by tag or title, once per group', () => {
    expect(score({ title: 'Scene', tags: ['tentacle'] })).toBe(D.tentacle);
    expect(score({ title: 'Werewolf WW scene' })).toBe(D.monster);
    expect(score({ title: 'Scene - footjob' })).toBe(D.footjob);
    expect(score({ title: 'Orc footjob', tags: ['feet'] })).toBe(D.monster + D.feet);
    expect(score({ title: 'Tentacle monster', tags: ['tentacle', 'monster', 'monstergirls'] })).toBe(D.monster);
  });

  it('starts creampie, riding, music and POV tags positive; synonyms count once, groups add up, whatever their case', () => {
    expect([D.creampie, D.riding, D.pmv, D.pov].every((s) => s > 0)).toBe(true);
    expect(score({ title: 'Scene', tags: ['PMV'] })).toBe(D.pmv);
    expect(score({ title: 'Scene', tags: ['pmv', 'hmv', 'cock-hero'] })).toBe(D.pmv);
    expect(score({ title: 'Scene', tags: ['Creampie', 'anal-creampie', 'riding', 'cowgirl', 'reverse-cowgirl'] })).toBe(D.creampie + D.riding);
    expect(score({ title: 'Scene', tags: ['pov', 'male-pov'] })).toBe(D.pov);
  });

  it('counts likes from the record, capped', () => {
    expect(score({ title: 'Scene', likes: RANKING.likesPer })).toBe(1);
    expect(score({ title: 'Scene', likes: 'many' })).toBe(0);
    expect(score({ title: 'Scene', likes: 1e6 })).toBe(RANKING.likesMax);
  });
});

describe('planPreferenceScores', () => {
  const plan = (o) => planPreferenceScores({ sourcePaths: SOURCES, ...o });

  it('gives a favourite\'s artist folder the library-creator bonus and the favourite bonus from one video', () => {
    const fav = scene('ScyllaHMV', 'One');
    const other = scene('Nobody', 'One');
    const o = { videos: [video(fav), video(other)], favourites: ['scyllahmv'], axisFlags: { [fav]: 'single', [other]: 'single' } };
    const s = plan(o);
    expect(s[fav] - s[other]).toBe(RANKING.artistInLibrary + RANKING.favouriteArtist);
    expect(plan({ ...o, favouriteScore: 1.5 })[fav]).toBe(RANKING.artistInLibrary + 1.5);
    // Bonus 0: the library-creator bonus only, as before favourites ranked higher.
    expect(plan({ ...o, favouriteScore: 0 })[fav]).toBe(RANKING.artistInLibrary);
  });

  it('matches a favourite to the artist folder whatever its case or Unicode form', () => {
    const fav = scene('Cafe\u0301 Girl', 'One');
    const s = plan({ videos: [video(fav, false)], favourites: ['  CAFÉ GIRL '] });
    expect(s[fav]).toBe(RANKING.artistInLibrary + RANKING.favouriteArtist);
    expect(plan({ videos: [video(fav, false)], favourites: ['Cafe Girl'] })[fav]).toBe(0);
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
    expect(s[a]).toBe(RANKING.vr + RANKING.multiAxis + D.creampie + 3);
    expect(s[b]).toBe(-RANKING.hentaiOnly);
    // No post tags: still scored from the title.
    expect(s[c]).toBe(D.monster + D.feet);
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
    expect(first).toEqual({ scores: { [a]: RANKING.multiAxis + D.riding }, changed: true });
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
  it('adds up ungrouped tags one by one, negative ones too; tags without a score count 0', () => {
    const scores = { blowjob: 2, ahegao: -1.5, '3d': 0.5 };
    expect(tagScoreSum(['blowjob', '3d', 'ahegao', 'sfm'], scores)).toBe(1);
    expect(tagScoreSum(['blowjob', 'Blowjob', ' BLOWJOB '], scores)).toBe(2);
    expect(tagScoreSum(['3d', 'sfm'], {})).toBe(0);
    expect(tagScoreSum([], scores)).toBe(0);
    expect(tagScoreSum(['sfm'], undefined)).toBe(0);
    expect(tagScoreSum(undefined, scores)).toBe(0);
    expect(tagScoreSum(['toString', 'constructor'], {})).toBe(0);
  });

  it('counts a group once: its highest positive plus its lowest negative score', () => {
    expect(tagScoreSum(['riding', 'cowgirl'], { cowgirl: 3 })).toBe(3);
    expect(tagScoreSum(['riding', 'cowgirl'], { riding: 1 })).toBe(D.cowgirl);
    expect(tagScoreSum(['riding', 'cowgirl'], { cowgirl: -1 })).toBe(D.riding - 1);
    expect(tagScoreSum(['riding', 'cowgirl', 'reverse-cowgirl'], { cowgirl: -1, 'reverse-cowgirl': -2 })).toBe(D.riding - 2);
    expect(tagScoreSum(['creampie', 'riding', 'blowjob'], { blowjob: 1 })).toBe(D.creampie + D.riding + 1);
  });

  it('starts every grouped tag at its group score; the user\'s change wins, an explicit 0 included', () => {
    expect(effectiveTagScores({})).toEqual(D);
    expect(effectiveTagScores(undefined)).toEqual(D);
    const e = effectiveTagScores({ Creampie: 0, ahegao: -2, pov: 'x' });
    expect([e.creampie, e.ahegao, e.pov]).toEqual([undefined, -2, D.pov]);
    expect(tagScoreSum(['creampie'], { creampie: 0 })).toBe(0);
    expect(tagScoreSum(['creampie', 'anal-creampie'], { creampie: 0 })).toBe(D['anal-creampie']);
  });

  it('counts a group the title or first post names at its first tag\'s score, unless one of its tags is there', () => {
    expect(tagScoreSum([], {}, { monster: true })).toBe(D.monster);
    expect(tagScoreSum([], { monster: -2 }, { monster: true })).toBe(-2);
    expect(tagScoreSum(['tentacle'], { tentacle: -3 }, { monster: true })).toBe(-3);
    expect(tagScoreSum([], {}, { feet: true, ffm: true })).toBe(D.footjob + D.ffm);
    expect(tagScoreSum([], {}, { monster: false })).toBe(0);
  });

  it('rounds scores to 0.5 steps (halves up) and clamps them to ±TAG_SCORE_MAX; anything else is not a score', () => {
    expect([TAG_SCORE_MAX, TAG_SCORE_STEP]).toEqual([10, 0.5]);
    expect([1.5, 1.2, 1.25, -1.25, -1.3, 0.24, '2.5', 11, -99, 10, -10].map(normalizeTagScore))
      .toEqual([1.5, 1, 1.5, -1, -1.5, 0, 2.5, 10, -10, 10, -10]);
    expect(Object.is(normalizeTagScore(-0.2), 0)).toBe(true);
    expect([NaN, Infinity, '', ' ', 'x', null, undefined, true, {}, []].map(normalizeTagScore)).toEqual(Array(10).fill(null));
    expect(tagScoreSum(['blowjob', 'ahegao'], { blowjob: 25, ahegao: 'abc' })).toBe(10);
  });

  it('keys tags trimmed and lower case', () => {
    expect([tagKey(' Riding '), tagKey('PMV'), tagKey(3), tagKey(null)]).toEqual(['riding', 'pmv', '', '']);
  });

  it('adds the tag scores on top of the RANKING score', () => {
    const post = { title: 'Tracer rides', tags: ['3d', 'riding', 'ahegao'], likes: 30, vr: true, axisType: 'multi' };
    const base = candidateScore(post);
    expect(candidateScore(post, {})).toBe(base);
    expect(candidateScore(post, { tagScores: { blowjob: 4 } })).toBe(base);
    expect(candidateScore(post, { tagScores: { ahegao: 1.5 } })).toBeCloseTo(base + 1.5, 10);
    expect(candidateScore(post, { tagScores: { riding: 4 } })).toBeCloseTo(base + 4 - D.riding, 10);
    expect(candidateScore(post, { tagScores: { '3d': -20 } })).toBeCloseTo(base - 10, 10);
  });

  it('scores library scenes with library.tagScores', () => {
    const a = scene('X', 'Riding scene');
    const b = scene('Y', 'Other scene');
    const scriptTags = { [a]: { tags: ['riding', 'Ahegao'] }, [b]: { tags: ['3d'] } };
    const base = { videos: [video(a), video(b)], sourcePaths: SOURCES, scriptTags, axisFlags: { [a]: 'generated', [b]: 'generated' } };
    const without = planPreferenceScores(base);
    const withScores = planPreferenceScores({ ...base, tagScores: { riding: 1, ahegao: -3, '3d': 0.5 } });
    expect(withScores[a]).toBe(without[a] + (1 - D.riding) - 3);
    expect(withScores[b]).toBe(without[b] + 0.5);

    const store = { 'library.sources': SOURCES.map((path) => ({ path })), 'library.scriptTags': scriptTags };
    const data = { get: (k) => store[k], set: (k, v) => { store[k] = v; } };
    applyPreferenceScores(data, base.videos, { axisFlags: base.axisFlags });
    expect(store[PREFERENCE_SCORES_KEY]).toEqual(without);
    store[TAG_SCORES_KEY] = { riding: 1, ahegao: -3, '3d': 0.5 };
    expect(applyPreferenceScores(data, base.videos, { axisFlags: base.axisFlags })).toEqual({ scores: withScores, changed: true });
  });
});

describe('favourite artist bonus', () => {
  const post = { title: 'Tracer rides', tags: ['3d', 'riding'], likes: 20, vr: true, axisType: 'multi' };

  it('adds the bonus on top of the library-creator bonus; 0 gives the score without it', () => {
    const plain = candidateScore(post);
    const known = candidateScore({ ...post, artistInLibrary: 'ScyllaHMV' });
    expect(known).toBe(plain + RANKING.artistInLibrary);
    expect(candidateScore({ ...post, favourite: 'ScyllaHMV' })).toBe(known + RANKING.favouriteArtist);
    expect(candidateScore({ ...post, favourite: 'ScyllaHMV' }, { favouriteScore: 2.5 })).toBe(known + 2.5);
    expect(candidateScore({ ...post, favourite: 'ScyllaHMV' }, { favouriteScore: -1 })).toBe(known - 1);
    expect(candidateScore({ ...post, favourite: 'ScyllaHMV' }, { favouriteScore: 0 })).toBe(known);
    expect(candidateScore({ ...post, favourite: null }, { favouriteScore: 9 })).toBe(plain);
  });

  it('scores library scenes with the favourites and bonus stored in the config', () => {
    const fav = scene('Shayuki', 'One');
    const builtIn = scene('ScyllaHMV', 'One');
    const videos = [video(fav, false), video(builtIn, false)];
    const store = { 'library.sources': SOURCES.map((path) => ({ path })) };
    const data = { get: (k) => store[k], set: (k, v) => { store[k] = v; } };
    const bonus = RANKING.artistInLibrary + RANKING.favouriteArtist;
    expect(applyPreferenceScores(data, videos).scores).toEqual({ [fav]: 0, [builtIn]: bonus });
    store['library.favouriteArtists'] = { shayuki: true, scyllahmv: false };
    store['library.favouriteArtistScore'] = 3;
    expect(applyPreferenceScores(data, videos).scores).toEqual({ [fav]: RANKING.artistInLibrary + 3, [builtIn]: 0 });
  });
});
