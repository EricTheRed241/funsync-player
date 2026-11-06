// Is a stored association's VIDEO provably gone for good?
//
// `library.associations` is keyed by absolute video path. Deleting a video
// (by hand, or by reorganising the library into new folders) left its entry
// behind forever: the validation pass only looks at script paths, and a
// script whose folder is gone looks exactly like a script on an unplugged
// drive, so it was kept "untouched" — correctly, as far as it could tell.
//
// The rule that keeps this safe: a missing file is only evidence of deletion
// when the DRIVE it lives on demonstrably answers. On 2026-08-11 an absent
// drive was mistaken for deleted files and 325 associations were destroyed
// (see association-prune-safety.test.js). So an entry counts as dead only
// when the video and its folder are absent AND some folder between them and
// the volume's root still exists. An unmounted drive has no such folder:
// `/Volumes/<name>` is absent, or (a stale mount point) present but empty,
// and neither ever qualifies.
//
// Pure apart from the injected `exists`, so the rule is testable without a
// filesystem and runnable from Node against a real one.

/**
 * Split a path into its leading marker, segments, and how many segments make
 * up the volume root, which is never itself enough to prove reachability.
 *
 *   C:\a\b          → drive root `C:`                 (1)
 *   \\server\share  → network share                    (2)
 *   /Volumes/<name> → macOS mount                      (2)
 *   /mnt/<name>     → Linux mount                      (2)
 *   /media/<u>/<n>  → Linux removable media            (3)
 *   /run/media/<u>/<n>                                 (4)
 *   /net/<host>/<share> → autofs                       (3)
 *   anything else under `/` → its first directory      (1), so a stale empty
 *                     mount point directly under `/` cannot count either.
 *
 * @param {string} p
 * @returns {{ lead: string, sep: string, parts: string[], rootDepth: number } | null}
 */
export function splitVolumePath(p) {
  const raw = String(p || '');
  const sep = raw.includes('\\') ? '\\' : '/';
  let lead;
  let rest;
  if (/^[\\/]{2}[^\\/]/.test(raw)) {
    lead = sep + sep;
    rest = raw.slice(2);
  } else if (/^[A-Za-z]:[\\/]/.test(raw)) {
    lead = '';
    rest = raw;
  } else if (raw.startsWith('/')) {
    lead = '/';
    rest = raw.slice(1);
  } else {
    return null;
  }
  const parts = rest.split(/[\\/]+/).filter(Boolean);
  let rootDepth;
  if (lead === '') rootDepth = 1;
  else if (lead !== '/') rootDepth = 2;
  else if (parts[0] === 'Volumes' || parts[0] === 'mnt') rootDepth = 2;
  else if (parts[0] === 'media' || parts[0] === 'net') rootDepth = 3;
  else if (parts[0] === 'run' && parts[1] === 'media') rootDepth = 4;
  else rootDepth = 1;
  return { lead, sep, parts, rootDepth };
}

/**
 * True only when `videoPath` and its folder are absent and a folder between
 * them and the volume root exists. Any doubt (unparseable path, an `exists`
 * that throws, nothing left above the folder but the root) answers false.
 *
 * @param {string} videoPath
 * @param {(p: string) => boolean | Promise<boolean>} exists
 * @returns {Promise<boolean>}
 */
export async function isVideoLocationGone(videoPath, exists) {
  const split = splitVolumePath(videoPath);
  if (!split) return false;
  const { lead, sep, parts, rootDepth } = split;
  // Need: root, ≥1 folder that could prove the drive answers, the video's
  // own folder, the video.
  if (parts.length < rootDepth + 3) return false;
  const at = (n) => lead + parts.slice(0, n).join(sep);
  const check = async (p) => {
    try { return !!(await exists(p)); } catch { return null; }
  };

  if ((await check(videoPath)) !== false) return false;
  if ((await check(at(parts.length - 1))) !== false) return false;
  for (let n = parts.length - 2; n > rootDepth; n--) {
    if ((await check(at(n))) === true) return true;
  }
  return false;
}
