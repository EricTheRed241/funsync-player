// Preference score: how well a post, a candidate or a library scene fits the
// user's preferences, with the RANKING weights in content-preferences.js.
//
// One implementation for everything that ranks: the EroScripts and FapTap
// scouts order their candidates by it (scripts/eroscripts-scout-lib.mjs
// re-exports it), and the app scores every library scene with it after each
// scan (library-scores.js), so a scene ranks in the library as its post did
// in the scouting. Pure and renderer-safe: no Node imports.

import {
  FEET_TAGS, FEET_TITLE, FLAT_HENTAI_TAGS, MONSTER_TAGS, MONSTER_TITLE, RANKING, THREE_D_TAGS, THREE_D_TITLE, WEREWOLF_TOKEN,
} from './content-preferences.js';

/** True when a post's tags or title name a monster (tentacles, werewolves, horse guys, orcs …). */
export function hasMonster({ tags = [], title = '' }) {
  return tags.some((t) => MONSTER_TAGS.has(t)) || MONSTER_TITLE.test(title) || WEREWOLF_TOKEN.test(title);
}

/** True when a post's tags or title are about feet (footjob, soles, toes …). */
export function hasFeet({ tags = [], title = '' }) {
  return tags.some((t) => FEET_TAGS.has(t)) || FEET_TITLE.test(title);
}

/**
 * The not-preferred themes a post names, by tags, title words and (when
 * given) its first post: a monster, feet. Neither keeps a post out; each
 * ranks it lower (RANKING.monster, RANKING.feet).
 *
 * @param {{tags?: string[], title?: string, text?: string}} post
 */
export function lessPreferred({ tags = [], title = '', text = '' }) {
  const s = String(text || '');
  return {
    monster: hasMonster({ tags, title }) || MONSTER_TITLE.test(s) || WEREWOLF_TOKEN.test(s),
    feet: hasFeet({ tags, title }) || FEET_TITLE.test(s),
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
export function scoreCandidate({ vr = false, music = false, artistInLibrary = false, preferred = [], multiAxis = false, likes = 0, hentaiOnly = false, monster = false, feet = false }) {
  return (vr ? RANKING.vr : 0)
    + (music ? RANKING.music : 0)
    + (artistInLibrary ? RANKING.artistInLibrary : 0)
    + (preferred.length ? RANKING.preferred + RANKING.preferredExtra * (preferred.length - 1) : 0)
    + (multiAxis ? RANKING.multiAxis : 0)
    + Math.min(RANKING.likesMax, likes / RANKING.likesPer)
    - (hentaiOnly ? RANKING.hentaiOnly : 0)
    - (monster ? RANKING.monster : 0)
    - (feet ? RANKING.feet : 0);
}

/**
 * Score of a saved candidate (either scout) or a library scene under the
 * current RANKING, from what the list keeps, so a weight change reorders
 * lists saved before it. Monster / feet come from its tags and title, the
 * first-post flags saved with it (`monster`, `feet`), or an older
 * `--recheck` mark. Two decimals.
 */
export function candidateScore(c) {
  const named = lessPreferred(c);
  return +scoreCandidate({
    vr: !!c.vr, music: !!c.music, artistInLibrary: !!(c.artistInLibrary || c.favourite), preferred: c.preferred || [],
    multiAxis: c.axisType === 'multi', likes: Number(c.likes) || 0, hentaiOnly: isHentaiOnly(c),
    monster: named.monster || !!c.monster || c.contentBreak === 'monster', feet: named.feet || !!c.feet || c.contentBreak === 'feet',
  }).toFixed(2);
}
