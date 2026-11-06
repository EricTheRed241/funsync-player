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
// when some folder between the video and the volume's root is present AND
// the OS says the video and its folder do not exist (ENOENT / ENOTDIR). An
// unmounted drive has no such folder: `/Volumes/<name>` is absent, or (a
// stale mount point) present but empty, and neither ever qualifies. A check
// that timed out or failed any other way (a drive spinning up, a busy
// threadpool, EACCES, EIO) is `'unknown'` and keeps the entry.
//
// Pure apart from the injected `pathState` (electron/path-state.js through
// `window.funsync.pathState`), so the rule is testable without a filesystem
// and runnable from Node against a real one.

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
 * True only when a folder between `videoPath` and its volume root is
 * `'present'`, and the video and its folder are both `'absent'`. Any doubt
 * (unparseable path, `'unknown'`, a `pathState` that throws, nothing above
 * the folder but the root) answers false.
 *
 * The ancestor is asked first: it is the cheapest proof the drive answers,
 * and asking it wakes a spun-down drive before the checks that matter.
 *
 * @param {string} videoPath
 * @param {(p: string) => ('present'|'absent'|'unknown') | Promise<'present'|'absent'|'unknown'>} pathState
 * @returns {Promise<boolean>}
 */
export async function isVideoLocationGone(videoPath, pathState) {
  const split = splitVolumePath(videoPath);
  if (!split) return false;
  const { lead, sep, parts, rootDepth } = split;
  // Need: root, ≥1 folder that could prove the drive answers, the video's
  // own folder, the video.
  if (parts.length < rootDepth + 3) return false;
  const at = (n) => lead + parts.slice(0, n).join(sep);
  const state = async (p) => {
    try { return await pathState(p); } catch { return 'unknown'; }
  };

  let driveAnswers = false;
  for (let n = parts.length - 2; n > rootDepth && !driveAnswers; n--) {
    driveAnswers = (await state(at(n))) === 'present';
  }
  if (!driveAnswers) return false;
  if ((await state(videoPath)) !== 'absent') return false;
  return (await state(at(parts.length - 1))) === 'absent';
}
