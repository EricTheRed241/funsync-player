// Merge the links found by `npm run eroscripts:sync`, and the sources of
// Discovery scenes, into the per-video script tags.
//
// The CLIs cannot write config.json (the app owns it and may be running).
// The sync writes `eroscripts-links.json` in userData; the scouts leave an
// `eroscripts.json` or `faptap.json` sidecar in each scene folder, which the
// library scan attaches to the video. Each library scan folds both into
// `library.scriptTags`, where the rest of the app — the EroScripts preview
// batch and the auto categories included — already looks for a video's tags.

import { SCRIPT_TAGS_KEY } from './script-tags.js';

/** `source` of scriptTags records that came from the sync CLI. */
export const SYNC_SOURCE = 'eroscripts-sync';

const APPLIED_CONFIDENCE = new Set(['confirmed', 'probable']);

function recordFromLink(link, now) {
  const record = {
    tags: Array.isArray(link.tags) ? link.tags.filter((t) => typeof t === 'string' && t.trim()) : [],
    topicId: link.topicId,
    topicUrl: link.topicUrl || null,
    source: SYNC_SOURCE,
    savedAt: now,
    confidence: link.confidence,
    matchedAt: link.matchedAt || null,
  };
  if (typeof link.previewUrl === 'string' && link.previewUrl) record.previewUrl = link.previewUrl;
  return record;
}

const sameRecord = (a, b) => a.topicId === b.topicId
  && a.topicUrl === b.topicUrl
  && a.confidence === b.confidence
  && a.matchedAt === b.matchedAt
  && (a.previewUrl || null) === (b.previewUrl || null)
  && JSON.stringify(a.tags || []) === JSON.stringify(b.tags || []);

/**
 * Fold links into a scriptTags map.
 *
 * - A record the app wrote itself (the user downloaded the script from a
 *   post: `source: 'eroscripts'`) or the EroScripts scout's record of the
 *   post a scene came from always wins; the sync never replaces it.
 * - A FapTap scout record is replaced: the EroScripts post carries a topic
 *   id (previews, hand-made axes) the FapTap scene lacks.
 * - Only `confirmed` and `probable` links are applied; the review list stays
 *   in the file for the user.
 * - A sync record whose link has gone from the file is removed, so a link
 *   the CLI withdrew does not linger.
 *
 * @param {Object<string, object>} current scriptTags map
 * @param {Object<string, object>|null|undefined} links `links` of the file
 * @param {number} now epoch ms
 * @returns {{map: Object<string, object>, added: number, updated: number,
 *   removed: number}|null} null when nothing changes
 */
export function mergeLinksIntoScriptTags(current, links, now) {
  if (!links || typeof links !== 'object') return null;
  const map = { ...(current || {}) };
  let added = 0;
  let updated = 0;
  let removed = 0;

  for (const [videoPath, link] of Object.entries(links)) {
    if (!videoPath || link?.topicId == null || !APPLIED_CONFIDENCE.has(link.confidence)) continue;
    const existing = map[videoPath];
    if (existing && existing.source !== SYNC_SOURCE && existing.source !== FAPTAP_SOURCE) continue;
    const next = recordFromLink(link, now);
    if (existing && sameRecord(existing, next)) continue;
    map[videoPath] = next;
    if (existing) updated++;
    else added++;
  }

  for (const [videoPath, record] of Object.entries(map)) {
    if (record?.source !== SYNC_SOURCE) continue;
    const link = links[videoPath];
    if (link?.topicId != null && APPLIED_CONFIDENCE.has(link.confidence)) continue;
    delete map[videoPath];
    removed++;
  }

  return added || updated || removed ? { map, added, updated, removed } : null;
}

/**
 * Read the links file through the main process and merge it into settings.
 *
 * @param {{get: Function, set: Function}} settings
 * @param {{readEroscriptsLinks?: Function}} api `window.funsync`
 * @param {number} [now]
 * @returns {Promise<{added: number, updated: number, removed: number}|null>}
 */
export async function applyEroscriptsLinks(settings, api, now = Date.now()) {
  if (!settings?.get || !settings?.set || !api?.readEroscriptsLinks) return null;
  const file = await api.readEroscriptsLinks();
  if (!file) return null;
  const merged = mergeLinksIntoScriptTags(settings.get(SCRIPT_TAGS_KEY) || {}, file.links, now);
  if (!merged) return null;
  settings.set(SCRIPT_TAGS_KEY, merged.map);
  const { added, updated, removed } = merged;
  return { added, updated, removed };
}

/** `source` of scriptTags records taken from a Discovery scene's `eroscripts.json`. */
export const SCOUT_SOURCE = 'eroscripts-scout';

/** `source` of scriptTags records taken from a Discovery scene's `faptap.json`. */
export const FAPTAP_SOURCE = 'faptap-scout';

const sameTags = (a, b) => JSON.stringify(a || []) === JSON.stringify(b || []);

/**
 * Fold the sources the library scan found in Discovery sidecars (see
 * electron/library-scan.js) into a scriptTags map.
 *
 * - `video.eroscripts`, the EroScripts post the scene was downloaded from:
 *   its record replaces a sync guess or a FapTap record; a record the app
 *   wrote itself (`source: 'eroscripts'`) still wins.
 * - `video.faptap`, the FapTap scene (no EroScripts post, `topicId: null`):
 *   its record only fills a gap or refreshes an older FapTap record; any
 *   EroScripts record (app download, sync link, scout) wins.
 *
 * Unchanged records are not rewritten.
 *
 * @param {Object<string, object>} current scriptTags map
 * @param {{path: string,
 *   eroscripts?: {topicId: number, url: string|null, tags: string[]},
 *   faptap?: {faptapId: string, url: string|null, tags: string[]}}[]} videos
 * @param {number} now epoch ms
 * @returns {{map: Object<string, object>, added: number, updated: number}|null} null when nothing changes
 */
export function mergeScoutPosts(current, videos, now) {
  const map = { ...(current || {}) };
  let added = 0;
  let updated = 0;
  const put = (videoPath, existing, record) => {
    map[videoPath] = record;
    if (existing) updated++;
    else added++;
  };
  for (const v of videos || []) {
    if (!v?.path) continue;
    const existing = map[v.path];
    const post = v.eroscripts;
    const scene = v.faptap;
    if (post?.topicId != null) {
      if (existing?.source === 'eroscripts') continue;
      const tags = Array.isArray(post.tags) ? post.tags : [];
      if (existing?.source === SCOUT_SOURCE && existing.topicId === post.topicId && sameTags(existing.tags, tags)) continue;
      put(v.path, existing, { tags, topicId: post.topicId, topicUrl: post.url || null, source: SCOUT_SOURCE, savedAt: now });
    } else if (scene?.faptapId != null && scene.faptapId !== '') {
      if (existing && existing.source !== FAPTAP_SOURCE) continue;
      const tags = Array.isArray(scene.tags) ? scene.tags : [];
      const faptapUrl = scene.url || null;
      if (existing && existing.faptapId === scene.faptapId && (existing.faptapUrl || null) === faptapUrl
        && sameTags(existing.tags, tags)) continue;
      put(v.path, existing, {
        tags, topicId: null, topicUrl: null, faptapId: scene.faptapId, faptapUrl, source: FAPTAP_SOURCE, savedAt: now,
      });
    }
  }
  return added || updated ? { map, added, updated } : null;
}
