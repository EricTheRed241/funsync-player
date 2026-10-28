// Preference score: how well a post, a candidate or a library scene fits the
// user's preferences, with the RANKING weights in content-preferences.js.
//
// One implementation for everything that ranks: the EroScripts and FapTap
// scouts order their candidates by it (scripts/eroscripts-scout-lib.mjs
// re-exports it), and the app scores every library scene with it after each
// scan (library-scores.js), so a scene ranks in the library as its post did
// in the scouting. On top of the RANKING weights, the user's own score per
// post tag (`library.tagScores`, set from the web remote's Tags view) adds
// up over a candidate's tags; callers pass that map in. Pure and
// renderer-safe: no Node imports.

import {
  FEET_TAGS, FEET_TITLE, FFM_TAGS, FFM_TITLE, FLAT_HENTAI_TAGS, MONSTER_TAGS, MONSTER_TITLE, RANKING, THREE_D_TAGS, THREE_D_TITLE, WEREWOLF_TOKEN,
} from './content-preferences.js';

/** Tag scores run from -TAG_SCORE_MAX to +TAG_SCORE_MAX in TAG_SCORE_STEP steps (backend copy: services/library_edits.py). */
export const TAG_SCORE_MAX = 10;
export const TAG_SCORE_STEP = 0.5;

/** The key a tag's score is stored under: trimmed, lower case. */
export function tagKey(tag) {
  return typeof tag === 'string' ? tag.trim().toLowerCase() : '';
}

/**
 * `value` as a tag score: a finite number (or numeric string) rounded to the
 * nearest TAG_SCORE_STEP (halves up) and clamped to ±TAG_SCORE_MAX; null for
 * anything else.
 */
export function normalizeTagScore(value) {
  if (typeof value === 'string' && !value.trim()) return null;
  if (typeof value !== 'number' && typeof value !== 'string') return null;
  const n = Number(value);
  if (!Number.isFinite(n)) return null;
  const stepped = Math.floor(n / TAG_SCORE_STEP + 0.5) * TAG_SCORE_STEP;
  return Math.min(TAG_SCORE_MAX, Math.max(-TAG_SCORE_MAX, stepped)) + 0;
}

/**
 * Sum of the user's tag scores over a post's distinct tags (case-insensitive);
 * a tag without a score counts 0.
 *
 * @param {string[]} tags
 * @param {Object<string, number>} [tagScores] `library.tagScores`
 */
export function tagScoreSum(tags, tagScores) {
  if (!tagScores || !Array.isArray(tags)) return 0;
  let sum = 0;
  for (const key of new Set(tags.map(tagKey))) {
    if (key && Object.hasOwn(tagScores, key)) sum += normalizeTagScore(tagScores[key]) ?? 0;
  }
  return sum;
}

/** True when a post's tags or title name a monster (tentacles, werewolves, horse guys, orcs …). */
export function hasMonster({ tags = [], title = '' }) {
  return tags.some((t) => MONSTER_TAGS.has(t)) || MONSTER_TITLE.test(title) || WEREWOLF_TOKEN.test(title);
}

/** True when a post's tags or title are about feet (footjob, soles, toes …). */
export function hasFeet({ tags = [], title = '' }) {
  return tags.some((t) => FEET_TAGS.has(t)) || FEET_TITLE.test(title);
}

/** True when a post's tags or title say FFM (two women, one man). */
export function hasFfm({ tags = [], title = '' }) {
  return tags.some((t) => FFM_TAGS.has(t)) || FFM_TITLE.test(title);
}

/**
 * The not-preferred themes a post names, by tags, title words and (when
 * given) its first post: a monster, feet, FFM. None keeps a post out; each
 * ranks it lower (RANKING.monster, RANKING.feet, RANKING.ffm).
 *
 * @param {{tags?: string[], title?: string, text?: string}} post
 */
export function lessPreferred({ tags = [], title = '', text = '' }) {
  const s = String(text || '');
  return {
    monster: hasMonster({ tags, title }) || MONSTER_TITLE.test(s) || WEREWOLF_TOKEN.test(s),
    feet: hasFeet({ tags, title }) || FEET_TITLE.test(s),
    ffm: hasFfm({ tags, title }) || FFM_TITLE.test(s),
  };
}

/**
 * True for 2D hentai: animation tags only hentai / 2d / anime, no 3D tag
 * (3d, sfm, vam, mmd, cgi, blender, koikatsu …) and no 3D / SFM / VaM / MMD
 * in the title. Such posts rank lower (RANKING.hentaiOnly).
 *
 * @param {{tags?: string[], title?: string}} post
 */
export function isHentaiOnly({ tags = [], title = '' }) {
  return tags.some((t) => FLAT_HENTAI_TAGS.has(t)) && !tags.some((t) => THREE_D_TAGS.has(t)) && !THREE_D_TITLE.test(title);
}

/** Ranking score; higher downloads first. */
export function scoreCandidate({ vr = false, music = false, artistInLibrary = false, preferred = [], multiAxis = false, likes = 0, hentaiOnly = false, monster = false, feet = false, ffm = false }) {
  return (vr ? RANKING.vr : 0)
    + (music ? RANKING.music : 0)
    + (artistInLibrary ? RANKING.artistInLibrary : 0)
    + (preferred.length ? RANKING.preferred + RANKING.preferredExtra * (preferred.length - 1) : 0)
    + (multiAxis ? RANKING.multiAxis : 0)
    + Math.min(RANKING.likesMax, likes / RANKING.likesPer)
    - (hentaiOnly ? RANKING.hentaiOnly : 0)
    - (monster ? RANKING.monster : 0)
    - (feet ? RANKING.feet : 0)
    - (ffm ? RANKING.ffm : 0);
}

/**
 * Score of a saved candidate (either scout) or a library scene under the
 * current RANKING, from what the list keeps, so a weight change reorders
 * lists saved before it, plus the user's tag scores over its tags. Monster /
 * feet / FFM come from its tags and title, the first-post flags saved with
 * it (`monster`, `feet`, `ffm`), or an older `--recheck` mark. Two decimals.
 *
 * @param {object} c
 * @param {Object<string, number>} [tagScores] `library.tagScores` (tag → score)
 */
export function candidateScore(c, tagScores) {
  const named = lessPreferred(c);
  return +(scoreCandidate({
    vr: !!c.vr, music: !!c.music, artistInLibrary: !!(c.artistInLibrary || c.favourite), preferred: c.preferred || [],
    multiAxis: c.axisType === 'multi', likes: Number(c.likes) || 0, hentaiOnly: isHentaiOnly(c),
    monster: named.monster || !!c.monster || c.contentBreak === 'monster', feet: named.feet || !!c.feet || c.contentBreak === 'feet',
    ffm: named.ffm || !!c.ffm,
  }) + tagScoreSum(c.tags, tagScores)).toFixed(2);
}
