// Score lists (user, 2026-11-10): one automatic collection per preference
// score band (SCORE_BANDS in content-preferences.js: "Score 15+",
// "Score 10–14.5", "Score below 10"), so the best-scored scenes are one tap
// away on the desktop and in the web remote's Collections tab.
//
// Unlike the other auto collections (auto-collections.js), a band follows
// the scores: it holds exactly the scanned videos whose current score
// (`library.preferenceScores`) falls in it, highest score first, and is
// rewritten whenever the scores change (each scan, a tag score or favourite
// artist change, Recalculate scores). A video taken out by hand comes back
// on the next refresh while its score stays in the band. The history is
// shared with the other auto collections (`library.autoCollections.created`
// holds the band's key once made), so a band collection the user deletes
// isn't made again; one with the same name is reused.

import { SCORE_BANDS } from './content-preferences.js';
import { AUTO_COLLECTIONS_KEY, COLLECTIONS_KEY, titleOf } from './auto-collections.js';
import { PREFERENCE_SCORES_KEY } from './library-scores.js';

const nameKey = (s) => String(s || '').trim().toLocaleLowerCase();

/**
 * The SCORE_BANDS entry a score falls in, or null (unscored or not a number).
 *
 * @param {unknown} score
 * @returns {{key: string, name: string, min?: number, below?: number}|null}
 */
export function scoreBandOf(score) {
  if (typeof score !== 'number' || !Number.isFinite(score)) return null;
  return SCORE_BANDS.find((b) => (b.min === undefined || score >= b.min) && (b.below === undefined || score < b.below)) || null;
}

/**
 * `items` by preference score, highest first, unscored last. Stable: ties
 * keep the order they came in. Returns a new array. For lists with no order
 * of their own (categories); the library sorts through sortVideos.
 *
 * @template T
 * @param {T[]} items
 * @param {(item: T) => string} pathOf
 * @param {Object<string, number>} scores `library.preferenceScores`
 * @returns {T[]}
 */
export function byScore(items, pathOf, scores) {
  const scoreOf = (item) => {
    const s = scores?.[pathOf(item)];
    return typeof s === 'number' && Number.isFinite(s) ? s : null;
  };
  return (items || [])
    .map((item, i) => ({ item, i, s: scoreOf(item) }))
    .sort((a, b) => ((a.s === null) - (b.s === null)) || ((b.s ?? 0) - (a.s ?? 0)) || (a.i - b.i))
    .map((x) => x.item);
}

/**
 * What each band collection should hold.
 *
 * @param {object} o
 * @param {{id: string, name: string, videoPaths?: string[]}[]} o.collections `library.collections`
 * @param {string[]} o.videoPaths the scanned videos
 * @param {Object<string, number>} o.scores `library.preferenceScores`
 * @param {{created?: string[]}} [o.state] `library.autoCollections`
 * @returns {{create: {key: string, name: string, videoPaths: string[]}[], update: {id: string, videoPaths: string[]}[],
 *   state: object}} `update` replaces a collection's videos; `state` keeps the other auto collections' history
 */
export function planScoreBandCollections({ collections = [], videoPaths = [], scores = {}, state = {} }) {
  const members = new Map(SCORE_BANDS.map((b) => [b.key, []]));
  for (const path of videoPaths) {
    const band = scoreBandOf(scores[path]);
    if (band) members.get(band.key).push(path);
  }
  const titles = new Map();
  const title = (p) => {
    if (!titles.has(p)) titles.set(p, titleOf(p));
    return titles.get(p);
  };
  for (const list of members.values()) {
    list.sort((a, b) => (scores[b] - scores[a]) || title(a).localeCompare(title(b)) || (a < b ? -1 : a > b ? 1 : 0));
  }

  const byName = new Map(collections.map((c) => [nameKey(c.name), c]));
  const created = new Set(state.created || []);
  const create = [];
  const update = [];
  for (const band of SCORE_BANDS) {
    const want = members.get(band.key);
    const existing = byName.get(nameKey(band.name));
    if (!existing) {
      if (created.has(band.key) || !want.length) continue;
      created.add(band.key);
      create.push({ key: band.key, name: band.name, videoPaths: want });
      continue;
    }
    created.add(band.key);
    const have = existing.videoPaths || [];
    if (have.length !== want.length || have.some((p, i) => p !== want[i])) update.push({ id: existing.id, videoPaths: want });
  }
  return { create, update, state: { ...state, created: [...created] } };
}

/**
 * Bring the band collections in line with the stored scores, with one
 * settings write per changed key.
 *
 * @param {{get: Function, set: Function}} data DataService
 * @param {string[]} videoPaths the scanned videos
 * @param {() => string} [newId]
 * @returns {{created: string[], updated: number}} names of new collections, count of rewritten ones
 */
export function applyScoreBandCollections(data, videoPaths, newId = () => crypto.randomUUID()) {
  const collections = data.get(COLLECTIONS_KEY) || [];
  const previous = data.get(AUTO_COLLECTIONS_KEY) || {};
  const plan = planScoreBandCollections({
    collections, videoPaths, scores: data.get(PREFERENCE_SCORES_KEY) || {}, state: previous,
  });
  if (plan.create.length || plan.update.length) {
    const replace = new Map(plan.update.map((u) => [u.id, u.videoPaths]));
    const next = collections.map((c) => (replace.has(c.id) ? { ...c, videoPaths: replace.get(c.id) } : c));
    for (const c of plan.create) next.push({ id: newId(), name: c.name, videoPaths: c.videoPaths });
    data.set(COLLECTIONS_KEY, next);
  }
  if ((previous.created || []).length !== plan.state.created.length) data.set(AUTO_COLLECTIONS_KEY, plan.state);
  return { created: plan.create.map((c) => c.name), updated: plan.update.length };
}
