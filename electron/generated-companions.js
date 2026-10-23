'use strict';

// Generated axis companions in one stroke script's folder, for the
// `trash-generated-companions` IPC. Hand-made axis scripts always win, per
// folder: once a hand-made axis file for a video arrives (an EroScripts
// download), every `<stem>.generated.<axis>.funscript` in that folder goes
// to the OS trash — the video's own and those of sibling videos (other
// versions of the scene: `_4k`, `_hard`, VR / flat). Nothing is copied to
// the siblings here; `npm run prune:generated` does that when the timing
// matches.
//
// Only files in the script's own folder (not subfolders) whose name is
// exactly `<stem>.generated.<axis>.funscript` (any stem, any case, any
// accepted axis spelling, see renderer/js/multi-axis.js AXIS_DEFINITIONS)
// are ever touched. The caller names a stroke script, never the files to
// trash.

const fs = require('fs');
const path = require('path');

const AXIS_NAMES = new Set([
  'surge', 'forward', 'l1',
  'sway', 'left', 'l2',
  'twist', 'yaw', 'r0',
  'roll', 'r1',
  'pitch', 'r2',
  'vib', 'vibrate', 'v0',
  'pump', 'v1',
  'valve', 'a0',
  'suck', 'suction', 'a1',
  'lube', 'a2',
]);

const FUNSCRIPT = '.funscript';
const GENERATED = '.generated';

/**
 * The stem of `<stem>.generated.<axis>.funscript`; null for any other name.
 * @param {string} name — a filename
 */
function generatedStemOf(name) {
  if (!name || name.startsWith('._') || !name.toLowerCase().endsWith(FUNSCRIPT)) return null;
  const base = name.slice(0, -FUNSCRIPT.length);
  const dot = base.lastIndexOf('.');
  if (dot < 0 || !AXIS_NAMES.has(base.slice(dot + 1).toLowerCase())) return null;
  const rest = base.slice(0, dot);
  if (!rest.toLowerCase().endsWith(GENERATED)) return null;
  return rest.slice(0, -GENERATED.length) || null;
}

/**
 * Paths of every generated companion in `scriptPath`'s folder.
 * @param {string} scriptPath — absolute path of a `.funscript` stroke script
 * @returns {Promise<string[]>} empty for anything that isn't one (an axis
 *   companion, generated or not, is not a stroke script)
 */
async function listGeneratedInFolder(scriptPath) {
  if (typeof scriptPath !== 'string' || !path.isAbsolute(scriptPath)) return [];
  const base = path.basename(scriptPath);
  if (!base.toLowerCase().endsWith(FUNSCRIPT)) return [];
  const stem = base.slice(0, -FUNSCRIPT.length);
  const dot = stem.lastIndexOf('.');
  if (!stem || (dot >= 0 && AXIS_NAMES.has(stem.slice(dot + 1).toLowerCase()))) return [];
  const dir = path.dirname(scriptPath);
  let entries;
  try {
    entries = await fs.promises.readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  return entries
    .filter((e) => e.isFile() && generatedStemOf(e.name) !== null)
    .map((e) => path.join(dir, e.name));
}

/**
 * Move every generated companion in `scriptPath`'s folder to the OS trash.
 * @param {string} scriptPath
 * @param {{trashItem: (p: string) => Promise<void>}} deps — Electron `shell`
 * @returns {Promise<{trashed: string[], failed: string[]}>}
 */
async function trashGeneratedCompanions(scriptPath, { trashItem }) {
  const trashed = [];
  const failed = [];
  for (const file of await listGeneratedInFolder(scriptPath)) {
    try {
      await trashItem(file);
      trashed.push(file);
    } catch {
      failed.push(file);
    }
  }
  return { trashed, failed };
}

module.exports = { generatedStemOf, listGeneratedInFolder, trashGeneratedCompanions };
