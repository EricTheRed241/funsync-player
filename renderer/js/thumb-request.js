// Video-frame thumbnails for every desktop surface (library grid, playlists,
// categories, Up Next), so they all follow the same rules:
//
// - The backend's ffmpeg frame first (`generate-single-thumbnail` IPC). It
//   picks the frame, rejects black / blank / title-card frames, crops VR to
//   one eye and caches the result shared with the headset.
// - A backend 404 means the file is gone or empty. It is remembered per path
//   until the next library scan (`forgetMissingThumbs`): grid rebuilds asked
//   again and again (main.log 2026-11-06: the same 9 moved files 404'd about
//   ten times in four minutes), and each failure also ran the <video>
//   fallback, which fails the same way. After a 404 there is no fallback.
// - The <video> fallback (backend down or timed out) seeks where the backend
//   does for an auto frame, past the studio ident: max(10 s, 10%) for videos
//   over a minute, 10% otherwise. It used min(10%, 5 s), right inside the
//   ident window, so fallback tiles were often black.

import { pathToFileURL } from './path-utils.js';

/** Paths the backend answered 404 for since the last scan. */
const _missing = new Set();

/** Forget remembered 404s (a new scan may have brought files back). */
export function forgetMissingThumbs() {
  _missing.clear();
}

/** Whether the backend answered 404 for `videoPath` since the last scan. */
export function isThumbMissing(videoPath) {
  return _missing.has(videoPath);
}

/**
 * Ask the backend for a frame.
 *
 * @param {{generateSingleThumbnail?: Function}} api `window.funsync`
 * @param {string} videoPath
 * @param {{seekPct: number, exact: boolean, width?: number}} opts
 * @returns {Promise<{dataUrl: string, duration: number}|{missing: true}|null>}
 *   null when the backend couldn't answer (down, timeout, ffmpeg error)
 */
export async function requestBackendThumb(api, videoPath, { seekPct, exact, width = 320 }) {
  if (_missing.has(videoPath)) return { missing: true };
  if (!api?.generateSingleThumbnail) return null;
  try {
    const result = await api.generateSingleThumbnail(videoPath, { seekPct, width, exact });
    if (result?.missing) {
      _missing.add(videoPath);
      return { missing: true };
    }
    if (result?.dataUrl) return { dataUrl: result.dataUrl, duration: result.duration || 0 };
  } catch { /* backend unreachable: the caller's fallback decides */ }
  return null;
}

/**
 * Where the <video> fallback seeks: the pinned position when the user picked
 * a frame, else the backend's auto-frame rule (services/ffmpeg.py).
 *
 * @param {number} duration seconds
 * @param {number|null} [pinnedSeekPct] 0..1
 * @returns {number} seconds
 */
export function fallbackSeekSeconds(duration, pinnedSeekPct = null) {
  if (!Number.isFinite(duration) || duration <= 0) return 0;
  if (typeof pinnedSeekPct === 'number' && Number.isFinite(pinnedSeekPct)) return duration * pinnedSeekPct;
  return duration > 60 ? Math.max(10, duration * 0.1) : duration * 0.1;
}

/**
 * Renderer-side decode with a hidden <video>. Slow (Chromium loads the
 * file) but works without the backend.
 *
 * @param {string} videoPath
 * @param {{pinnedSeekPct?: number|null, timeoutMs?: number}} [opts]
 * @returns {Promise<{dataUrl: string, duration: number}|null>}
 */
export function captureFrameViaVideoElement(videoPath, { pinnedSeekPct = null, timeoutMs = 10000 } = {}) {
  return new Promise((resolve) => {
    const video = document.createElement('video');
    video.preload = 'metadata';
    video.muted = true;
    video.playsInline = true;
    video.style.position = 'fixed';
    video.style.left = '-9999px';
    video.style.width = '1px';
    video.style.height = '1px';
    document.body.appendChild(video);

    const cleanup = () => {
      video.removeAttribute('src');
      video.load();
      video.remove();
    };

    const timeout = setTimeout(() => { cleanup(); resolve(null); }, timeoutMs);

    video.addEventListener('loadedmetadata', () => {
      video.currentTime = fallbackSeekSeconds(video.duration, pinnedSeekPct);
    }, { once: true });

    video.addEventListener('seeked', () => {
      clearTimeout(timeout);
      try {
        const canvas = document.createElement('canvas');
        canvas.width = 320;
        canvas.height = Math.round(320 * (video.videoHeight / video.videoWidth)) || 180;
        canvas.getContext('2d').drawImage(video, 0, 0, canvas.width, canvas.height);
        const dataUrl = canvas.toDataURL('image/jpeg', 0.7);
        const duration = isFinite(video.duration) ? video.duration : 0;
        cleanup();
        resolve({ dataUrl, duration });
      } catch {
        cleanup();
        resolve(null);
      }
    }, { once: true });

    video.addEventListener('error', () => { clearTimeout(timeout); cleanup(); resolve(null); }, { once: true });
    video.src = pathToFileURL(videoPath);
  });
}

/**
 * A video frame for a tile: the backend's, else the <video> fallback, except
 * after a backend 404.
 *
 * @param {object} api `window.funsync`
 * @param {string} videoPath
 * @param {{seekPct: number, exact: boolean}} opts from `thumbRequestOpts`
 * @param {{timeoutMs?: number}} [fallback]
 * @returns {Promise<{dataUrl: string, duration: number}|null>}
 */
export async function captureVideoThumb(api, videoPath, { seekPct, exact }, { timeoutMs = 10000 } = {}) {
  const backend = await requestBackendThumb(api, videoPath, { seekPct, exact });
  if (backend?.dataUrl) return backend;
  if (backend?.missing) return null;
  return captureFrameViaVideoElement(videoPath, { pinnedSeekPct: exact ? seekPct : null, timeoutMs });
}
