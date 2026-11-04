// Merge the video tags file (`<userData>/video-tags.json`, written by the
// `video-tags` CLI in crates/video-tags) into the per-video script tags.
//
// `library.scriptTags` is rebuilt from the scene sidecars and the EroScripts
// links on every scan (eroscripts-links.js), so a tag written straight into
// it would be lost. The file is the durable store instead; this merge runs
// right after those two and adds its tags on top of the post's:
//
// - a record keeps the tags it got from the file in `addedTags`; its own
//   (post) tags are `tags` minus `addedTags` (`baseTags`), and the sidecar
//   and link merges compare only those, so the added tags don't make them
//   rewrite the record on every scan;
// - a video without a record gets one of its own (`source: 'video-tags'`),
//   which any post record (sidecar, link) replaces; the added tags are then
//   put back on top of the post's;
// - only tags in VIDEO_TAGGER_TAGS (content-preferences.js) are applied.

import { SCRIPT_TAGS_KEY, baseTags } from './script-tags.js';
import { VIDEO_TAGGER_TAGS } from './content-preferences.js';

/** `source` of scriptTags records made only for the video tags file's tags. */
export const VIDEO_TAGS_SOURCE = 'video-tags';

const sameList = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

/** The file entry's tags: lower case, deduplicated, only allowed ones. */
function wantedTags(entry, allowed) {
  if (!Array.isArray(entry?.tags)) return [];
  const out = [];
  for (const tag of entry.tags) {
    if (typeof tag !== 'string') continue;
    const t = tag.trim().toLowerCase();
    if (t && allowed.has(t) && !out.includes(t)) out.push(t);
  }
  return out;
}

/**
 * Fold the video tags file into a scriptTags map.
 *
 * - A record's tags become its own tags plus the file's tags it lacks; the
 *   tags it gained are kept in `addedTags` (omitted when none).
 * - A video with no record and tags in the file gets a `video-tags` record.
 * - A video gone from the file loses its added tags; a `video-tags` record
 *   left without tags is deleted.
 * - Unchanged records are not rewritten.
 *
 * @param {Object<string, object>} current scriptTags map
 * @param {{videos?: Object<string, {tags?: string[]}>}|null|undefined} file
 *   the file as read; null (missing or unreadable) changes nothing
 * @param {string[]} allowed tags that may be applied (VIDEO_TAGGER_TAGS)
 * @param {number} now epoch ms
 * @returns {{map: Object<string, object>, added: number, updated: number,
 *   removed: number}|null} null when nothing changes
 */
export function mergeVideoTags(current, file, allowed, now) {
  if (!file || typeof file !== 'object') return null;
  const videos = file.videos && typeof file.videos === 'object' && !Array.isArray(file.videos) ? file.videos : {};
  const entries = new Map(Object.entries(videos));
  const allowedSet = new Set((allowed || []).map((t) => String(t).toLowerCase()));
  const map = { ...(current || {}) };
  let added = 0;
  let updated = 0;
  let removed = 0;

  const paths = new Set(entries.keys());
  for (const [videoPath, record] of Object.entries(map)) {
    if (record?.source === VIDEO_TAGS_SOURCE || record?.addedTags?.length) paths.add(videoPath);
  }

  for (const videoPath of paths) {
    if (!videoPath) continue;
    const wanted = wantedTags(entries.get(videoPath), allowedSet);
    const existing = map[videoPath];
    if (!existing) {
      if (!wanted.length) continue;
      map[videoPath] = {
        tags: wanted, addedTags: [...wanted], topicId: null, topicUrl: null, source: VIDEO_TAGS_SOURCE, savedAt: now,
      };
      added++;
      continue;
    }
    const base = baseTags(existing);
    const own = new Set(base);
    const add = wanted.filter((t) => !own.has(t));
    if (existing.source === VIDEO_TAGS_SOURCE && !base.length && !add.length) {
      delete map[videoPath];
      removed++;
      continue;
    }
    const tags = [...base, ...add];
    const before = Array.isArray(existing.addedTags) ? existing.addedTags : [];
    if (sameList(Array.isArray(existing.tags) ? existing.tags : [], tags) && sameList(before, add)
      && (add.length || !('addedTags' in existing))) continue;
    const next = { ...existing, tags };
    if (add.length) next.addedTags = add;
    else delete next.addedTags;
    map[videoPath] = next;
    updated++;
  }

  return added || updated || removed ? { map, added, updated, removed } : null;
}

/**
 * Read the video tags file through the main process and merge it into
 * settings.
 *
 * @param {{get: Function, set: Function}} settings
 * @param {{readVideoTags?: Function}} api `window.funsync`
 * @param {number} [now]
 * @returns {Promise<{added: number, updated: number, removed: number}|null>}
 */
export async function applyVideoTags(settings, api, now = Date.now()) {
  if (!settings?.get || !settings?.set || !api?.readVideoTags) return null;
  const file = await api.readVideoTags();
  if (!file) return null;
  const merged = mergeVideoTags(settings.get(SCRIPT_TAGS_KEY) || {}, file, VIDEO_TAGGER_TAGS, now);
  if (!merged) return null;
  settings.set(SCRIPT_TAGS_KEY, merged.map);
  const { added, updated, removed } = merged;
  return { added, updated, removed };
}
