// Thumbnail config of videos that are gone.
//
// `library.customThumbnails` (posters, pinned frames), `library.faptapThumbnails`
// (FapTap import attempts) and `library.durationCache` are keyed by video
// path, and nothing removed an entry when its video was deleted or moved:
// on 2026-11-09 a quarter of the posters and FapTap attempts, and more than
// half of the cached durations, pointed into a folder that no longer existed.
//
// After a scan that read every source, entries whose video the scan didn't
// find are checked with the association rule (association-dead.js): gone only
// when the video and its folder are absent while a folder above them answers,
// so an unplugged drive or unreachable share never loses anything. Gone
// entries are dropped and their custom-thumbs files deleted (main process).

import { isVideoLocationGone } from './association-dead.js';
import { CUSTOM_THUMBS_KEY, FAPTAP_ATTEMPTS_KEY } from './eroscripts-preview.js';

const DURATION_CACHE_KEY = 'library.durationCache';
const KEYS = [CUSTOM_THUMBS_KEY, FAPTAP_ATTEMPTS_KEY, DURATION_CACHE_KEY];

/**
 * @param {{get: Function, set: Function}} settings
 * @param {object} opts
 * @param {Iterable<string>} opts.scannedPaths every video the complete scan found
 * @param {(p: string) => Promise<'present'|'absent'|'unknown'>} opts.pathState
 * @param {(p: string) => boolean} [opts.skip] paths never to touch (unavailable sources)
 * @param {(videoPaths: string[], imagePaths: string[], keep: string[]) => Promise<number>} [opts.deleteFiles]
 * @param {number} [opts.concurrency]
 * @returns {Promise<{gone: number, customThumbnails: number, faptapThumbnails: number,
 *   durationCache: number, files: number}>}
 */
export async function pruneThumbConfig(settings, { scannedPaths, pathState, skip = () => false, deleteFiles, concurrency = 8 }) {
  const live = new Set(scannedPaths);
  const candidates = new Set();
  for (const key of KEYS) {
    for (const p of Object.keys(settings.get(key) || {})) {
      if (!live.has(p) && !skip(p)) candidates.add(p);
    }
  }

  const gone = new Set();
  const list = [...candidates];
  for (let i = 0; i < list.length; i += concurrency) {
    await Promise.all(list.slice(i, i + concurrency).map(async (p) => {
      if (await isVideoLocationGone(p, pathState)) gone.add(p);
    }));
  }

  const summary = { gone: gone.size, customThumbnails: 0, faptapThumbnails: 0, durationCache: 0, files: 0 };
  if (gone.size === 0) return summary;

  // Fresh reads: an import may have written these maps during the checks.
  const images = [];
  let keep = [];
  for (const key of KEYS) {
    const map = { ...(settings.get(key) || {}) };
    let removed = 0;
    for (const p of gone) {
      if (!(p in map)) continue;
      if (key === CUSTOM_THUMBS_KEY && typeof map[p]?.imagePath === 'string') images.push(map[p].imagePath);
      delete map[p];
      removed++;
    }
    if (key === CUSTOM_THUMBS_KEY) {
      keep = Object.values(map).map((e) => e?.imagePath).filter((x) => typeof x === 'string');
    }
    if (removed) settings.set(key, map);
    summary[key.slice('library.'.length)] = removed;
  }
  if (deleteFiles) summary.files = (await deleteFiles([...gone], images, keep)) || 0;
  return summary;
}
