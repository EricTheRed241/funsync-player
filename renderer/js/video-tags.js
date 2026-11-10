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
// - only tags in VIDEO_TAGGER_TAGS (content-preferences.js) are applied,
//   plus the pornstar tags below.
//
// The pornstar tags file (`<userData>/pornstar-tags.json`, written by the
// `pornstar-tags` CLI in crates/pornstar-tags) is merged in the same pass:
// its tags join each video's tags from video-tags.json (`combineTagFiles`),
// and its performers with `applied: true` join the allowed tags.

import { SCRIPT_TAGS_KEY, baseTags } from './script-tags.js';
import { VIDEO_TAGGER_TAGS } from './content-preferences.js';

/** `source` of scriptTags records made only for the video tags file's tags. */
export const VIDEO_TAGS_SOURCE = 'video-tags';

const sameList = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * One file for `mergeVideoTags` out of the video tags file and the pornstar
 * tags file, with the tags it may apply.
 *
 * - Per video, the union of both files' tags.
 * - Allowed: `videoTaggerTags` plus the pornstar file's applied tags.
 * - A missing pornstar file applies no pornstar tags (those added before
 *   are withdrawn); the video tags are unaffected.
 * - A missing video tags file changes no video tag: the VIDEO_TAGGER_TAGS a
 *   record already got (`addedTags`) are kept.
 *
 * @param {Object<string, object>} current scriptTags map
 * @param {{videos?: Object<string, {tags?: string[]}>}|null} videoFile
 * @param {{pornstars?: Object<string, {applied?: boolean}>, videos?: Object<string, string[]>}|null} pornstarFile
 * @param {string[]} videoTaggerTags
 * @returns {{file: {videos: Object<string, {tags: string[]}>}, allowed: string[]}|null}
 *   null when both files are missing (nothing changes)
 */
export function combineTagFiles(current, videoFile, pornstarFile, videoTaggerTags) {
  if (!videoFile && !pornstarFile) return null;
  const isMap = (v) => !!v && typeof v === 'object' && !Array.isArray(v);
  const videos = {};
  const add = (videoPath, tags) => {
    if (!Array.isArray(tags) || !tags.length) return;
    videos[videoPath] = { tags: [...(videos[videoPath]?.tags || []), ...tags] };
  };
  if (videoFile) {
    for (const [videoPath, entry] of Object.entries(isMap(videoFile.videos) ? videoFile.videos : {})) add(videoPath, entry?.tags);
  } else {
    const tagger = new Set(videoTaggerTags.map((t) => String(t).toLowerCase()));
    for (const [videoPath, record] of Object.entries(current || {})) {
      if (Array.isArray(record?.addedTags)) add(videoPath, record.addedTags.filter((t) => tagger.has(t)));
    }
  }
  const pornstars = isMap(pornstarFile?.pornstars) ? pornstarFile.pornstars : {};
  const applied = Object.keys(pornstars).filter((tag) => pornstars[tag]?.applied === true);
  for (const [videoPath, tags] of Object.entries(isMap(pornstarFile?.videos) ? pornstarFile.videos : {})) add(videoPath, tags);
  return { file: { videos }, allowed: [...videoTaggerTags, ...applied] };
}

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
 * Read the video tags and pornstar tags files through the main process and
 * merge them into settings.
 *
 * @param {{get: Function, set: Function}} settings
 * @param {{readVideoTags?: Function, readPornstarTags?: Function}} api `window.funsync`
 * @param {number} [now]
 * @returns {Promise<{added: number, updated: number, removed: number}|null>}
 */
export async function applyVideoTags(settings, api, now = Date.now()) {
  if (!settings?.get || !settings?.set || !api?.readVideoTags) return null;
  const [videoFile, pornstarFile] = await Promise.all([
    api.readVideoTags(),
    api.readPornstarTags ? api.readPornstarTags() : null,
  ]);
  const current = settings.get(SCRIPT_TAGS_KEY) || {};
  const combined = combineTagFiles(current, videoFile, pornstarFile, VIDEO_TAGGER_TAGS);
  if (!combined) return null;
  const merged = mergeVideoTags(current, combined.file, combined.allowed, now);
  if (!merged) return null;
  settings.set(SCRIPT_TAGS_KEY, merged.map);
  const { added, updated, removed } = merged;
  return { added, updated, removed };
}
