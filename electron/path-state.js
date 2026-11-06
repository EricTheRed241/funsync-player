// Time-boxed existence checks for the main process.

const fs = require('fs');

/**
 * Default budget for an existence check, in ms.
 *
 * A DEAD MAPPED NETWORK DRIVE is the case this exists for. Windows keeps the
 * drive letter after a NAS goes offline, so the path looks real and the OS
 * blocks on an SMB round trip that can take seconds. lnlytrckr (EroScripts
 * #251/#255) saw the whole app freeze for ~5s every 20-30s because of it.
 *
 * External drives do NOT hit this: Windows removes the letter entirely, so the
 * check fails immediately.
 */
const EXISTS_TIMEOUT_MS = 800;

// The only answers that prove a path is not there. Anything else (EACCES,
// EIO, ETIMEDOUT, a slow drive spinning up) says nothing about the file.
const ABSENT_CODES = new Set(['ENOENT', 'ENOTDIR']);

/**
 * Async, time-boxed existence check with three answers:
 * `'present'`, `'absent'` (the OS said ENOENT / ENOTDIR) or `'unknown'`
 * (timed out, any other error, or not a path).
 *
 * Two deliberate changes from the old `fs.existsSync`:
 *
 *  1. ASYNC. `existsSync` runs on the main process's JS thread, so a blocking
 *     network stat froze the entire UI, not just the library. `fs.promises`
 *     work happens on libuv's threadpool and leaves the app responsive.
 *
 *  2. TIME-BOXED. We stop WAITING after the budget and answer `'unknown'`.
 *     Note this does not cancel the underlying syscall — it cannot be
 *     cancelled — so the threadpool slot stays occupied until the OS gives up.
 *     That is acceptable because callers now skip disabled sources and cache
 *     results, so we issue few of these. If that ever changes, watch the
 *     4-slot default threadpool.
 *
 * @param {string} filePath
 * @param {number} [timeoutMs]
 * @param {(p: string) => Promise<void>} [access] injectable for tests
 * @returns {Promise<'present'|'absent'|'unknown'>}
 */
async function pathState(filePath, timeoutMs = EXISTS_TIMEOUT_MS, access = (p) => fs.promises.access(p)) {
  if (typeof filePath !== 'string' || filePath.length === 0) return 'unknown';
  let timer;
  try {
    return await Promise.race([
      access(filePath).then(
        () => 'present',
        (err) => (ABSENT_CODES.has(err?.code) ? 'absent' : 'unknown'),
      ),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve('unknown'), timeoutMs);
      }),
    ]);
  } catch {
    return 'unknown';
  } finally {
    clearTimeout(timer);
  }
}

/**
 * `true` only when the path answered within the budget. An unreadable path
 * is `false`, which is how existing callers treat it.
 *
 * @param {string} filePath
 * @param {number} [timeoutMs]
 */
async function pathReachable(filePath, timeoutMs = EXISTS_TIMEOUT_MS) {
  return (await pathState(filePath, timeoutMs)) === 'present';
}

module.exports = { EXISTS_TIMEOUT_MS, pathState, pathReachable };
