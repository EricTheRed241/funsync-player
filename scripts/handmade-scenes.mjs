// Scenes for `upgrade:axes` and `prune:generated`: one video and every
// stroke script that belongs to it (its own script plus variants such as
// `Title_hardcore.funscript`), all in one folder. A stroke script that
// belongs to no video is a scene of its own.
//
// Hand-made axis scripts always win, per folder: when any script in a
// folder has hand-made axes (renderer/js/handmade-axes.js), none of the
// folder's scripts gets or keeps generated companions. Sibling videos (other
// versions of the scene: `_4k`, `_hard`, VR / flat) get a copy of the
// hand-made axes only when their stroke script's timing matches
// (`timingMatch`).

import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import { parseAxisSuffix } from '../renderer/js/multi-axis.js';
import { handmadeAxisSuffixes } from '../renderer/js/handmade-axes.js';
import { sceneVideoStem } from './upgrade-pairing.mjs';

export const VIDEO_EXTS = new Set(['.mp4', '.m4v', '.mkv', '.webm', '.avi', '.mov']);

const FUNSCRIPT = '.funscript';

/** True for a stroke script name: `.funscript` without an axis suffix, not AppleDouble. */
export function isStrokeScriptName(name) {
  return !name.startsWith('._') && name.toLowerCase().endsWith(FUNSCRIPT) && !parseAxisSuffix(name);
}

/**
 * Group one folder's listing into scenes.
 *
 * @param {string[]} names — filenames in the folder
 * @returns {Array<{video: string|null, scripts: string[]}>} filenames; `video`
 *   is null for a script that belongs to no video. Videos without a stroke
 *   script are left out.
 */
export function groupScenes(names) {
  const files = names.filter((n) => !n.startsWith('._'));
  const videos = files.filter((n) => VIDEO_EXTS.has(path.extname(n).toLowerCase()));
  const videoStems = videos.map((n) => n.slice(0, -path.extname(n).length));
  const byVideo = new Map();
  const scenes = [];
  for (const name of files.filter(isStrokeScriptName)) {
    const stem = sceneVideoStem(name.slice(0, -FUNSCRIPT.length), videoStems);
    if (stem === null) {
      scenes.push({ video: null, scripts: [name] });
      continue;
    }
    let scene = byVideo.get(stem);
    if (!scene) {
      scene = { video: videos[videoStems.indexOf(stem)], scripts: [] };
      byVideo.set(stem, scene);
      scenes.push(scene);
    }
    scene.scripts.push(name);
  }
  return scenes;
}

/** The parsed stroke script, or null when it can't be read or parsed. */
export async function readStrokeScript(file) {
  try {
    return JSON.parse((await fs.readFile(file, 'utf8')).replace(/^\uFEFF/, ''));
  } catch {
    return null;
  }
}

/**
 * Hand-made axes of every script of a scene.
 *
 * @param {string[]} scripts — the scene's script filenames
 * @param {string[]} names — the folder's listing
 * @param {(name: string) => object|null|Promise<object|null>} parse — parsed script by filename
 * @returns {Promise<{handmade: string[], byScript: Map<string, string[]>}>}
 *   `handmade` is the sorted union; `byScript` only lists scripts that have any
 */
export async function sceneHandmadeAxes(scripts, names, parse) {
  const all = new Set();
  const byScript = new Map();
  for (const name of scripts) {
    const found = handmadeAxisSuffixes({ primaryPath: name, allFiles: names, parsed: await parse(name) });
    if (found.length === 0) continue;
    byScript.set(name, found);
    for (const s of found) all.add(s);
  }
  return { handmade: [...all].sort(), byScript };
}

/**
 * The stroke scripts of a folder listing, every scene's.
 * @param {string[]} names
 */
export function folderStrokeScripts(names) {
  return groupScenes(names).flatMap((s) => s.scripts);
}

// ── Timing ──────────────────────────────────────────────────────────────

/** A sibling's script must end within this much of the source's. */
export const SIBLING_END_TOLERANCE_MS = 1000;
/** An action counts as matched by one at most this far away. */
export const SIBLING_ACTION_WINDOW_MS = 50;
/** Share of the source's actions that need a match. */
export const SIBLING_MATCH_SHARE = 0.95;

/** `[at, pos]` pairs, rounded and sorted; metadata and key order don't count. */
export function normalisedActions(script) {
  const acts = Array.isArray(script?.actions) ? script.actions : [];
  return acts
    .filter((a) => Number.isFinite(Number(a?.at)) && Number.isFinite(Number(a?.pos)))
    .map((a) => [Math.round(Number(a.at)), Math.round(Number(a.pos))])
    .sort((a, b) => a[0] - b[0] || a[1] - b[1]);
}

/**
 * Hash of the normalised actions (rounded, sorted). Metadata, key order and
 * whitespace don't count. null for a script with fewer than 2 actions:
 * nothing to tell apart.
 */
export function actionsHash(script) {
  const acts = normalisedActions(script);
  if (acts.length < 2) return null;
  return crypto.createHash('sha1').update(JSON.stringify(acts)).digest('hex');
}

/**
 * Does a sibling's stroke script provably share the source script's timing,
 * so the source's axis scripts fit the sibling video too? No frames needed.
 *
 * @param {object|null} source — parsed stroke script that has the hand-made axes
 * @param {object|null} sibling — parsed stroke script of the sibling video
 * @returns {'identical'|'near'|null} `identical`: same normalised actions;
 *   `near`: both end within 1 s of each other and ≥ 95 % of the source's
 *   action times have one in the sibling within ±50 ms; null otherwise
 */
export function timingMatch(source, sibling) {
  const a = actionsHash(source);
  if (!a) return null;
  if (a === actionsHash(sibling)) return 'identical';
  const src = normalisedActions(source).map((x) => x[0]);
  const sib = normalisedActions(sibling).map((x) => x[0]);
  if (sib.length < 2) return null;
  if (Math.abs(src[src.length - 1] - sib[sib.length - 1]) > SIBLING_END_TOLERANCE_MS) return null;
  let matched = 0;
  let j = 0;
  for (const at of src) {
    while (j < sib.length && sib[j] < at - SIBLING_ACTION_WINDOW_MS) j++;
    if (j < sib.length && sib[j] <= at + SIBLING_ACTION_WINDOW_MS) matched++;
  }
  return matched >= SIBLING_MATCH_SHARE * src.length ? 'near' : null;
}
