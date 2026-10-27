// Video facts for duplicate checks (scripts/scene-dedupe-lib.mjs): length,
// picture size, fps and bitrate from ffprobe, cached in
// `<userData>/scene-probe-cache.json` by path, size and mtime so a library
// of hundreds of videos is read once. Used by `npm run dedupe:scenes` and
// the FapTap scout.

import { execFile } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { promisify } from 'node:util';
import { VIDEO_EXTS } from './handmade-scenes.mjs';
import { projectionOf, titleWords } from './scene-dedupe-lib.mjs';

const run = promisify(execFile);
const cachePath = (userData) => path.join(userData, 'scene-probe-cache.json');

export function readProbeCache(userData) {
  try { return JSON.parse(fs.readFileSync(cachePath(userData), 'utf8')); } catch { return {}; }
}

/** Save the cache without entries for files that are gone. */
export function writeProbeCache(userData, cache) {
  for (const f of Object.keys(cache)) if (!fs.existsSync(f)) delete cache[f];
  const tmp = `${cachePath(userData)}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(cache));
  fs.renameSync(tmp, cachePath(userData));
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
 */
export async function libraryVideos(roots, cache) {
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
      const p = projectionOf({ name: n, ...info });
      out.push({ video: full, ...info, vr: p.vr, portrait: p.portrait, words: titleWords(n.slice(0, -path.extname(n).length), artist), source });
    }
  };
  for (const root of roots) await visit(root, root);
  return out;
}
