'use strict';

// Generated axis companions of one stroke script, for the
// `trash-generated-companions` IPC. Hand-made axis scripts always win: once
// a hand-made axis file for a video arrives (an EroScripts download), the
// video's `<stem>.generated.<axis>.funscript` files go to the OS trash.
//
// Only files in the script's own folder whose name is exactly
// `<stem>.generated.<axis>.funscript` (any case, any accepted axis spelling,
// see renderer/js/multi-axis.js AXIS_DEFINITIONS) are ever touched. The
// caller names the script, never the files to trash.

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
const GENERATED = '.generated.';

/**
 * True when `name` is `<stem>.generated.<axis>.funscript` for `stem`.
 * @param {string} name — a filename
 * @param {string} stem — the stroke script's name without `.funscript`
 */
function isGeneratedCompanionOf(name, stem) {
  if (!name || !stem || name.startsWith('._')) return false;
  const lower = name.toLowerCase();
  const prefix = stem.toLowerCase() + GENERATED;
  if (!lower.startsWith(prefix) || !lower.endsWith(FUNSCRIPT)) return false;
  return AXIS_NAMES.has(lower.slice(prefix.length, -FUNSCRIPT.length));
}

/**
 * Paths of the generated companions next to `scriptPath`.
 * @param {string} scriptPath — absolute path of a `.funscript` stroke script
 * @returns {Promise<string[]>} empty for anything that isn't one
 */
async function listGeneratedCompanions(scriptPath) {
  if (typeof scriptPath !== 'string' || !path.isAbsolute(scriptPath)) return [];
  const base = path.basename(scriptPath);
  if (!base.toLowerCase().endsWith(FUNSCRIPT)) return [];
  const stem = base.slice(0, -FUNSCRIPT.length);
  const dir = path.dirname(scriptPath);
  let entries;
  try {
    entries = await fs.promises.readdir(dir, { withFileTypes: true });
  } catch {
    return [];
  }
  return entries
    .filter((e) => e.isFile() && isGeneratedCompanionOf(e.name, stem))
    .map((e) => path.join(dir, e.name));
}

/**
 * Move the generated companions next to `scriptPath` to the OS trash.
 * @param {string} scriptPath
 * @param {{trashItem: (p: string) => Promise<void>}} deps — Electron `shell`
 * @returns {Promise<{trashed: string[], failed: string[]}>}
 */
async function trashGeneratedCompanions(scriptPath, { trashItem }) {
  const trashed = [];
  const failed = [];
  for (const file of await listGeneratedCompanions(scriptPath)) {
    try {
      await trashItem(file);
      trashed.push(file);
    } catch {
      failed.push(file);
    }
  }
  return { trashed, failed };
}

module.exports = { isGeneratedCompanionOf, listGeneratedCompanions, trashGeneratedCompanions };
