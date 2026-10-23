// Hand-made axis scripts always win, also for EroScripts downloads: once a
// download saves a hand-made axis file for a script (a companion such as
// `Title.twist.funscript`, or a main script with axes embedded in it), that
// script's `Title.generated.<axis>.funscript` files go to the OS trash.
// Used by the EroScripts panel and the Associate dialog's EroScripts search.

import { getBaseName, parseAxisSuffix, isGeneratedCompanion } from './multi-axis.js';
import { extractEmbeddedAxes } from './embedded-multi-axis.js';
import { stripBOM } from './funscript-engine.js';

const dirOf = (p) => p.replace(/[\\/][^\\/]*$/, '');
const sepOf = (p) => (p.includes('\\') && !p.includes('/') ? '\\' : '/');

/**
 * Stroke scripts whose generated companions the saved files replace: the
 * main script each hand-made companion belongs to, and every saved main
 * script with embedded axes. Paths of the form `<dir>/<stem>.funscript`.
 *
 * @param {string[]} savedPaths — files a download just wrote
 * @param {(path: string) => Promise<string|null>} readScript — file text
 * @returns {Promise<string[]>}
 */
export async function scriptsReplacedBy(savedPaths, readScript) {
  const out = new Set();
  for (const p of savedPaths || []) {
    if (!p || !p.toLowerCase().endsWith('.funscript')) continue;
    const name = p.split(/[\\/]/).pop();
    if (parseAxisSuffix(name)) {
      if (!isGeneratedCompanion(name)) out.add(`${dirOf(p)}${sepOf(p)}${getBaseName(name)}.funscript`);
      continue;
    }
    let parsed = null;
    try { parsed = JSON.parse(stripBOM((await readScript(p)) || '')); } catch { /* unreadable: no embedded axes */ }
    if (extractEmbeddedAxes(parsed).size > 0) out.add(p);
  }
  return [...out];
}

/**
 * Trash the generated companions replaced by freshly saved files.
 *
 * @param {string[]} savedPaths
 * @param {object} deps
 * @param {(path: string) => Promise<string|null>} deps.readScript
 * @param {(scriptPath: string) => Promise<{trashed: string[], failed: string[]}>} deps.trash
 * @returns {Promise<string[]>} the trashed paths
 */
export async function trashReplacedGenerated(savedPaths, { readScript, trash }) {
  const trashed = [];
  for (const script of await scriptsReplacedBy(savedPaths, readScript)) {
    try {
      const result = await trash(script);
      trashed.push(...(result?.trashed || []));
    } catch (err) {
      console.warn('[HandmadeWins] trashing generated companions failed for', script, err?.message);
    }
  }
  return trashed;
}
