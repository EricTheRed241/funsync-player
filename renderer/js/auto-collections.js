// Collections made from what the library already knows about each video:
// the EroScripts post tags (`library.scriptTags`), the duration
// (`library.durationCache`) and the video's title (its file name). After
// each library scan the AUTO_COLLECTIONS of content-preferences.js (medium,
// session length, franchise, script style, source) are kept up to date.
//
// The user stays in charge, as with the EroScripts categories and the
// artist playlists: a collection of the same name is reused, a collection
// is made once and only when something matches (a deleted one is not made
// again), and each video is added once (taking it out by hand sticks).
// That history is `library.autoCollections`: `{created: [key], added:
// {videoPath: [key]}}`.

import { FAPTAP_SOURCE } from './eroscripts-links.js';
import { SCRIPT_TAGS_KEY } from './script-tags.js';
import { AUTO_COLLECTIONS } from './content-preferences.js';

export const AUTO_COLLECTIONS_KEY = 'library.autoCollections';
export const COLLECTIONS_KEY = 'library.collections';
const DURATION_KEY = 'library.durationCache';

/**
 * A collection definition as a test on a video: any of its tags; its title
 * pattern as whole words; its length (the post's `len-*` tags while the
 * duration isn't known); or FapTap as the source.
 */
function matcher({ tags = [], title, length, lengthTags = [], fapTap }) {
  const titleRe = title ? new RegExp(`\\b(?:${title})\\b`) : null;
  return (v) => {
    if (fapTap) return v.source === FAPTAP_SOURCE;
    if (length) {
      if (!(v.duration > 0)) return lengthTags.some((t) => v.tags.has(t));
      return length.under !== undefined ? v.duration < length.under : v.duration >= length.atLeast;
    }
    return tags.some((t) => v.tags.has(t)) || !!titleRe?.test(v.title || '');
  };
}

const DEFINITIONS = AUTO_COLLECTIONS.map((def) => ({ key: def.key, name: def.name, match: matcher(def) }));

/**
 * A video's title as the collections match it: the file name without its
 * extension, lower-cased, underscores as spaces.
 *
 * @param {string} videoPath
 */
export function titleOf(videoPath) {
  const name = String(videoPath || '').split(/[\\/]/).pop();
  const dot = name.lastIndexOf('.');
  return (dot > 0 ? name.slice(0, dot) : name).normalize('NFC').toLocaleLowerCase().replace(/_+/g, ' ');
}

const nameKey = (s) => String(s || '').trim().toLocaleLowerCase();

/**
 * What to create and add.
 *
 * @param {object} o
 * @param {{id: string, name: string, videoPaths?: string[]}[]} o.collections
 * @param {{path: string, tags: Set<string>, title?: string, source?: string|null, duration?: number}[]} o.videos
 * @param {{created?: string[], added?: Object<string, string[]>}} [o.state] `library.autoCollections`
 * @returns {{create: {key: string, name: string, videoPaths: string[]}[], extend: {id: string, videoPaths: string[]}[],
 *   state: {created: string[], added: Object<string, string[]>}}} `extend` holds the videos to append
 */
export function planAutoCollections({ collections = [], videos = [], state = {} }) {
  const byName = new Map(collections.map((c) => [nameKey(c.name), c]));
  const created = new Set(state.created || []);
  const added = { ...(state.added || {}) };
  const create = [];
  const extend = [];
  for (const def of DEFINITIONS) {
    const existing = byName.get(nameKey(def.name));
    if (!existing && created.has(def.key)) continue;
    const inList = new Set(existing?.videoPaths || []);
    const fresh = videos
      .filter((v) => !(added[v.path] || []).includes(def.key) && def.match(v))
      .map((v) => v.path);
    if (!existing && !fresh.length) continue;
    created.add(def.key);
    for (const p of fresh) added[p] = [...(added[p] || []), def.key];
    const toAdd = fresh.filter((p) => !inList.has(p));
    if (!existing) create.push({ key: def.key, name: def.name, videoPaths: toAdd });
    else if (toAdd.length) extend.push({ id: existing.id, videoPaths: toAdd });
  }
  return { create, extend, state: { created: [...created], added } };
}

/**
 * Create and fill the collections for `videoPaths`, with one settings write.
 *
 * @param {{get: Function, set: Function}} data DataService
 * @param {string[]} videoPaths
 * @param {() => string} [newId]
 * @returns {{created: string[], extended: number}} names of new collections, count of grown ones
 */
export function applyAutoCollections(data, videoPaths, newId = () => crypto.randomUUID()) {
  const scriptTags = data.get(SCRIPT_TAGS_KEY) || {};
  const durations = data.get(DURATION_KEY) || {};
  const videos = videoPaths.map((path) => ({
    path, title: titleOf(path), tags: new Set(scriptTags[path]?.tags || []), source: scriptTags[path]?.source || null, duration: Number(durations[path]) || 0,
  }));
  const collections = data.get(COLLECTIONS_KEY) || [];
  const plan = planAutoCollections({ collections, videos, state: data.get(AUTO_COLLECTIONS_KEY) || {} });
  if (plan.create.length || plan.extend.length) {
    const grow = new Map(plan.extend.map((e) => [e.id, e.videoPaths]));
    const next = collections.map((c) => (grow.has(c.id) ? { ...c, videoPaths: [...(c.videoPaths || []), ...grow.get(c.id)] } : c));
    for (const c of plan.create) next.push({ id: newId(), name: c.name, videoPaths: c.videoPaths });
    data.set(COLLECTIONS_KEY, next);
  }
  if (JSON.stringify(plan.state) !== JSON.stringify(data.get(AUTO_COLLECTIONS_KEY) || {})) data.set(AUTO_COLLECTIONS_KEY, plan.state);
  return { created: plan.create.map((c) => c.name), extended: plan.extend.length };
}
