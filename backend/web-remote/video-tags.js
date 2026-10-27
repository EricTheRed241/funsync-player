// Tags and VR for the web remote's library filters and the Tags view.
//
// `/api/remote/videos` gives every video `isVR` (the backend's decision,
// the same the headsets get), `tags` (EroScripts / FapTap post tags without
// the noise, lower case) and `categories` (desktop category names).

import { isVRVideo } from './vr-detect.js';

/** The backend's VR decision; the filename heuristic for a backend that
 *  doesn't send one yet. */
export function videoIsVR(video) {
  if (typeof video?.isVR === 'boolean') return video.isVR;
  return isVRVideo(video?.name);
}

const norm = (s) => String(s ?? '').trim().toLowerCase();

/** True when `video` carries `tag` (case-insensitive). */
export function hasTag(video, tag) {
  const key = norm(tag);
  return !!key && (video?.tags || []).some((t) => norm(t) === key);
}

/** `videos` carrying `tag`; all of them when `tag` is empty. */
export function filterByTag(videos, tag) {
  if (!norm(tag)) return videos;
  return videos.filter((v) => hasTag(v, tag));
}

/**
 * One entry per tag in `videos`, shaped like a grouping from
 * `/api/remote/<kind>` (`{id, name, videoCount, totalDuration, videoIds}`,
 * id = the tag) so the grouping list can render it. Most used first, then
 * by name.
 */
export function tagGroupings(videos) {
  const byTag = new Map();
  for (const v of videos || []) {
    for (const raw of new Set((v.tags || []).map(norm))) {
      if (!raw) continue;
      let g = byTag.get(raw);
      if (!g) {
        g = { id: raw, name: raw, videoCount: 0, totalDuration: 0, videoIds: [] };
        byTag.set(raw, g);
      }
      g.videoCount += 1;
      g.totalDuration += Number(v.duration) || 0;
      g.videoIds.push(v.id);
    }
  }
  return [...byTag.values()].sort((a, b) =>
    b.videoCount - a.videoCount || a.name.localeCompare(b.name));
}
