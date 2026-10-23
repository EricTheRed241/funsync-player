// prune:generated — hand-made axis scripts always win, per folder.
//
// For every folder where any stroke script has hand-made axes (a companion
// that isn't `.generated.`, or axes embedded in the script; see
// renderer/js/handmade-axes.js):
//   - embedded axes are written out as `<stem>.<axis>.funscript` companions
//     of their script (canonical suffix; an existing file or a hand-made
//     companion for the axis is never replaced; the script is not touched);
//   - every sibling video (another scene in the folder: `_4k`, `_hard`,
//     VR / flat …) whose scripts have no hand-made axes gets a copy of one
//     script's hand-made axis set as `<its script stem>.<axis>.funscript`,
//     but only when the two stroke scripts' timing provably matches
//     (`timingMatch` in handmade-scenes.mjs). An existing file is never
//     replaced. A sibling that doesn't match gets nothing and is reported;
//   - every `<stem>.generated.<axis>.funscript` in the folder (any stem) is
//     moved into the archive at `<archive>/<path relative to the library
//     folder's parent>`, and `original<TAB>archived` is appended to
//     `<archive>/MANIFEST.txt`.
// macOS `._` AppleDouble files are ignored. Nothing is ever deleted.

import fs from 'node:fs/promises';
import path from 'node:path';
import { extractEmbeddedAxes, buildCompanionFiles } from '../renderer/js/embedded-multi-axis.js';
import { getBaseName, isGeneratedCompanion, listCompanionFiles } from '../renderer/js/multi-axis.js';
import { handmadeAxisSuffixes, canonicalAxisSuffix } from '../renderer/js/handmade-axes.js';
import { groupScenes, readStrokeScript, sceneHandmadeAxes, timingMatch } from './handmade-scenes.mjs';

const MANIFEST = 'MANIFEST.txt';
const MANIFEST_HEADER = 'Moved by funsync-player prune:generated (hand-made axes win). '
  + 'Restore: move each file back to its original path.\n';
const FUNSCRIPT = '.funscript';

/** Filenames of the files in `dir`, AppleDouble left out; [] when unreadable. */
export async function listFolder(dir) {
  try {
    const entries = await fs.readdir(dir, { withFileTypes: true });
    return entries.filter((e) => e.isFile() && !e.name.startsWith('._')).map((e) => e.name);
  } catch {
    return [];
  }
}

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

/** Embedded axes of a parsed script by canonical suffix (first spelling wins). */
function embeddedByCanonical(parsed) {
  const embedded = new Map();
  for (const [suffix, actions] of extractEmbeddedAxes(parsed)) {
    const canonical = canonicalAxisSuffix(suffix);
    if (!embedded.has(canonical)) embedded.set(canonical, actions);
  }
  return embedded;
}

/** Companion files to write for the axes embedded in one script. */
function plannedExtractions(dir, name, parsed, names) {
  const covered = new Set(handmadeAxisSuffixes({ primaryPath: name, allFiles: names }));
  const taken = new Set(names.map((n) => n.toLowerCase()));
  const stem = name.slice(0, -FUNSCRIPT.length);
  const embedded = embeddedByCanonical(parsed);
  const extract = [];
  for (const file of buildCompanionFiles(embedded)) {
    const target = `${stem}.${file.suffix}${FUNSCRIPT}`;
    if (covered.has(file.suffix) || taken.has(target.toLowerCase())) continue;
    extract.push({ path: path.join(dir, target), suffix: file.suffix, content: file.content });
  }
  return { embedded: [...embedded.keys()].sort(), extract };
}

/**
 * One script's hand-made axis set: companion files first (their own suffix
 * spelling), then embedded axes they don't cover.
 * @returns {Map<string, {ending: string, from?: string, content?: object}>}
 *   canonical suffix → name ending (`.twist.funscript`) and the file to copy
 *   or the content to write
 */
function handmadeSet(dir, name, parsed, names) {
  const set = new Map();
  for (const c of listCompanionFiles(name, names)) {
    if (c.generated || set.has(c.axis.suffix)) continue;
    set.set(c.axis.suffix, { ending: c.path.slice(getBaseName(c.path).length), from: path.join(dir, c.path) });
  }
  for (const file of buildCompanionFiles(embeddedByCanonical(parsed))) {
    if (!set.has(file.suffix)) set.set(file.suffix, { ending: `.${file.suffix}${FUNSCRIPT}`, content: file.content });
  }
  return set;
}

/** The scene's own stroke script: the one named as its video, else its first. */
function ownScript(scene) {
  const stem = scene.video.slice(0, -path.extname(scene.video).length).toLowerCase();
  return scene.scripts.find((n) => n.slice(0, -FUNSCRIPT.length).toLowerCase() === stem) || scene.scripts[0];
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
 * Plan one folder. Reads only.
 *
 * @param {string} dir — absolute
 * @param {string} archiveDir
 * @param {string} relativeTo — the library folder's parent
 * @param {string[]} [names] — the folder's listing; read when absent
 * @returns {Promise<null|{
 *   dir: string,
 *   relativeTo: string,
 *   scenes: Array<{video: string|null, scripts: string[], handmade: string[], embedded: string[]}>,
 *   extract: Array<{path: string, suffix: string, content: object}>,
 *   siblings: Array<{video: string, script: string, source: string|null, match: 'identical'|'near'|null,
 *     copy: Array<{path: string, suffix: string, from?: string, content?: object}>}>,
 *   archive: Array<{from: string, to: string}>,
 * }>} null when no script in the folder has hand-made axes; paths absolute
 */
export async function planFolder(dir, archiveDir, relativeTo, names = null) {
  const listing = names || await listFolder(dir);
  const parsedByName = new Map();
  const parse = async (name) => {
    if (!parsedByName.has(name)) parsedByName.set(name, await readStrokeScript(path.join(dir, name)));
    return parsedByName.get(name);
  };

  const scenes = [];
  const others = [];
  const sources = [];
  for (const scene of groupScenes(listing)) {
    const { handmade, byScript } = await sceneHandmadeAxes(scene.scripts, listing, parse);
    if (handmade.length === 0) others.push(scene);
    else {
      scenes.push({ scene, handmade });
      sources.push(...byScript.keys());
    }
  }
  if (scenes.length === 0) return null;
  sources.sort();

  const extract = [];
  const planned = [];
  for (const { scene, handmade } of scenes) {
    const embedded = new Set();
    for (const name of scene.scripts) {
      const p = plannedExtractions(dir, name, await parse(name), listing);
      for (const s of p.embedded) embedded.add(s);
      extract.push(...p.extract);
    }
    planned.push({
      video: scene.video ? path.join(dir, scene.video) : null,
      scripts: scene.scripts.map((n) => path.join(dir, n)),
      handmade,
      embedded: [...embedded].sort(),
    });
  }

  // A sibling has no hand-made companion (else its scene would have
  // hand-made axes), so the targets are free; applyPrune still never
  // replaces a file that appears meanwhile.
  const siblings = [];
  for (const scene of others) {
    if (!scene.video) continue;
    const script = ownScript(scene);
    const parsed = await parse(script);
    let best = null;
    for (const src of sources) {
      const match = timingMatch(await parse(src), parsed);
      if (match === 'identical') { best = { src, match }; break; }
      if (match && !best) best = { src, match };
    }
    const copy = [];
    if (best) {
      const stem = script.slice(0, -FUNSCRIPT.length);
      for (const [suffix, item] of handmadeSet(dir, best.src, await parse(best.src), listing)) {
        copy.push({ path: path.join(dir, stem + item.ending), suffix, ...(item.from ? { from: item.from } : { content: item.content }) });
      }
    }
    siblings.push({
      video: path.join(dir, scene.video),
      script: path.join(dir, script),
      source: best ? path.join(dir, best.src) : null,
      match: best ? best.match : null,
      copy,
    });
  }

  const archive = listing.filter((n) => isGeneratedCompanion(n)).map((n) => {
    const from = path.join(dir, n);
    return { from, to: archiveTarget(from, archiveDir, relativeTo) };
  });
  return { dir, relativeTo, scenes: planned, extract, siblings, archive };
}

/** True when carrying out a folder plan changes any file. */
export function folderChanges(folder) {
  return folder.extract.length > 0 || folder.archive.length > 0
    || folder.siblings.some((s) => s.copy.length > 0);
}

/**
 * Plan the prune for library folders. Reads only.
 *
 * @param {string[]} roots — library folders
 * @param {string} archiveDir
 * @returns {Promise<object[]>} `planFolder` results of every folder with
 *   hand-made axes, in walk order
 */
export async function planPrune(roots, archiveDir) {
  const folders = [];
  for (const rootArg of roots) {
    const root = path.resolve(rootArg);
    const relativeTo = path.dirname(root);
    const listed = [];
    await walk(root, listed);
    for (const { dir, names } of listed) {
      const plan = await planFolder(dir, archiveDir, relativeTo, names);
      if (plan) folders.push(plan);
    }
  }
  return folders;
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
 * Carry out folder plans from `planFolder` / `planPrune`: write the
 * extracted companions, copy hand-made axes to timing-compatible siblings
 * (never replacing a file), then archive the generated files.
 *
 * @returns {Promise<{extracted: string[], copied: string[], archived: Array<{from: string, to: string}>, errors: string[]}>}
 */
export async function applyPrune(folders, archiveDir) {
  const extracted = [];
  const copied = [];
  const archived = [];
  const errors = [];
  for (const folder of folders) {
    for (const file of folder.extract) {
      try {
        await fs.writeFile(file.path, JSON.stringify(file.content) + '\n', { encoding: 'utf8', flag: 'wx' });
        extracted.push(file.path);
      } catch (err) {
        errors.push(`extract ${file.path}: ${err.message}`);
      }
    }
    for (const sibling of folder.siblings) {
      for (const file of sibling.copy) {
        try {
          if (file.from) await fs.copyFile(file.from, file.path, fs.constants.COPYFILE_EXCL);
          else await fs.writeFile(file.path, JSON.stringify(file.content) + '\n', { encoding: 'utf8', flag: 'wx' });
          copied.push(file.path);
        } catch (err) {
          errors.push(`copy ${file.path}: ${err.message}`);
        }
      }
    }
    if (folder.archive.length === 0) continue;
    const results = await archiveFiles(folder.archive.map((a) => a.from), { archiveDir, relativeTo: folder.relativeTo });
    for (const r of results) {
      if (r.error) errors.push(`archive ${r.from}: ${r.error}`);
      else archived.push({ from: r.from, to: r.to });
    }
  }
  return { extracted, copied, archived, errors };
}
