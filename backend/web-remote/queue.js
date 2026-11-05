// The shared play queue, seen from the phone.
//
// The desktop app owns the queue (manual items the user added, then items
// it auto-fills). The phone reads it from `GET /api/remote/queue` and
// changes it through the backend like every other library edit
// (`routes/remote.py` → the desktop): a write may answer
// `delivered: false`, meaning it applies when the desktop reconnects.
//
// "Next" on the phone = manual queue items → the phone's own Play All list
// → auto-filled items, never the video already playing.

import { postJson } from './triage.js';

export const AUTOPLAY_STORAGE_KEY = 'funsync.remote.autoplay';

/** `GET /api/remote/queue` body normalised: `{items: [{videoId, auto}], autoFill}`. */
export function normalizeQueue(data) {
  const items = Array.isArray(data?.items)
    ? data.items
      .filter((it) => it && typeof it.videoId === 'string' && it.videoId)
      .map((it) => ({ videoId: it.videoId, auto: !!it.auto }))
    : [];
  return { items, autoFill: data?.autoFill !== false };
}

/** The shared queue, or null when it can't be read (offline, old backend). */
export async function fetchQueue(fetchImpl = globalThis.fetch) {
  try {
    const resp = await fetchImpl('/api/remote/queue');
    if (!resp.ok) return null;
    return normalizeQueue(await resp.json());
  } catch {
    return null;
  }
}

/** Request bodies, kept pure so they're testable. */
export function queueAddBody(videoId, position = 'end') {
  return { videoId, position: position === 'next' ? 'next' : 'end' };
}

export function queueAdd(videoId, position, fetchImpl) {
  return postJson('/api/remote/queue/add', queueAddBody(videoId, position), fetchImpl);
}

export function queueRemove(videoId, fetchImpl) {
  return postJson('/api/remote/queue/remove', { videoId }, fetchImpl);
}

export function queuePlayed(videoId, fetchImpl) {
  return postJson('/api/remote/queue/played', { videoId }, fetchImpl);
}

export function queueClear(fetchImpl) {
  return postJson('/api/remote/queue/clear', {}, fetchImpl);
}

export function queueSetAutoFill(enabled, fetchImpl) {
  return postJson('/api/remote/queue/auto-fill', { enabled: !!enabled }, fetchImpl);
}

/** The queue a write answered with (`{…, queue}`), or null. */
export function queueFromResult(result) {
  return result?.ok && result.data?.queue ? normalizeQueue(result.data.queue) : null;
}

/** Ids of the manual items, in order. */
export function manualIds(shared) {
  return (shared?.items || []).filter((it) => !it.auto).map((it) => it.videoId);
}

/** Ids of the auto-filled items, in order. */
export function autoIds(shared) {
  return (shared?.items || []).filter((it) => it.auto).map((it) => it.videoId);
}

/**
 * What plays after `currentId`: `{id, source}` with source 'manual' (the
 * user's queue), 'playAll' (the phone's Play All list, `{ids, index}`) or
 * 'auto' (auto-filled), or null when nothing is up next. The video being
 * played is never its own next.
 */
export function nextUp({ currentId = null, shared = null, phoneQueue = null } = {}) {
  const manual = manualIds(shared).find((id) => id !== currentId);
  if (manual) return { id: manual, source: 'manual' };

  if (phoneQueue && Array.isArray(phoneQueue.ids)) {
    const start = Number.isInteger(phoneQueue.index) ? phoneQueue.index + 1 : 0;
    const next = phoneQueue.ids.slice(Math.max(0, start)).find((id) => id && id !== currentId);
    if (next) return { id: next, source: 'playAll' };
  }

  const auto = autoIds(shared).find((id) => id !== currentId);
  if (auto) return { id: auto, source: 'auto' };
  return null;
}

/** Autoplay (count down to the next video at the end), per phone; default on. */
export function getAutoplay(storage = globalThis.localStorage) {
  try {
    return storage?.getItem(AUTOPLAY_STORAGE_KEY) !== '0';
  } catch {
    return true;
  }
}

export function setAutoplay(on, storage = globalThis.localStorage) {
  try { storage?.setItem(AUTOPLAY_STORAGE_KEY, on ? '1' : '0'); } catch { /* private mode */ }
}
