// Preference score: how well a post, a candidate or a library scene fits the
// user's preferences: the RANKING weights in content-preferences.js (VR,
// hand-made axes, known artist, likes, 2D hentai) plus the tag scores.
//
// One implementation for everything that ranks: the EroScripts and FapTap
// scouts order their candidates by it (scripts/eroscripts-scout-lib.mjs
// re-exports it), and the app scores every library scene with it after each
// scan (library-scores.js), so a scene ranks in the library as its post did
// in the scouting. Tag scores start from TAG_SCORE_GROUPS (creampie, riding,
// music, POV, monster, feet, FFM) and the user changes them per tag
// (`library.tagScores`, set from the web remote's Tags view); callers pass
// that map in. Pure and renderer-safe: no Node imports.

import {
  FEET_TAGS, FEET_TITLE, FFM_TAGS, FFM_TITLE, FLAT_HENTAI_TAGS, MONSTER_TAGS, MONSTER_TITLE, RANKING, TAG_SCORE_GROUPS, THREE_D_TAGS, THREE_D_TITLE,
  WEREWOLF_TOKEN,
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

/** Grouped tag → its TAG_SCORE_GROUPS entry. */
const GROUP_OF = new Map(TAG_SCORE_GROUPS.flatMap((g) => g.tags.map((t) => [tagKey(t), g])));

/** Every grouped tag's starting score (its group's). */
export const DEFAULT_TAG_SCORES = Object.freeze(Object.fromEntries([...GROUP_OF].map(([tag, g]) => [tag, g.score])));

/**
 * What the web remote's Tags view shows besides the scores: each grouped
 * tag's group name, and the built-in RANKING weights that aren't about a
 * tag (read-only there).
 */
export function tagScoreInfo() {
  return {
    groups: Object.fromEntries([...GROUP_OF].map(([tag, g]) => [tag, g.name])),
    rules: { multiAxis: RANKING.multiAxis, vr: RANKING.vr, artistInLibrary: RANKING.artistInLibrary, likes: RANKING.likesMax, hentaiOnly: -RANKING.hentaiOnly },
  };
}

/**
 * Every tag's score: DEFAULT_TAG_SCORES with the user's changes
 * (`library.tagScores`, an explicit 0 included) on top. Tags at 0 are left
 * out.
 *
 * @param {Object<string, number>} [tagScores] `library.tagScores`
 * @returns {Object<string, number>}
 */
export function effectiveTagScores(tagScores) {
  const out = { ...DEFAULT_TAG_SCORES };
  if (tagScores && typeof tagScores === 'object') {
    for (const [tag, value] of Object.entries(tagScores)) {
      const key = tagKey(tag);
      const score = normalizeTagScore(value);
      if (key && score !== null) out[key] = score;
    }
  }
  for (const key of Object.keys(out)) if (out[key] === 0) delete out[key];
  return out;
}

/**
 * A post's tag score, over its distinct tags (case-insensitive), each at its
 * effectiveTagScores score. A TAG_SCORE_GROUPS group counts once: its
 * highest positive plus its lowest negative score. A group marked `named`
 * also counts, at its first tag's score, when `named[group.named]` holds and
 * none of its tags is there (the title or first post names it).
 *
 * @param {string[]} tags
 * @param {Object<string, number>} [tagScores] `library.tagScores`
 * @param {{monster?: boolean, feet?: boolean, ffm?: boolean}} [named]
 */
export function tagScoreSum(tags, tagScores, named = {}) {
  const effective = effectiveTagScores(tagScores);
  const scoreOf = (key) => (Object.hasOwn(effective, key) ? effective[key] : 0);
  const groups = new Map();  // group → [highest positive, lowest negative]
  const count = (group, score) => {
    const [high, low] = groups.get(group) || [0, 0];
    groups.set(group, [Math.max(high, score), Math.min(low, score)]);
  };
  let sum = 0;
  for (const key of new Set((Array.isArray(tags) ? tags : []).map(tagKey))) {
    if (!key) continue;
    const score = scoreOf(key);
    const group = GROUP_OF.get(key);
    if (group) count(group, score);
    else sum += score;
  }
  for (const g of TAG_SCORE_GROUPS) {
    if (g.named && named[g.named] && !groups.has(g)) count(g, scoreOf(tagKey(g.tags[0])));
  }
  for (const [high, low] of groups.values()) sum += high + low;
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
 * ranks it lower (its TAG_SCORE_GROUPS group).
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

/** The RANKING part of the score (everything but the tags); higher downloads first. */
export function scoreCandidate({ vr = false, artistInLibrary = false, multiAxis = false, likes = 0, hentaiOnly = false }) {
  return (vr ? RANKING.vr : 0)
    + (artistInLibrary ? RANKING.artistInLibrary : 0)
    + (multiAxis ? RANKING.multiAxis : 0)
    + Math.min(RANKING.likesMax, likes / RANKING.likesPer)
    - (hentaiOnly ? RANKING.hentaiOnly : 0);
}

/**
 * Score of a saved candidate (either scout) or a library scene under the
 * current RANKING and tag scores, from what the list keeps, so a weight
 * change reorders lists saved before it. Monster / feet / FFM named outside
 * the tags come from its title, the first-post flags saved with it
 * (`monster`, `feet`, `ffm`), or an older `--recheck` mark. Two decimals.
 *
 * @param {object} c
 * @param {Object<string, number>} [tagScores] `library.tagScores` (the user's changes)
 */
export function candidateScore(c, tagScores) {
  const named = lessPreferred(c);
  return +(scoreCandidate({
    vr: !!c.vr, artistInLibrary: !!(c.artistInLibrary || c.favourite), multiAxis: c.axisType === 'multi',
    likes: Number(c.likes) || 0, hentaiOnly: isHentaiOnly(c),
  }) + tagScoreSum(c.tags, tagScores, {
    monster: named.monster || !!c.monster || c.contentBreak === 'monster',
    feet: named.feet || !!c.feet || c.contentBreak === 'feet',
    ffm: named.ffm || !!c.ffm,
  })).toFixed(2);
}
