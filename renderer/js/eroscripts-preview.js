// An EroScripts post's preview image as a video's library thumbnail.
//
// Stored like a user-uploaded poster (`library.customThumbnails[path]`,
// `{ type: 'image', imagePath }`) so every thumbnail surface (library,
// web remote, HereSphere, DeoVR) already shows it, plus `source` and
// `topicId` so it can be told apart from an upload. The image itself is
// downloaded and stored locally by the main process; nothing hot-links the
// forum.
//
// Automatic imports never replace an existing custom thumbnail: a poster
// the user uploaded or a frame they pinned always wins, and Reset brings
// back the generated frame.

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
 */
export function makePreviewEntry(imagePath, topicId) {
  return { type: 'image', imagePath, source: PREVIEW_SOURCE, topicId };
}

/** Is this customThumbnails entry an EroScripts preview (vs. user-chosen)? */
export function isPreviewEntry(entry) {
  return entry?.type === 'image' && entry.source === PREVIEW_SOURCE;
}

/**
 * May an automatic import set this video's thumbnail? Only when it has no
 * custom thumbnail at all.
 */
export function canAutoImport(entry) {
  return !entry;
}

/**
 * The image URL to import for a topic: the search result's poster-size
 * `preview`, a `previewUrl` recorded with the video's post link, else the
 * topic's own preview (one forum request). The search listing's small
 * `thumbnail` is deliberately not used: a missing `preview` means it was
 * too small for a tile, and the topic often has a bigger image.
 *
 * @param {{eroscriptsTopicImage?: Function}} api `window.funsync`
 * @param {{id?: number, preview?: string|null, previewUrl?: string}} topic
 */
export async function resolvePreviewUrl(api, topic) {
  const known = topic?.preview || topic?.previewUrl;
  if (typeof known === 'string' && known) return known;
  if (topic?.id == null || !api?.eroscriptsTopicImage) return null;
  return (await api.eroscriptsTopicImage(topic.id)) || null;
}

/**
 * Import a post's preview as a video's thumbnail.
 *
 * @param {object} args
 * @param {{get: Function, set: Function}} args.settings
 * @param {object} args.api `window.funsync`
 * @param {string} args.videoPath
 * @param {object} args.topic search result or `{ id }`
 * @param {boolean} [args.force] the user asked for it: replace whatever is set
 * @returns {Promise<{status: 'imported'|'skipped'|'no-image'|'failed', dataUrl?: string}>}
 */
export async function importPostPreview({ settings, api, videoPath, topic, force = false }) {
  const current = () => (settings.get(CUSTOM_THUMBS_KEY) || {})[videoPath];
  if (!videoPath || topic?.id == null || !api?.eroscriptsImportPreview) return { status: 'failed' };
  if (!force && !canAutoImport(current())) return { status: 'skipped' };

  const url = await resolvePreviewUrl(api, topic);
  if (!url) return { status: 'no-image' };

  const imported = await api.eroscriptsImportPreview(videoPath, url);
  if (!imported?.path) return { status: 'failed' };

  // The download took a while: the user may have picked a thumbnail since.
  if (!force && !canAutoImport(current())) return { status: 'skipped' };

  const map = { ...(settings.get(CUSTOM_THUMBS_KEY) || {}) };
  map[videoPath] = makePreviewEntry(imported.path, topic.id);
  settings.set(CUSTOM_THUMBS_KEY, map);
  thumbCache.remove(videoPath, 0);
  if (imported.dataUrl) thumbCache.set(videoPath, 0, imported.dataUrl);
  eventBus.emit(PREVIEW_EVENT, { videoPath, dataUrl: imported.dataUrl || null });
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
 * Fetch previews for every video with a known post and no custom
 * thumbnail, one at a time. Videos sharing a post resolve its image once.
 *
 * @param {object} args
 * @param {{get: Function, set: Function}} args.settings
 * @param {object} args.api `window.funsync`
 * @param {Array<{path: string}>} args.videos
 * @param {(done: number, total: number) => void} [args.onProgress]
 * @param {(ms: number) => Promise<void>} [args.sleep]
 * @param {number} [args.delayMs]
 * @returns {Promise<{known: number, total: number, imported: number,
 *   skipped: number, noImage: number, failed: number}>}
 */
export async function fetchPreviewsBatch({
  settings, api, videos, onProgress,
  sleep = (ms) => new Promise((r) => setTimeout(r, ms)),
  delayMs = BATCH_DELAY_MS,
}) {
  const { known, pending } = planPreviewBatch(settings, videos);
  const summary = { known, total: pending.length, imported: 0, skipped: 0, noImage: 0, failed: 0 };
  const urls = new Map(); // topicId → resolved URL (null = post has none)

  for (let i = 0; i < pending.length; i++) {
    const { videoPath, topic } = pending[i];
    onProgress?.(i, pending.length);
    if (!urls.has(topic.id)) urls.set(topic.id, await resolvePreviewUrl(api, topic));
    const url = urls.get(topic.id);

    let status;
    if (!url) {
      status = 'no-image';
    } else {
      ({ status } = await importPostPreview({ settings, api, videoPath, topic: { id: topic.id, previewUrl: url } }));
    }
    if (status === 'imported') summary.imported++;
    else if (status === 'no-image') summary.noImage++;
    else if (status === 'skipped') summary.skipped++;
    else summary.failed++;

    if (i < pending.length - 1) await sleep(delayMs);
  }
  onProgress?.(pending.length, pending.length);
  return summary;
}
