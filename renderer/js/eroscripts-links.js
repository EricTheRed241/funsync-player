// Merge the links found by `npm run eroscripts:sync` into the per-video
// script tags.
//
// The CLI cannot write config.json (the app owns it and may be running), so
// it writes `eroscripts-links.json` in userData. Each library scan reads that
// file and folds it into `library.scriptTags`, where the rest of the app —
// the EroScripts preview batch included — already looks for a video's post.

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
 *   post: `source: 'eroscripts'`) always wins; the sync never replaces it.
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
    if (existing && existing.source !== SYNC_SOURCE) continue;
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
