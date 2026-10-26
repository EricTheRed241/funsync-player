// File-system side of the Discovery scouts (scripts/eroscripts-scout.mjs,
// scripts/faptap-scout.mjs): the app's config.json and lock, the library's
// folders, and the Discovery playlist. config.json is written only while the
// app is closed; otherwise finished videos wait in
// `<userData>/eroscripts-scout-pending.json` for the next run of either scout.

import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { addToDiscovery, coreName, DISCOVERY_NAME, isVideoName, normName } from './eroscripts-scout-lib.mjs';

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

/** Enabled library sources other than the Discovery folder. */
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

/** What the library already has: creators, scene folder names, video names. */
export function libraryState(roots) {
  const creators = new Set();
  const sceneNames = [];
  const videos = [];
  for (const root of roots) {
    walk(root, 0, (full, e, depth) => {
      if (e.isDirectory()) {
        if (depth === 0) creators.add(e.name.toLowerCase());
        if (depth === 1) sceneNames.push(normName(e.name));
      } else if (isVideoName(e.name)) videos.push(coreName(e.name));
    });
  }
  return { creators, sceneNames, videos };
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

/** Add finished videos (and any left from earlier runs) to Discovery; false when the app is open. */
export function flushPlaylist(userData, incoming, videoPaths) {
  const pendingFile = path.join(userData, 'eroscripts-scout-pending.json');
  let pending = [];
  try { pending = JSON.parse(fs.readFileSync(pendingFile, 'utf8')); } catch { /* none */ }
  const all = [...new Set([...pending, ...videoPaths].filter((p) => fs.existsSync(p)).map(onDiskPath))];
  if (!all.length) return true;
  if (appRunning(userData)) { fs.writeFileSync(pendingFile, JSON.stringify(all, null, 1)); return false; }
  const config = readConfig(userData);
  const changes = addToDiscovery(config, all, { incomingDir: incoming, newId: randomUUID });
  if (changes.sourceAdded || changes.playlistCreated || changes.added.length) writeConfig(userData, config);
  fs.rmSync(pendingFile, { force: true });
  if (changes.sourceAdded) console.log(`  added library source "${DISCOVERY_NAME}" → ${incoming}`);
  if (changes.playlistCreated) console.log(`  created playlist "${DISCOVERY_NAME}"`);
  return true;
}

/** The Discovery folder: --incoming, the Discovery source, else `_incoming` beside the first library source. */
export function incomingDir(config, override) {
  const sources = librarySources(config, override);
  const discovery = (config.settings?.library?.sources || []).find((s) => s.name === DISCOVERY_NAME);
  const dir = override || discovery?.path || (sources[0] && path.join(path.dirname(sources[0].path), '_incoming'));
  if (!dir) throw new Error('No library source to put _incoming next to; pass --incoming <dir>.');
  return dir;
}
