// Categories made from EroScripts post tags, plus VR from the video itself.
//
// Every video whose script came from an EroScripts post has the post's tags
// in `library.scriptTags` (downloads from the app, `eroscripts:sync` links and
// Discovery sidecars; FapTap scenes bring their FapTap tags, same slugs). A
// fixed set of categories (Creampie, Riding, Blowjob, PMV / HMV …) is
// filled from those tags after each library scan, so the desktop filters,
// the web remote and HereSphere (`Category:<name>` tags) can use them.
//
// VR is not taken from tags: posts tagged "2D/VR" put flat videos in it and
// untagged VR videos were missing. The backend decides per video (the VR
// Format panel's projection, the Mark as VR/Flat override, filename markers,
// then the frame shape) and the VR category follows that (`planVRCategory`).
//
// The user stays in charge: each category is created once (a deleted one is
// not made again), an existing category with the same name is reused, and
// each video is put in each category once (taking it out by hand sticks).
// That history is `library.autoCategories`: `{created: [key], assigned:
// {videoPath: [key]}}`. For VR, `vr` marks a video the app put there (taken
// out again when the video turns out flat) and `vr-manual` one that was
// already in the category when found to be VR (never taken out by the app).

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
  // Filled from the backend's VR decision, never from tags (planVRCategory).
  { key: 'vr', name: 'VR', color: '#3498db', tags: [] },
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

const VR_KEY = 'vr';
const VR_MANUAL_KEY = 'vr-manual';

/**
 * Follow the backend's VR decision in the VR category.
 *
 * A VR video is added once (taken out by hand sticks); a video already in
 * the category is left as the user's. A video the app put there (by flag or
 * by the old post-tag rule) that is now decided flat is taken out. Videos
 * missing from `flags` (frame size still unknown) are left alone.
 *
 * @param {object} o
 * @param {{id: string, name: string}[]} o.categories
 * @param {Object<string, string[]>} o.videoCategories videoPath → category ids
 * @param {Object<string, boolean>} o.flags videoPath → VR, from `GET /api/media/vr-flags`
 * @param {{created?: string[], assigned?: Object<string, string[]>}} [o.state] `library.autoCategories`
 * @returns {{add: [string, string][], remove: [string, string][], state: object}}
 */
export function planVRCategory({ categories = [], videoCategories = {}, flags = {}, state = {} }) {
  const def = AUTO_CATEGORIES.find((d) => d.key === VR_KEY);
  const cat = categories.find((c) => nameKey(c.name) === nameKey(def.name));
  if (!cat) return { add: [], remove: [], state };
  const assigned = { ...(state.assigned || {}) };
  const add = [];
  const remove = [];
  for (const [videoPath, isVR] of Object.entries(flags)) {
    const done = assigned[videoPath] || [];
    const inCategory = (videoCategories[videoPath] || []).includes(cat.id);
    if (isVR === true) {
      if (done.includes(VR_KEY) || done.includes(VR_MANUAL_KEY)) continue;
      if (inCategory) {
        assigned[videoPath] = [...done, VR_MANUAL_KEY];
      } else {
        add.push([videoPath, cat.id]);
        assigned[videoPath] = [...done, VR_KEY];
      }
    } else if (isVR === false && done.includes(VR_KEY) && inCategory) {
      remove.push([videoPath, cat.id]);
      const rest = done.filter((k) => k !== VR_KEY);
      if (rest.length) assigned[videoPath] = rest;
      else delete assigned[videoPath];
    }
  }
  return { add, remove, state: { ...state, assigned } };
}

/**
 * Apply `planVRCategory` to the library's categories.
 *
 * @param {object} data DataService (get, set, getCategories, assignCategories, unassignCategory, _cache.videoCategories)
 * @param {Object<string, boolean>} flags
 * @returns {{added: number, removed: number}}
 */
export function applyVRCategory(data, flags) {
  const plan = planVRCategory({
    categories: data.getCategories(),
    videoCategories: data._cache?.videoCategories || {},
    flags: flags || {},
    state: data.get(AUTO_CATEGORIES_KEY) || {},
  });
  if (plan.add.length) data.assignCategories(plan.add);
  for (const [videoPath, catId] of plan.remove) data.unassignCategory(videoPath, catId);
  const before = JSON.stringify(data.get(AUTO_CATEGORIES_KEY) || {});
  if (JSON.stringify(plan.state) !== before) data.set(AUTO_CATEGORIES_KEY, plan.state);
  return { added: plan.add.length, removed: plan.remove.length };
}
