// prune:generated — hand-made axis scripts always win.
//
// For every scene (a video and its stroke scripts, see handmade-scenes.mjs)
// where any script has hand-made axes (a companion that isn't `.generated.`,
// or axes embedded in the script; see renderer/js/handmade-axes.js):
//   - embedded axes are written out as `<stem>.<axis>.funscript` companions
//     of their script (canonical suffix; an existing file or a hand-made
//     companion for the axis is never replaced; the script is not touched);
//   - every `<stem>.generated.<axis>.funscript` of the scene's scripts (and
//     of the video's own name) is moved into the archive at
//     `<archive>/<path relative to the library folder's parent>`, and
//     `original<TAB>archived` is appended to `<archive>/MANIFEST.txt`.
// macOS `._` AppleDouble files are ignored. Nothing is ever deleted.

import fs from 'node:fs/promises';
import path from 'node:path';
import { extractEmbeddedAxes, buildCompanionFiles } from '../renderer/js/embedded-multi-axis.js';
import {
  handmadeAxisSuffixes,
  generatedCompanionFiles,
  canonicalAxisSuffix,
} from '../renderer/js/handmade-axes.js';
import { groupScenes, readStrokeScript, sceneHandmadeAxes } from './handmade-scenes.mjs';

const MANIFEST = 'MANIFEST.txt';
const MANIFEST_HEADER = 'Moved by funsync-player prune:generated (hand-made axes win). '
  + 'Restore: move each file back to its original path.\n';

/** Every folder under `dir` (itself included) with its file listing, depth first. */
async function walk(dir, out) {
  let entries;
  try {
    entries = await fs.readdir(dir, { withFileTypes: true });
  } catch {
    return;
  }
  out.push({ dir, names: entries.filter((e) => e.isFile() && !e.name.startsWith('._')).map((e) => e.name) });
  for (const ent of entries) {
    if (ent.isDirectory() && !ent.name.startsWith('.')) await walk(path.join(dir, ent.name), out);
  }
}

/** Companion files to write for the axes embedded in one script. */
function plannedExtractions(dir, name, parsed, names) {
  const covered = new Set(handmadeAxisSuffixes({ primaryPath: name, allFiles: names }));
  const taken = new Set(names.map((n) => n.toLowerCase()));
  const stem = name.slice(0, -'.funscript'.length);
  const embedded = new Map();
  for (const [suffix, actions] of extractEmbeddedAxes(parsed)) {
    const canonical = canonicalAxisSuffix(suffix);
    if (!embedded.has(canonical)) embedded.set(canonical, actions);
  }
  const extract = [];
  for (const file of buildCompanionFiles(embedded)) {
    const target = `${stem}.${file.suffix}.funscript`;
    if (covered.has(file.suffix) || taken.has(target.toLowerCase())) continue;
    extract.push({ path: path.join(dir, target), suffix: file.suffix, content: file.content });
  }
  return { embedded: [...embedded.keys()].sort(), extract };
}

/**
 * Where a file goes in the archive: same path relative to `relativeTo`.
 * @param {string} file
 * @param {string} archiveDir
 * @param {string} relativeTo — the library folder's parent
 */
export function archiveTarget(file, archiveDir, relativeTo) {
  return path.join(archiveDir, path.relative(relativeTo, file));
}

/**
 * Plan the prune for library folders. Reads only.
 *
 * @param {string[]} roots — library folders
 * @param {string} archiveDir
 * @returns {Promise<Array<{
 *   video: string|null,
 *   scripts: string[],
 *   relativeTo: string,
 *   handmade: string[],
 *   embedded: string[],
 *   extract: Array<{path: string, suffix: string, content: object}>,
 *   archive: Array<{from: string, to: string}>,
 * }>>} one entry per scene with hand-made axes, in walk order; paths absolute
 */
export async function planPrune(roots, archiveDir) {
  const scenes = [];
  for (const rootArg of roots) {
    const root = path.resolve(rootArg);
    const relativeTo = path.dirname(root);
    const folders = [];
    await walk(root, folders);
    for (const { dir, names } of folders) {
      for (const scene of groupScenes(names)) {
        const parsedByName = new Map();
        const parse = async (name) => {
          if (!parsedByName.has(name)) parsedByName.set(name, await readStrokeScript(path.join(dir, name)));
          return parsedByName.get(name);
        };
        const { handmade } = await sceneHandmadeAxes(scene.scripts, names, parse);
        if (handmade.length === 0) continue;

        const embedded = new Set();
        const extract = [];
        for (const name of scene.scripts) {
          const planned = plannedExtractions(dir, name, await parse(name), names);
          for (const s of planned.embedded) embedded.add(s);
          extract.push(...planned.extract);
        }
        const anchors = scene.video ? [...scene.scripts, scene.video] : scene.scripts;
        const generated = new Set(anchors.flatMap((anchor) => generatedCompanionFiles(anchor, names)));
        const archive = [...generated].map((n) => {
          const from = path.join(dir, n);
          return { from, to: archiveTarget(from, archiveDir, relativeTo) };
        });
        scenes.push({
          video: scene.video ? path.join(dir, scene.video) : null,
          scripts: scene.scripts.map((n) => path.join(dir, n)),
          relativeTo,
          handmade,
          embedded: [...embedded].sort(),
          extract,
          archive,
        });
      }
    }
  }
  return scenes;
}

async function moveFile(from, to) {
  await fs.mkdir(path.dirname(to), { recursive: true });
  try {
    await fs.access(to);
    throw new Error(`archive target exists: ${to}`);
  } catch (err) {
    if (err.code !== 'ENOENT') throw err;
  }
  try {
    await fs.rename(from, to);
  } catch (err) {
    if (err.code !== 'EXDEV') throw err;
    await fs.copyFile(from, to, fs.constants.COPYFILE_EXCL);
    await fs.unlink(from);
  }
}

/**
 * Move files into the archive, keeping their path relative to `relativeTo`,
 * and append `original<TAB>archived` per moved file to
 * `<archiveDir>/MANIFEST.txt`. An existing archive file is never replaced:
 * that file stays where it is and gets an `error`.
 *
 * @param {string[]} files
 * @param {{archiveDir: string, relativeTo: string, dryRun?: boolean}} opts
 * @returns {Promise<Array<{from: string, to: string, error?: string}>>}
 */
export async function archiveFiles(files, { archiveDir, relativeTo, dryRun = false }) {
  const results = files.map((from) => ({ from, to: archiveTarget(from, archiveDir, relativeTo) }));
  if (dryRun) return results;
  const lines = [];
  for (const r of results) {
    try {
      await moveFile(r.from, r.to);
      lines.push(`${r.from}\t${r.to}\n`);
    } catch (err) {
      r.error = err.message;
    }
  }
  if (lines.length > 0) {
    await fs.mkdir(archiveDir, { recursive: true });
    const manifest = path.join(archiveDir, MANIFEST);
    let exists = true;
    try { await fs.access(manifest); } catch { exists = false; }
    await fs.appendFile(manifest, (exists ? '' : MANIFEST_HEADER) + lines.join(''), 'utf8');
  }
  return results;
}

/**
 * Carry out a plan from `planPrune`: write the extracted companions (never
 * replacing a file), then archive the generated ones.
 *
 * @returns {Promise<{extracted: string[], archived: Array<{from: string, to: string}>, errors: string[]}>}
 */
export async function applyPrune(scenes, archiveDir) {
  const extracted = [];
  const archived = [];
  const errors = [];
  for (const scene of scenes) {
    for (const file of scene.extract) {
      try {
        await fs.writeFile(file.path, JSON.stringify(file.content) + '\n', { encoding: 'utf8', flag: 'wx' });
        extracted.push(file.path);
      } catch (err) {
        errors.push(`extract ${file.path}: ${err.message}`);
      }
    }
  }
  for (const scene of scenes) {
    if (scene.archive.length === 0) continue;
    const results = await archiveFiles(scene.archive.map((a) => a.from), { archiveDir, relativeTo: scene.relativeTo });
    for (const r of results) {
      if (r.error) errors.push(`archive ${r.from}: ${r.error}`);
      else archived.push({ from: r.from, to: r.to });
    }
  }
  return { extracted, archived, errors };
}
