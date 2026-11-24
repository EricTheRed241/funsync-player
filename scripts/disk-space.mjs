// Disk space for the scouts' downloads: the library drive keeps
// MIN_FREE_GB (content-preferences.json `swipeDeck.minFreeGb`) free after
// every download, counting the downloads still in flight in this process.
// A video or archive that wouldn't leave that much is skipped and left for a
// later run; scripts (tiny) aren't checked.

import fs from 'node:fs';
import path from 'node:path';

/**
 * Bytes free for this user on the drive holding `dir` (statfs: bavail × bsize). A folder not made yet
 * is measured at its nearest existing parent.
 */
export function freeBytes(dir) {
  let at = path.resolve(dir);
  while (!fs.existsSync(at) && path.dirname(at) !== at) at = path.dirname(at);
  const s = fs.statfsSync(at);
  return s.bavail * s.bsize;
}

const gb = (bytes) => `${Number((bytes / 1e9).toFixed(1))} GB`;

/**
 * Whether a download of `bytes` (0 / null: size unknown) fits on the drive of `dir` while `inFlight` bytes
 * are still coming and `minFree` bytes must stay free afterwards: free − inFlight − bytes ≥ minFree.
 * Null when it fits, else the reason (`not enough disk space (X GB free, Y GB needed to keep Z GB free)`).
 * `free` (bytes) is measured with freeBytes unless given.
 */
export function roomFor({ dir, bytes = 0, inFlight = 0, minFree, free = freeBytes(dir) }) {
  const needed = minFree + (inFlight || 0) + (bytes > 0 ? bytes : 0);
  if (free >= needed) return null;
  return `not enough disk space (${gb(free)} free, ${gb(needed)} needed to keep ${gb(minFree)} free)`;
}
