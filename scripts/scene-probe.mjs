// Video facts for duplicate checks (scripts/scene-dedupe-lib.mjs): length,
// picture size, fps and bitrate from ffprobe, cached in
// `<userData>/scene-probe-cache.json` by path, size and mtime so a library
// of hundreds of videos is read once. Frame hashes `npm run dedupe:scenes`
// takes are cached the same way in `<userData>/scene-frame-cache.json`.
// Whether a video is VR comes from the backend's stereo check
// (`<userData>/vr-stereo-cache.json`, backend/services/stereo_detect.py,
// only read here) when it has looked at the file. Used by `npm run
// dedupe:scenes` and the FapTap scout.

import { execFile } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { promisify } from 'node:util';
import { VIDEO_EXTS } from './handmade-scenes.mjs';
import { projectionOf, titleWords } from './scene-dedupe-lib.mjs';

const run = promisify(execFile);
const cachePath = (userData) => path.join(userData, 'scene-probe-cache.json');
const frameCachePath = (userData) => path.join(userData, 'scene-frame-cache.json');
// stereo_detect.ANALYSIS_VERSION: entries of another version are stale.
const STEREO_ANALYSIS_VERSION = 1;

function readJson(file) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return {}; }
}

// Save a per-path cache without entries for files that are gone.
function writeCache(file, cache) {
  for (const f of Object.keys(cache)) if (!fs.existsSync(f)) delete cache[f];
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(cache));
  fs.renameSync(tmp, file);
}

export function readProbeCache(userData) {
  return readJson(cachePath(userData));
}

/** Save the cache without entries for files that are gone. */
export function writeProbeCache(userData, cache) {
  writeCache(cachePath(userData), cache);
}

/** Frame hashes per video: `{[path]: {size, mtimeMs, bursts: {[key]: string[]}}}` (scripts/dedupe-scenes.mjs). */
export function readFrameCache(userData) {
  return readJson(frameCachePath(userData));
}

/** Save the frame cache without entries for files that are gone. */
export function writeFrameCache(userData, cache) {
  writeCache(frameCachePath(userData), cache);
}

/** The backend's stereo check results by path (`vr-stereo-cache.json` entries); `{}` without one. */
export function readStereoCache(userData) {
  const entries = readJson(path.join(userData, 'vr-stereo-cache.json')).entries;
  return entries && typeof entries === 'object' ? entries : {};
}

/**
 * VR by the backend's look at the frames: true for two eyes side by side,
 * false for one picture (a VR-shaped or VR-named frame too), undefined when
 * it has no current answer for `file` (not looked at, undecided, or the file
 * changed since).
 *
 * @param {object} stereo readStereoCache
 * @param {string} file
 * @param {{size: number, mtimeMs: number}} stat
 */
export function stereoVr(stereo, file, stat) {
  const e = stereo[file] ?? stereo[file.normalize('NFC')] ?? stereo[file.normalize('NFD')];
  if (!e || e.version !== STEREO_ANALYSIS_VERSION || e.size !== stat.size) return undefined;
  if (!(Math.abs(Number(e.mtimeNs) / 1e6 - stat.mtimeMs) < 1)) return undefined;
  return e.layout === 'sbs' ? true : e.layout === 'mono' ? false : undefined;
}

/** `{duration, width, height, fps, bitrate, size, mtimeMs}`; duration 0 when unreadable. */
export async function probeVideo(file, cache, stat = fs.statSync(file)) {
  const hit = cache[file];
  if (hit && hit.size === stat.size && hit.mtimeMs === stat.mtimeMs) return hit;
  let info = { duration: 0, width: 0, height: 0, fps: 0, bitrate: 0 };
  try {
    const { stdout } = await run('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries',
      'stream=width,height,avg_frame_rate,r_frame_rate:format=duration,bit_rate', '-of', 'json', file]);
    const j = JSON.parse(stdout);
    const s = j.streams?.[0] || {};
    const rate = (r) => { const [n, d] = String(r || '0/1').split('/').map(Number); return d ? n / d : 0; };
    info = {
      duration: Number(j.format?.duration) || 0, width: s.width || 0, height: s.height || 0,
      fps: Math.round((rate(s.avg_frame_rate) || rate(s.r_frame_rate)) * 100) / 100, bitrate: Number(j.format?.bit_rate) || 0,
    };
  } catch { /* unreadable: duration 0 keeps it out of every comparison */ }
  cache[file] = { ...info, size: stat.size, mtimeMs: stat.mtimeMs };
  return cache[file];
}

/**
 * Every video under `roots` with what a title + length duplicate check needs:
 * `{video, duration, width, height, vr, portrait, words, source}`.
 *
 * @param {string[]} roots
 * @param {object} cache readProbeCache
 * @param {object} [stereo] readStereoCache
 */
export async function libraryVideos(roots, cache, stereo = {}) {
  const out = [];
  const visit = async (dir, root) => {
    let names;
    try { names = fs.readdirSync(dir); } catch { return; }
    const visible = names.filter((n) => !n.startsWith('.'));
    const source = visible.includes('faptap.json') ? 'faptap' : visible.includes('eroscripts.json') ? 'eroscripts' : 'library';
    const artist = path.relative(root, dir).split(path.sep)[0] || '';
    for (const n of visible) {
      const full = path.join(dir, n);
      let stat;
      try { stat = fs.statSync(full); } catch { continue; }
      if (stat.isDirectory()) { await visit(full, root); continue; }
      if (!VIDEO_EXTS.has(path.extname(n).toLowerCase())) continue;
      const info = await probeVideo(full, cache, stat);
      const p = projectionOf({ name: n, ...info, vr: stereoVr(stereo, full, stat) });
      out.push({ video: full, ...info, vr: p.vr, portrait: p.portrait, words: titleWords(n.slice(0, -path.extname(n).length), artist), source });
    }
  };
  for (const root of roots) await visit(root, root);
  return out;
}
