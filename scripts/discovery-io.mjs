// File-system side of the Discovery scouts (scripts/eroscripts-scout.mjs,
// scripts/faptap-scout.mjs): the app's config.json and lock, the library's
// folders (each scout downloads into its own source folder, EroScripts or
// FapTap), and the Discovery playlist. config.json is written only while the
// app is closed; otherwise finished videos wait in
// `<userData>/eroscripts-scout-pending.json` for the next run of either scout.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { addToDiscovery, coreName, DISCOVERY_NAME, isVideoName, normName, remapPaths } from './eroscripts-scout-lib.mjs';

const configPath = (userData) => path.join(userData, 'config.json');
export const readConfig = (userData) => JSON.parse(fs.readFileSync(configPath(userData), 'utf8'));

/** The app holds Chromium's SingletonLock (`<host>-<pid>`) in userData while it runs. */
export function appRunning(userData) {
  let target;
  try { target = fs.readlinkSync(path.join(userData, 'SingletonLock')); } catch { return false; }
  const pid = Number(target.split('-').pop());
  if (!pid) return true;
  try { process.kill(pid, 0); return true; } catch (err) { return err.code === 'EPERM'; }
}

export function writeConfig(userData, config) {
  const file = configPath(userData);
  fs.copyFileSync(file, `${file}.bak-discovery`);
  const tmp = `${file}.tmp-discovery`;
  fs.writeFileSync(tmp, JSON.stringify(config, null, '\t'));
  fs.renameSync(tmp, file);
}

/**
 * Remove a folder that holds nothing but `._*` / `.DS_Store` leftovers. exFAT
 * lists names decomposed (NFD) but finds some `._` files only by their
 * composed (NFC) spelling, so both are tried; a folder that won't go stays.
 */
export function removeIfEmpty(dir) {
  let names;
  try { names = fs.readdirSync(dir); } catch { return; }
  if (names.some((n) => !n.startsWith('._') && n !== '.DS_Store')) return;
  for (const n of names) for (const form of [n, n.normalize('NFC')]) fs.rmSync(path.join(dir, form), { force: true });
  try { fs.rmdirSync(dir); } catch { return; }
  const side = path.join(path.dirname(dir), `._${path.basename(dir)}`);
  for (const form of [side, side.normalize('NFC')]) fs.rmSync(form, { force: true });
}

/** Enabled library sources other than the scout's own download folder. */
export function librarySources(config, incoming) {
  return (config.settings?.library?.sources || [])
    .filter((s) => s.enabled !== false && s.path && path.resolve(s.path) !== path.resolve(incoming || '/nonexistent'));
}

/** Visit files and folders up to four levels down, skipping dot names (AppleDouble `._*` included). */
export function walk(dir, depth, visit) {
  let entries = [];
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const e of entries) {
    if (e.name.startsWith('.')) continue;
    const full = path.join(dir, e.name);
    if (e.isDirectory()) { visit(full, e, depth); if (depth < 3) walk(full, depth + 1, visit); } else visit(full, e, depth);
  }
}

/** What the library already has: creators (lower case, and as spelled), scene folder names, video names. */
export function libraryState(roots) {
  const creators = new Set();
  const creatorNames = new Map();
  const sceneNames = [];
  const videos = [];
  for (const root of roots) {
    walk(root, 0, (full, e, depth) => {
      if (e.isDirectory()) {
        if (depth === 0) {
          creators.add(e.name.toLowerCase());
          if (!creatorNames.has(e.name.toLowerCase())) creatorNames.set(e.name.toLowerCase(), e.name.normalize('NFC'));
        }
        if (depth === 1) sceneNames.push(normName(e.name));
      } else if (isVideoName(e.name)) videos.push(coreName(e.name));
    });
  }
  return { creators, creatorNames: [...creatorNames.values()], sceneNames, videos };
}

/** Rename `from` to `to`, with its AppleDouble `._` sidecar if the volume keeps one. */
export function renameWithSidecar(from, to) {
  fs.renameSync(from, to);
  const side = (p) => path.join(path.dirname(p), `._${path.basename(p)}`);
  if (fs.existsSync(side(from)) && !fs.existsSync(side(to))) fs.renameSync(side(from), side(to));
}

/**
 * Carry out `retitlePlan` moves: rename each scene's files and folder (a
 * scene that fails is put back), point its sidecar's `file` at the new video
 * name, and rewrite every path config.json and the pending playlist adds
 * keep for it. The app must be closed (config.json).
 *
 * @param {object[]} moves retitlePlan's result
 * @param {{userData: string, incoming: string, sidecar: string}} o sidecar: `eroscripts.json` / `faptap.json`
 * @returns {number} exit code: 0, or 1 when a scene could not be renamed
 */
export function applyMoves(moves, { userData, incoming, sidecar }) {
  const rel = (p) => path.relative(incoming, p);
  const done = [];
  let failed = 0;
  for (const m of moves) {
    const renamed = [];
    try {
      for (const f of m.files) {
        renameWithSidecar(path.join(m.from, f.from), path.join(m.from, f.to));
        renamed.push(f);
      }
      if (path.resolve(m.to) !== path.resolve(m.from)) {
        fs.mkdirSync(path.dirname(m.to), { recursive: true });
        renameWithSidecar(m.from, m.to);
      }
    } catch (err) {
      for (const f of renamed.reverse()) {
        try { renameWithSidecar(path.join(m.from, f.to), path.join(m.from, f.from)); } catch { /* reported below */ }
      }
      console.log(`✗ ${rel(m.from)}: ${err.message}`);
      failed++;
      continue;
    }
    const file = path.join(m.to, sidecar);
    fs.writeFileSync(file, JSON.stringify({ ...JSON.parse(fs.readFileSync(file, 'utf8')), file: m.video }, null, 1));
    removeIfEmpty(path.dirname(m.from));
    const to = onDiskPath(m.to);
    done.push({ from: m.from, to, files: m.files.map((f) => ({ from: f.from, to: path.basename(onDiskPath(path.join(to, f.to))) })) });
  }
  if (done.length) {
    const { value, changed } = remapPaths(readConfig(userData), done);
    writeConfig(userData, value);
    const pendingFile = path.join(userData, 'eroscripts-scout-pending.json');
    if (fs.existsSync(pendingFile)) {
      fs.writeFileSync(pendingFile, JSON.stringify(remapPaths(JSON.parse(fs.readFileSync(pendingFile, 'utf8')), done).value, null, 1));
    }
    console.log(`Renamed ${done.length} scenes; ${changed} paths updated in config.json`);
  }
  return failed ? 1 : 0;
}

/**
 * `p` spelled the way the drive lists it. Names from MEGA come in one Unicode
 * form (NFC) while some volumes list another, a folder may exist in another
 * case (`bubukuin` for `Bubukuin`), and the playlist and headset match the
 * library scan's paths byte for byte.
 */
export function onDiskPath(p) {
  let cur = path.sep;
  for (const seg of path.resolve(p).split(path.sep).filter(Boolean)) {
    let hit = seg;
    try {
      const names = fs.readdirSync(cur);
      const nfc = seg.normalize('NFC');
      hit = names.find((n) => n.normalize('NFC') === nfc) ?? names.find((n) => n.normalize('NFC').toLowerCase() === nfc.toLowerCase()) ?? seg;
    } catch { /* keep as given */ }
    cur = path.join(cur, hit);
  }
  return cur;
}

/**
 * Add finished videos (and any left from earlier runs) to Discovery, with the
 * scout's download folder (`{name, path}`) as a library source; false when the app is open.
 */
export function flushPlaylist(userData, folder, videoPaths) {
  const pendingFile = path.join(userData, 'eroscripts-scout-pending.json');
  let pending = [];
  try { pending = JSON.parse(fs.readFileSync(pendingFile, 'utf8')); } catch { /* none */ }
  const all = [...new Set([...pending, ...videoPaths].filter((p) => fs.existsSync(p)).map(onDiskPath))];
  if (!all.length) return true;
  if (appRunning(userData)) { fs.writeFileSync(pendingFile, JSON.stringify(all, null, 1)); return false; }
  const config = readConfig(userData);
  const changes = addToDiscovery(config, all, { folder, newId: randomUUID });
  if (changes.sourceAdded || changes.playlistCreated || changes.added.length) writeConfig(userData, config);
  fs.rmSync(pendingFile, { force: true });
  if (changes.sourceAdded) console.log(`  added library source "${folder.name}" → ${folder.path}`);
  if (changes.playlistCreated) console.log(`  created playlist "${DISCOVERY_NAME}"`);
  return true;
}

/**
 * After downloads: remove extra copies of a video the library now holds twice
 * (`npm run dedupe:scenes -- --apply`; rules in scripts/scene-dedupe-lib.mjs).
 */
export function dedupeAfterFetch(userData) {
  const cli = path.join(path.dirname(fileURLToPath(import.meta.url)), 'dedupe-scenes.mjs');
  const r = spawnSync(process.execPath, [cli, '--apply', '--quiet', '--user-data', userData], { stdio: 'inherit' });
  if (r.status !== 0) console.log('  duplicate check failed; run npm run dedupe:scenes to see why');
}

/**
 * A scout's download folder (`{name, path}`): --incoming, else the library
 * source called `name` (EroScripts / FapTap), else `<name>` beside the first
 * library source (the library:reorganize layout).
 */
export function downloadFolder(config, name, override) {
  if (override) return { name, path: override };
  const all = (config.settings?.library?.sources || []).filter((s) => s.path);
  const own = all.find((s) => s.name === name);
  if (own) return { name, path: own.path };
  const first = all.find((s) => s.enabled !== false);
  if (!first) throw new Error(`No library source to put ${name} next to; pass --incoming <dir>.`);
  return { name, path: path.join(path.dirname(first.path), name) };
}
