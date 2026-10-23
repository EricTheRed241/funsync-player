// An EroScripts post's preview image as a video's library thumbnail.
//
// Stored like a user-uploaded poster (`library.customThumbnails[path]`,
// `{ type: 'image', imagePath }`) so every thumbnail surface (library,
// web remote, HereSphere, DeoVR) already shows it, plus `source` and
// `topicId` so it can be told apart from an upload. The image itself is
// downloaded and stored locally by the main process; nothing hot-links the
// forum.
//
// A post image becomes a poster only when it shows the video: the main
// process compares every candidate image of the post with frames sampled
// across the video (electron/preview-match.js) and imports the closest one
// within the threshold. `matchScore` is that distance (bits of 64, lower is
// closer) and `imageUrl` the image's address. No match leaves the generated
// frame in place.
//
// Automatic imports never replace an existing custom thumbnail: a poster
// the user uploaded or a frame they pinned always wins, and Reset brings
// back the generated frame. Revalidation removes automatic EroScripts
// posters that don't match their video; one the user picked by hand
// (`forced`) is theirs and is never touched.

import * as thumbCache from './thumbnail-cache.js';
import { eventBus } from './event-bus.js';
import { SCRIPT_TAGS_KEY } from './script-tags.js';

export const CUSTOM_THUMBS_KEY = 'library.customThumbnails';
export const PREVIEW_SOURCE = 'eroscripts';

/** Spacing between posts in a batch, so the forum sees ≤ 2 requests/s. */
export const BATCH_DELAY_MS = 600;

/** Event the library listens to, to repaint a tile and re-sync the backend. */
export const PREVIEW_EVENT = 'library:customThumbnailChanged';

/**
 * @param {string} imagePath file in the custom-thumbs dir
 * @param {number} topicId
 * @param {{imageUrl?: string|null, matchScore?: number|null, forced?: boolean}} [extra]
 */
export function makePreviewEntry(imagePath, topicId, { imageUrl = null, matchScore = null, forced = false } = {}) {
  const entry = { type: 'image', imagePath, source: PREVIEW_SOURCE, topicId };
  if (imageUrl) entry.imageUrl = imageUrl;
  if (typeof matchScore === 'number') entry.matchScore = matchScore;
  if (forced) entry.forced = true;
  return entry;
}

/** Is this customThumbnails entry an EroScripts preview (vs. user-chosen)? */
export function isPreviewEntry(entry) {
  return entry?.type === 'image' && entry.source === PREVIEW_SOURCE;
}

/**
 * May revalidation remove this entry? Only an EroScripts poster FunSync
 * picked by itself. Uploads, pinned frames and previews the user chose by
 * hand are left alone.
 */
export function isRevalidatable(entry) {
  return isPreviewEntry(entry) && !entry.forced;
}

/**
 * May an automatic import set this video's thumbnail? Only when it has no
 * custom thumbnail at all.
 */
export function canAutoImport(entry) {
  return !entry;
}

/** Image URLs known for a topic besides its post: a search result's preview, a recorded link's. */
function knownUrls(topic) {
  return [topic?.preview, topic?.previewUrl].filter((u) => typeof u === 'string' && u);
}

function writeEntry(settings, videoPath, entry, dataUrl) {
  const map = { ...(settings.get(CUSTOM_THUMBS_KEY) || {}) };
  map[videoPath] = entry;
  settings.set(CUSTOM_THUMBS_KEY, map);
  thumbCache.remove(videoPath, 0);
  if (dataUrl) thumbCache.set(videoPath, 0, dataUrl);
  eventBus.emit(PREVIEW_EVENT, { videoPath, dataUrl: dataUrl || null });
}

/**
 * Import the post image that matches the video as its thumbnail, if any.
 *
 * @param {object} args
 * @param {{get: Function, set: Function}} args.settings
 * @param {object} args.api `window.funsync`
 * @param {string} args.videoPath
 * @param {object} args.topic search result or `{ id, previewUrl? }`
 * @returns {Promise<{status: 'imported'|'skipped'|'no-match'|'no-image'|'failed',
 *   dataUrl?: string, matchScore?: number, closest?: number|null}>}
 */
export async function importPostPreview({ settings, api, videoPath, topic }) {
  const current = () => (settings.get(CUSTOM_THUMBS_KEY) || {})[videoPath];
  if (!videoPath || topic?.id == null || !api?.eroscriptsMatchPreview) return { status: 'failed' };
  if (!canAutoImport(current())) return { status: 'skipped' };

  const res = await api.eroscriptsMatchPreview(videoPath, topic.id, knownUrls(topic));
  if (res?.status === 'no-match') return { status: 'no-match', closest: res.closest ?? null };
  if (res?.status === 'no-image') return { status: 'no-image' };
  if (res?.status !== 'imported' || !res.path) return { status: 'failed' };

  // Matching took a while: the user may have picked a thumbnail since.
  if (!canAutoImport(current())) return { status: 'skipped' };

  writeEntry(settings, videoPath, makePreviewEntry(res.path, topic.id, { imageUrl: res.url, matchScore: res.matchScore }), res.dataUrl);
  return { status: 'imported', dataUrl: res.dataUrl, matchScore: res.matchScore };
}

/**
 * The user picked one candidate in "Use EroScripts preview": it becomes the
 * poster whatever its score, replacing whatever is set, and revalidation
 * will leave it alone.
 *
 * @param {{url: string, distance?: number|null}} candidate
 * @returns {Promise<{status: 'imported'|'failed', dataUrl?: string}>}
 */
export async function usePreviewCandidate({ settings, api, videoPath, topicId, candidate }) {
  if (!videoPath || !candidate?.url || !api?.eroscriptsImportPreview) return { status: 'failed' };
  const imported = await api.eroscriptsImportPreview(videoPath, candidate.url);
  if (!imported?.path) return { status: 'failed' };
  writeEntry(settings, videoPath, makePreviewEntry(imported.path, topicId, {
    imageUrl: candidate.url, matchScore: candidate.distance ?? null, forced: true,
  }), imported.dataUrl);
  return { status: 'imported', dataUrl: imported.dataUrl };
}

/**
 * Videos a batch fetch would cover: a known post and no custom thumbnail.
 *
 * @param {{get: Function}} settings
 * @param {Array<{path: string}>} videos
 * @returns {{known: number, pending: Array<{videoPath: string, topic: object}>}}
 */
export function planPreviewBatch(settings, videos) {
  const links = settings.get(SCRIPT_TAGS_KEY) || {};
  const custom = settings.get(CUSTOM_THUMBS_KEY) || {};
  let known = 0;
  const pending = [];
  for (const video of videos || []) {
    const record = links[video?.path];
    if (record?.topicId == null) continue;
    known++;
    if (!canAutoImport(custom[video.path])) continue;
    const topic = { id: record.topicId };
    if (typeof record.previewUrl === 'string' && record.previewUrl) topic.previewUrl = record.previewUrl;
    pending.push({ videoPath: video.path, topic });
  }
  return { known, pending };
}

/**
 * Check every automatic EroScripts poster of `videos` against its video.
 * One that doesn't match is removed (the generated frame comes back) and
 * its image file deleted; one that matches gets its `matchScore`. A poster
 * that can't be checked (video offline) is kept.
 *
 * @returns {Promise<{checked: number, kept: number, removed: number, unchecked: number}>}
 */
export async function revalidatePreviews({ settings, api, videos, onProgress }) {
  const custom = settings.get(CUSTOM_THUMBS_KEY) || {};
  const todo = (videos || []).filter((v) => isRevalidatable(custom[v?.path]));
  const summary = { checked: todo.length, kept: 0, removed: 0, unchecked: 0 };
  for (let i = 0; i < todo.length; i++) {
    onProgress?.(i, todo.length);
    const videoPath = todo[i].path;
    const entry = custom[videoPath];
    const res = await api.eroscriptsValidatePoster(videoPath, entry.imagePath);
    // The user may have changed the thumbnail while this ran.
    const now = (settings.get(CUSTOM_THUMBS_KEY) || {})[videoPath];
    if (now !== entry || res?.error || typeof res?.accepted !== 'boolean') {
      summary.unchecked++;
      continue;
    }
    const map = { ...(settings.get(CUSTOM_THUMBS_KEY) || {}) };
    if (res.accepted) {
      summary.kept++;
      if (entry.matchScore !== res.distance) {
        map[videoPath] = { ...entry, matchScore: res.distance };
        settings.set(CUSTOM_THUMBS_KEY, map);
      }
      continue;
    }
    summary.removed++;
    delete map[videoPath];
    settings.set(CUSTOM_THUMBS_KEY, map);
    thumbCache.remove(videoPath, 0);
    await api.deleteEroScriptsThumbnail?.(entry.imagePath);
    eventBus.emit(PREVIEW_EVENT, { videoPath, dataUrl: null });
  }
  onProgress?.(todo.length, todo.length);
  return summary;
}

/**
 * Fetch previews for every video with a known post and no custom
 * thumbnail, one at a time, after (by default) revalidating the EroScripts
 * posters already set, so those that failed get a fresh chance.
 *
 * @param {object} args
 * @param {{get: Function, set: Function}} args.settings
 * @param {object} args.api `window.funsync`
 * @param {Array<{path: string}>} args.videos
 * @param {boolean} [args.revalidate]
 * @param {(phase: 'revalidate'|'fetch', done: number, total: number) => void} [args.onProgress]
 * @param {(ms: number) => Promise<void>} [args.sleep]
 * @param {number} [args.delayMs]
 * @returns {Promise<{known: number, total: number, imported: number, skipped: number,
 *   noMatch: number, noImage: number, failed: number,
 *   revalidated: {checked: number, kept: number, removed: number, unchecked: number}|null}>}
 */
export async function fetchPreviewsBatch({
  settings, api, videos, onProgress, revalidate = true,
  sleep = (ms) => new Promise((r) => setTimeout(r, ms)),
  delayMs = BATCH_DELAY_MS,
}) {
  const revalidated = revalidate
    ? await revalidatePreviews({ settings, api, videos, onProgress: (d, t) => onProgress?.('revalidate', d, t) })
    : null;

  const { known, pending } = planPreviewBatch(settings, videos);
  const summary = { known, total: pending.length, imported: 0, skipped: 0, noMatch: 0, noImage: 0, failed: 0, revalidated };
  const topicsSeen = new Set();

  for (let i = 0; i < pending.length; i++) {
    const { videoPath, topic } = pending[i];
    onProgress?.('fetch', i, pending.length);
    // The main process caches topics, so only a new post costs a forum request.
    if (topicsSeen.size > 0 && !topicsSeen.has(topic.id)) await sleep(delayMs);
    topicsSeen.add(topic.id);

    const { status } = await importPostPreview({ settings, api, videoPath, topic });
    if (status === 'imported') summary.imported++;
    else if (status === 'no-match') summary.noMatch++;
    else if (status === 'no-image') summary.noImage++;
    else if (status === 'skipped') summary.skipped++;
    else summary.failed++;
  }
  onProgress?.('fetch', pending.length, pending.length);
  return summary;
}
