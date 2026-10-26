// Categories made from EroScripts post tags.
//
// Every video whose script came from an EroScripts post has the post's tags
// in `library.scriptTags` (downloads from the app, `eroscripts:sync` links and
// Discovery sidecars; FapTap scenes bring their FapTap tags, same slugs). A
// fixed set of categories (Creampie, Riding, Blowjob, VR, PMV / HMV …) is
// filled from those tags after each library scan, so the desktop filters,
// the web remote and HereSphere (`Category:<name>` tags) can use them.
//
// The user stays in charge: each category is created once (a deleted one is
// not made again), an existing category with the same name is reused, and
// each video is put in each category once (taking it out by hand sticks).
// That history is `library.autoCategories`: `{created: [key], assigned:
// {videoPath: [key]}}`.

import { SCRIPT_TAGS_KEY } from './script-tags.js';

export const AUTO_CATEGORIES_KEY = 'library.autoCategories';

/** Category definitions: EroScripts tags that put a video in each. */
export const AUTO_CATEGORIES = [
  { key: 'creampie', name: 'Creampie', color: '#e94560', tags: ['creampie', 'anal-creampie', 'oral-creampie'] },
  { key: 'riding', name: 'Riding', color: '#f39c12', tags: ['riding', 'cowgirl', 'reverse-cowgirl'] },
  { key: 'blowjob', name: 'Blowjob', color: '#ff6b81', tags: ['blowjob', 'bj', 'deepthroat', 'facefuck', 'throatpie', 'cum-in-mouth'] },
  { key: 'handjob', name: 'Handjob', color: '#fd79a8', tags: ['handjob', 'milking'] },
  { key: 'titjob', name: 'Titjob', color: '#e74c3c', tags: ['titfuck', 'titjob', 'paizuri', 'boobjob'] },
  { key: 'footjob', name: 'Footjob', color: '#1abc9c', tags: ['footjob'] },
  { key: 'anal', name: 'Anal', color: '#9b59b6', tags: ['anal', 'anal-creampie'] },
  { key: 'doggy', name: 'Doggy', color: '#3498db', tags: ['doggy', 'doggy-style', 'doggystyle', 'standing-doggy', 'pronebone'] },
  { key: 'missionary', name: 'Missionary', color: '#00cec9', tags: ['missionary'] },
  { key: 'group', name: 'Group', color: '#2ecc71', tags: ['gangbang', 'threesome', 'foursome', 'fivesome', 'orgy', 'double-penetration', 'harem'] },
  { key: 'monster', name: 'Monster', color: '#9b59b6', tags: ['monster', 'monstergirls', 'tentacle', 'werewolf'] },
  { key: 'pmv', name: 'PMV / HMV', color: '#f39c12', tags: ['pmv', 'hmv', 'cock-hero', 'fap-hero', 'music-video'] },
  { key: 'vr', name: 'VR', color: '#3498db', tags: ['vr'] },
  { key: 'pov', name: 'POV', color: '#00cec9', tags: ['pov', 'male-pov'] },
];

const nameKey = (s) => String(s || '').trim().toLocaleLowerCase();

/**
 * What to create and assign.
 *
 * @param {object} o
 * @param {{id: string, name: string}[]} o.categories
 * @param {Object<string, string[]>} o.videoCategories videoPath → category ids
 * @param {Object<string, {tags?: string[]}>} o.scriptTags videoPath → post record
 * @param {string[]} o.videoPaths the library's videos
 * @param {{created?: string[], assigned?: Object<string, string[]>}} [o.state] `library.autoCategories`
 * @returns {{create: object[], pairs: [string, string][], state: {created: string[], assigned: Object<string, string[]>}}}
 *   `create`: definitions to make; `pairs`: `[videoPath, categoryId]` to assign
 *   with the categories that exist; `state`: the history after both
 */
export function planAutoCategories({ categories = [], videoCategories = {}, scriptTags = {}, videoPaths = [], state = {} }) {
  const byName = new Map(categories.map((c) => [nameKey(c.name), c]));
  const created = new Set(state.created || []);
  const create = AUTO_CATEGORIES.filter((d) => !created.has(d.key) && !byName.has(nameKey(d.name)));
  for (const d of AUTO_CATEGORIES) created.add(d.key);

  const assigned = { ...(state.assigned || {}) };
  const pairs = [];
  for (const videoPath of videoPaths) {
    const tags = new Set(scriptTags[videoPath]?.tags || []);
    if (!tags.size) continue;
    const done = new Set(assigned[videoPath] || []);
    let changed = false;
    for (const d of AUTO_CATEGORIES) {
      if (done.has(d.key) || !d.tags.some((t) => tags.has(t))) continue;
      const cat = byName.get(nameKey(d.name));
      if (!cat) continue;
      if (!(videoCategories[videoPath] || []).includes(cat.id)) pairs.push([videoPath, cat.id]);
      done.add(d.key);
      changed = true;
    }
    if (changed) assigned[videoPath] = [...done];
  }
  return { create, pairs, state: { created: [...created], assigned } };
}

/**
 * Create the missing categories and fill them for `videoPaths`.
 *
 * @param {object} data DataService (get, set, getCategories, addCategory, assignCategories, _cache.videoCategories)
 * @param {string[]} videoPaths
 * @returns {Promise<{created: number, assigned: number}>}
 */
export async function applyAutoCategories(data, videoPaths) {
  const input = () => ({
    categories: data.getCategories(),
    videoCategories: data._cache?.videoCategories || {},
    scriptTags: data.get(SCRIPT_TAGS_KEY) || {},
    videoPaths,
    state: data.get(AUTO_CATEGORIES_KEY) || {},
  });
  const first = planAutoCategories(input());
  for (const d of first.create) await data.addCategory(d.name, d.color, null);
  const plan = first.create.length ? planAutoCategories(input()) : first;
  if (plan.pairs.length) data.assignCategories(plan.pairs);
  const before = JSON.stringify(data.get(AUTO_CATEGORIES_KEY) || {});
  if (JSON.stringify(plan.state) !== before) data.set(AUTO_CATEGORIES_KEY, plan.state);
  return { created: first.create.length, assigned: plan.pairs.length };
}
