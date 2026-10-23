// Scenes for `upgrade:axes` and `prune:generated`: one video and every
// stroke script that belongs to it (its own script plus variants such as
// `Title_hardcore.funscript`), all in one folder. A stroke script that
// belongs to no video is a scene of its own.
//
// Hand-made axis scripts always win, per scene: when any script of the
// scene has hand-made axes (renderer/js/handmade-axes.js), none of the
// scene's scripts gets or keeps generated companions.

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
