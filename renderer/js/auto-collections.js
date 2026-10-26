// Collections made from what the library already knows about each video:
// the EroScripts post tags (`library.scriptTags`) and the duration
// (`library.durationCache`). After each library scan four themes are kept
// up to date: medium (Hentai, SFM, VaM …), session length (Quick, Sessions,
// Marathons), franchise (Overwatch, Genshin Impact …) and script style
// (Music / beat, Action).
//
// The user stays in charge, as with the EroScripts categories and the
// artist playlists: a collection of the same name is reused, a collection
// is made once and only when something matches (a deleted one is not made
// again), and each video is added once (taking it out by hand sticks).
// That history is `library.autoCollections`: `{created: [key], added:
// {videoPath: [key]}}`.

import { SCRIPT_TAGS_KEY } from './script-tags.js';

export const AUTO_COLLECTIONS_KEY = 'library.autoCollections';
export const COLLECTIONS_KEY = 'library.collections';
const DURATION_KEY = 'library.durationCache';

const anyTag = (...tags) => (v) => tags.some((t) => v.tags.has(t));
// Duration when known (seconds), else the post's `len-*` bucket.
const byLength = (test, tags) => (v) => (v.duration > 0 ? test(v.duration) : anyTag(...tags)(v));

/** Collection definitions, in the order they appear. */
export const AUTO_COLLECTIONS = [
  // Medium
  { key: 'hentai', name: 'Hentai', match: anyTag('hentai', '2d') },
  { key: 'sfm', name: 'SFM', match: anyTag('sfm') },
  { key: 'vam', name: 'VaM', match: anyTag('vam') },
  { key: 'mmd', name: 'MMD', match: anyTag('mmd') },
  { key: 'cgi', name: 'CGI / 3D', match: anyTag('cgi', '3d', 'blender', 'koikatsu', 'honey-select') },
  { key: 'ai', name: 'AI video', match: anyTag('ai-video', 'ai-assisted', 'ai', 'ai-generated') },
  // Session length
  { key: 'quick', name: 'Quick (under 5 min)', match: byLength((s) => s < 300, ['len-0-2', 'len-2-5']) },
  { key: 'sessions', name: 'Sessions (15 min+)', match: byLength((s) => s >= 900, ['len-25-60', 'len-60-plus']) },
  { key: 'marathons', name: 'Marathons (1 hr+)', match: byLength((s) => s >= 3600, ['len-60-plus']) },
  // Franchise
  { key: 'overwatch', name: 'Overwatch', match: anyTag('overwatch') },
  { key: 'genshin', name: 'Genshin Impact', match: anyTag('genshin-impact', 'genshin') },
  { key: 'zzz', name: 'Zenless Zone Zero', match: anyTag('zenless-zone-zero') },
  { key: 'final-fantasy', name: 'Final Fantasy', match: anyTag('final-fantasy', 'ff7', 'final-fantasy-7') },
  // Script style
  { key: 'music', name: 'Music / beat', match: anyTag('music', 'beat-based', 'music-based', 'music-video', 'hybrid-based', 'pmv', 'hmv', 'cock-hero', 'fap-hero') },
  { key: 'action', name: 'Action', match: anyTag('action-based', 'hybrid-based') },
];

const nameKey = (s) => String(s || '').trim().toLocaleLowerCase();

/**
 * What to create and add.
 *
 * @param {object} o
 * @param {{id: string, name: string, videoPaths?: string[]}[]} o.collections
 * @param {{path: string, tags: Set<string>, duration?: number}[]} o.videos
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
  for (const def of AUTO_COLLECTIONS) {
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
  const videos = videoPaths.map((path) => ({ path, tags: new Set(scriptTags[path]?.tags || []), duration: Number(durations[path]) || 0 }));
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
