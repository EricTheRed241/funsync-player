'use strict';

// `Title_hardcore.funscript` next to `Title.mp4` is an alternate script for
// that video, not a separate title. Normalised matching cannot see this:
// `_` becomes a space, so the name reads as "title hardcore" and no video
// has that name.
//
// `videoStems` are the raw basenames (no extension) of the videos in the
// funscript's own folder. Returns null when the funscript is itself named
// after one of them, so `Title_4k.funscript` stays the primary of
// `Title_4k.mp4`. Picks the longest stem, so `Title_eve_extra` prefers
// `Title_eve.mp4` over `Title.mp4`. Case-insensitive, like the rest of the
// scan.
function underscoreVariant(nameNoExt, videoStems) {
  if (!nameNoExt || !videoStems || videoStems.length === 0) return null;
  const lower = nameNoExt.toLowerCase();
  let best = null;
  for (const stem of videoStems) {
    const s = stem.toLowerCase();
    if (s === lower) return null;
    if (lower.length > s.length + 1 && lower.startsWith(s + '_')
        && (best === null || s.length > best.length)) {
      best = stem;
    }
  }
  if (best === null) return null;
  return { stem: best, label: nameNoExt.slice(best.length + 1) };
}

module.exports = { underscoreVariant };
