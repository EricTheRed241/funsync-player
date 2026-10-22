// Saving EroScripts downloads without clobbering existing scripts.
//
// Every download surface plans its target paths here first. If any target
// already exists the user picks: replace, keep both, or cancel. "Keep both"
// saves the new files as `<video> (EroScripts).funscript` (and
// `<video> (EroScripts).roll.funscript` …), which the library already pairs
// with `<video>.mp4` as a variant, so the download is usable next to the
// script it would have replaced. The main process refuses to overwrite
// unless the plan says so, so a file that appears after planning is not
// clobbered either.

import { parseAxisSuffix } from './multi-axis.js';

const KEEP_LABEL = 'EroScripts';
const MAX_KEEP_ATTEMPTS = 50;

/**
 * `/v/Video.roll.funscript`, 1 → `/v/Video (EroScripts).roll.funscript`
 * `/v/Video.funscript`, 2      → `/v/Video (EroScripts 2).funscript`
 * The label goes before any axis suffix so companions keep pairing with
 * their main script.
 */
export function keepBothPath(path, attempt = 1) {
  const label = attempt > 1 ? `${KEEP_LABEL} ${attempt}` : KEEP_LABEL;
  const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  const dir = path.slice(0, slash + 1);
  const file = path.slice(slash + 1);
  const ext = file.match(/\.[^.]+$/)?.[0] || '';
  const axis = parseAxisSuffix(file) ? file.slice(0, -ext.length).match(/\.[^.]+$/)[0] : '';
  const stem = file.slice(0, file.length - ext.length - axis.length);
  return `${dir}${stem} (${label})${axis}${ext}`;
}

/**
 * Decide where a set of downloads goes.
 *
 * @param {string[]} paths — intended targets (a main script and its axes, or one file)
 * @param {(fileName: string) => Promise<'replace'|'keep'|null>} ask
 *   called at most once, only when a target already exists
 * @returns {Promise<{paths: string[], overwrite: boolean}|null>} null = cancelled
 */
export async function planTargets(paths, ask) {
  const exists = await window.funsync.filesExist(paths);
  const clash = paths.find((_, i) => exists?.[i]);
  if (!clash) return { paths, overwrite: false };

  const choice = await ask(clash.split(/[\\/]/).pop());
  if (choice === 'replace') return { paths, overwrite: true };
  if (choice !== 'keep') return null;

  for (let n = 1; n <= MAX_KEEP_ATTEMPTS; n++) {
    const alt = paths.map((p) => keepBothPath(p, n));
    const altExists = await window.funsync.filesExist(alt);
    if (!alt.some((_, i) => altExists?.[i])) return { paths: alt, overwrite: false };
  }
  return null;
}
