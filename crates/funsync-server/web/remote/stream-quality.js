// Stream quality (user, 2026-11-25: buffering over Wi-Fi): the player can ask
// the server for a live, lower-bitrate transcode (`?quality=1080|720&t=<s>`)
// instead of the original file. A transcode can't be seeked inside, so a
// seek asks again from the new time; this module hides that from the rest of
// the page by giving the <video> its own `currentTime` / `duration`: the
// whole video's clock, as if it played the original.

export const QUALITIES = ['original', '1080', '720'];
const KEY = 'funsync.remote.streamQuality';

export function getQuality(storage = globalThis.localStorage) {
  try {
    const q = storage?.getItem(KEY);
    return QUALITIES.includes(q) ? q : 'original';
  } catch {
    return 'original';
  }
}

export function setQuality(q, storage = globalThis.localStorage) {
  if (!QUALITIES.includes(q)) return;
  try {
    if (q === 'original') storage?.removeItem(KEY);
    else storage?.setItem(KEY, q);
  } catch { /* private browsing: the choice just will not persist */ }
}

/** The URL the <video> plays for `quality`, starting at `t` seconds. */
export function streamSrc(base, quality, t = 0) {
  if (quality === 'original') return base;
  const sep = base.includes('?') ? '&' : '?';
  return `${base}${sep}quality=${quality}&t=${Math.max(0, t).toFixed(3)}`;
}

/**
 * Play `base` as a transcode on `video`, with the original's clock.
 * `currentTime` reads offset + the element's own time; setting it reloads
 * the transcode from there and fires `seeking` (currentTime already the
 * target, as natively) and `seeked` once the new stream has data.
 * `duration` is the library's.
 */
export function playTranscoded(video, { base, quality, duration, start = 0 }) {
  const proto = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'currentTime');
  let offset = 0;
  const load = (t) => {
    const wasPlaying = !video.paused;
    offset = Math.max(0, duration > 0 ? Math.min(t, duration) : t);
    video.src = streamSrc(base, quality, offset);
    if (wasPlaying) video.play()?.catch(() => {});
  };
  Object.defineProperty(video, 'currentTime', {
    configurable: true,
    get: () => offset + proto.get.call(video),
    set: (t) => {
      const target = Number(t);
      if (!Number.isFinite(target)) return;
      video.addEventListener('loadeddata', () => video.dispatchEvent(new Event('seeked')), { once: true });
      load(target);
      video.dispatchEvent(new Event('seeking'));
    },
  });
  Object.defineProperty(video, 'duration', {
    configurable: true,
    get: () => duration || offset + (video.seekable?.length ? video.seekable.end(0) : 0),
  });
  load(start);
}
