// npm run library:reorganize — sort the library's scene folders into one
// folder per source and rewrite every path the app keeps for them.
//
//   npm run library:reorganize -- --dry-run          show the plan, change nothing
//   npm run library:reorganize                       move the scenes (the app must be closed)
//   npm run library:reorganize -- --undo <log>       put a run back
//
// Scenes go to `<root>/EroScripts|FapTap|Other/<creator>/<scene>/`
// (scripts/library-reorganize-lib.mjs decides which). Folders are renamed on
// the same volume, so nothing is copied. Then every path under a moved folder
// is rewritten in config.json (playlists, ratings, resume points, categories,
// collections, script pairings, thumbnails …) and in the other JSON files in
// userData (EroScripts links, scout lists, caches), each backed up first as
// `<file>.bak-reorganize`, and the library sources become the new folders.
// The run is logged to `<userData>/logs/library-reorganize-<time>.json` for
// --undo. The app's next scan rebuilds what it derives (auto categories,
// collections, artist playlists, thumbnails).
//
// Exit codes: 0 done, 1 error or some scenes not moved, 2 the app is running.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { randomUUID } from 'node:crypto';

// renderer/*.js (via eroscripts-scout-lib) is ESM inside a CommonJS package.
// Re-exec once so Node does not print MODULE_TYPELESS_PACKAGE_JSON.
if (!process.execArgv.includes('--disable-warning=MODULE_TYPELESS_PACKAGE_JSON')) {
  const relaunch = spawnSync(process.execPath, [
    '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON',
    ...process.argv.slice(1),
  ], { stdio: 'inherit' });
  process.exit(relaunch.status ?? 1);
}

const { planReorganize, sourcesAfter, reverseMoves, SOURCE_FOLDERS } = await import('./library-reorganize-lib.mjs');
const { remapPaths } = await import('./eroscripts-scout-lib.mjs');
const { appRunning, onDiskPath, readConfig, removeIfEmpty, renameWithSidecar } = await import('./discovery-io.mjs');
const { defaultUserDataDir } = await import('./eroscripts-sync-lib.mjs');
const require = createRequire(import.meta.url);
const { VIDEO_EXTS, isAppleDoubleSidecar } = require('../electron/library-scan.js');
const { readLinksFile } = require('../electron/eroscripts-links.js');

const USAGE = `Usage: npm run library:reorganize -- [options]

Sort every scene folder of the library into <root>/EroScripts, <root>/FapTap
and <root>/Other, and rewrite the paths the app keeps. Close the app first.

  --dry-run           show what would move, change nothing (the app may run)
  --root <dir>        folder the source folders go in (default: the first library source's parent)
  --undo <log>        put back the run logged in <log> (logs/library-reorganize-*.json)
  --user-data <dir>   app data folder (default: the desktop app's)
  -h, --help          this help`;

function parseArgs(argv) {
  const opts = { dryRun: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const value = () => {
      if (i + 1 >= argv.length) throw new Error(`${a} needs a value`);
      return argv[++i];
    };
    switch (a) {
      case '--dry-run': opts.dryRun = true; break;
      case '--root': opts.root = path.resolve(value()); break;
      case '--undo': opts.undo = path.resolve(value()); break;
      case '--user-data': opts.userData = path.resolve(value()); break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  return opts;
}

const VIDEO_EXT = new Set(VIDEO_EXTS);
const fold = (p) => path.resolve(p).normalize('NFC').toLowerCase();

/** Every folder under `root` that holds a video, with its file names. */
function sceneFolders(root) {
  const scenes = new Map();
  const visit = (dir) => {
    let entries = [];
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const e of entries) {
      if (e.name.startsWith('.') || isAppleDoubleSidecar(e.name)) continue;
      const full = path.join(dir, e.name);
      if (e.isDirectory()) visit(full);
      else if (VIDEO_EXT.has(path.extname(e.name).toLowerCase())) {
        if (!scenes.has(dir)) scenes.set(dir, { dir, videos: [], files: entries.filter((x) => x.isFile() && !isAppleDoubleSidecar(x.name)).map((x) => x.name) });
        scenes.get(dir).videos.push(full);
      }
    }
  };
  visit(root);
  return [...scenes.values()];
}

/** Indentation a JSON file was written with, to write it back the same way. */
function indentOf(text) {
  const m = text.match(/^[[{]\r?\n([ \t]+)/);
  if (!m) return 0;
  return m[1][0] === '\t' ? '\t' : m[1].length;
}

function writeJson(file, value, indent) {
  const tmp = `${file}.tmp-reorganize`;
  fs.writeFileSync(tmp, JSON.stringify(value, null, indent));
  fs.renameSync(tmp, file);
}

/** Rename the folders; returns the moves done (target spelled as the drive lists it) and the failures. */
function moveScenes(moves) {
  const done = [];
  const failed = [];
  for (const m of moves) {
    try {
      fs.mkdirSync(path.dirname(m.to), { recursive: true });
      renameWithSidecar(m.from, m.to);
      done.push({ from: m.from, to: onDiskPath(m.to), files: [] });
    } catch (err) {
      failed.push({ dir: m.from, reason: err.message });
    }
  }
  return { done, failed };
}

/** Remove folders the moves left empty, from each scene's parent up to (not including) `stopAt`. */
function pruneEmpty(moves, stopAt) {
  const stops = new Set(stopAt.map(fold));
  const parents = [...new Set(moves.map((m) => path.dirname(m.from)))].sort((a, b) => b.length - a.length);
  for (let dir of parents) {
    while (!stops.has(fold(dir)) && dir !== path.dirname(dir)) {
      removeIfEmpty(dir);
      if (fs.existsSync(dir)) break;
      dir = path.dirname(dir);
    }
  }
}

/**
 * Rewrite paths in config.json and every other JSON file in userData; config
 * also gets `sources`. Each changed file is backed up first.
 */
function rewritePaths(userData, done, sources) {
  const report = [];
  for (const name of fs.readdirSync(userData).filter((n) => n.endsWith('.json')).sort()) {
    const file = path.join(userData, name);
    const text = fs.readFileSync(file, 'utf8');
    let data;
    try { data = JSON.parse(text); } catch { continue; }
    const { value, changed } = remapPaths(data, done);
    const isConfig = name === 'config.json';
    if (!changed && !isConfig) continue;
    if (isConfig) value.settings.library.sources = sources;
    fs.copyFileSync(file, `${file}.bak-reorganize`);
    writeJson(file, value, indentOf(text));
    report.push(`${name}: ${changed} paths`);
  }
  return report;
}

function logFile(userData) {
  const dir = path.join(userData, 'logs');
  fs.mkdirSync(dir, { recursive: true });
  return path.join(dir, `library-reorganize-${new Date().toISOString().replace(/[:.]/g, '-')}.json`);
}

function undo(opts, userData) {
  const log = JSON.parse(fs.readFileSync(opts.undo, 'utf8'));
  const back = reverseMoves(log.moves);
  console.log(`Putting back ${back.length} scenes from ${path.basename(opts.undo)}`);
  if (opts.dryRun) return 0;
  const { done, failed } = moveScenes(back);
  pruneEmpty(back, [log.root]);
  for (const name of SOURCE_FOLDERS) removeIfEmpty(path.join(log.root, name));
  for (const line of rewritePaths(userData, done, log.sourcesBefore)) console.log(`  ${line}`);
  for (const f of failed) console.log(`✗ ${f.dir}: ${f.reason}`);
  console.log(`Put back ${done.length} scenes${failed.length ? `, ${failed.length} failed` : ''}. Rescan the library in the app.`);
  return failed.length ? 1 : 0;
}

function main() {
  let opts;
  try { opts = parseArgs(process.argv.slice(2)); } catch (err) { console.error(`${err.message}\n\n${USAGE}`); return 1; }
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserDataDir();
  if (!opts.dryRun && appRunning(userData)) { console.error('The app is running; close it first (config.json is rewritten).'); return 2; }
  if (opts.undo) return undo(opts, userData);

  const config = readConfig(userData);
  const sources = config.settings?.library?.sources || [];
  const active = sources.filter((s) => s.enabled !== false && s.path && fs.existsSync(s.path));
  if (!active.length) { console.error('No library source on disk.'); return 1; }
  const root = opts.root || path.dirname(active[0].path);
  const linked = new Set(Object.keys(readLinksFile(userData).links).map(fold));
  const scenes = active.flatMap((s) => sceneFolders(s.path).map((sc) => ({
    ...sc, sourcePath: s.path, linked: sc.videos.some((v) => linked.has(fold(v))),
  })));
  const plan = planReorganize({ scenes, root, exists: (p) => fs.existsSync(p) });

  console.log(`${scenes.length} scene folders in ${active.length} sources → ${root}`);
  for (const f of SOURCE_FOLDERS) console.log(`  ${f.padEnd(10)} ${plan.counts[f]}`);
  console.log(`${plan.moves.length} to move, ${scenes.length - plan.moves.length - plan.conflicts.length} already in place, ${plan.conflicts.length} left where they are`);
  for (const c of plan.conflicts) console.log(`  ! ${path.relative(root, c.dir)}: ${c.reason}${c.to ? ` (${path.relative(root, c.to)})` : ''}`);
  if (opts.dryRun) {
    for (const m of plan.moves.slice(0, 8)) console.log(`  ${path.relative(root, m.from)} → ${path.relative(root, m.to)}`);
    if (plan.moves.length > 8) console.log(`  … ${plan.moves.length - 8} more`);
    console.log('Paths that would be rewritten:');
    for (const name of fs.readdirSync(userData).filter((n) => n.endsWith('.json')).sort()) {
      let data;
      try { data = JSON.parse(fs.readFileSync(path.join(userData, name), 'utf8')); } catch { continue; }
      const { changed } = remapPaths(data, plan.moves);
      if (changed) console.log(`  ${name}: ${changed}`);
    }
    return 0;
  }

  const { done, failed } = moveScenes(plan.moves);
  pruneEmpty(done, active.map((s) => s.path));
  const left = new Set([...plan.conflicts.map((c) => c.dir), ...failed.map((f) => f.dir)].map(fold));
  const emptied = active.filter((s) => !scenes.some((sc) => sc.sourcePath === s.path && left.has(fold(sc.dir)))
    && !SOURCE_FOLDERS.some((f) => fold(path.join(root, f)) === fold(s.path))).map((s) => s.path);
  for (const p of emptied) removeIfEmpty(p);
  const folders = SOURCE_FOLDERS.filter((f) => fs.existsSync(path.join(root, f)));
  const after = sourcesAfter(sources, { root, folders, emptied, newId: randomUUID });
  const log = logFile(userData);
  fs.writeFileSync(log, JSON.stringify({ at: new Date().toISOString(), root, moves: done, failed, sourcesBefore: sources }, null, 1));
  for (const line of rewritePaths(userData, done, after)) console.log(`  ${line}`);
  for (const f of failed) console.log(`✗ ${path.relative(root, f.dir)}: ${f.reason}`);
  console.log(`Moved ${done.length} scenes${failed.length ? `, ${failed.length} failed` : ''}. Sources: ${after.map((s) => s.name).join(', ')}`);
  console.log(`Log: ${log} (npm run library:reorganize -- --undo <log>). Start the app and rescan the library.`);
  return failed.length ? 1 : 0;
}

process.exit(main());
