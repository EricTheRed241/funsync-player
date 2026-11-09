#!/usr/bin/env node
// npm run dedupe:scenes — find videos the library holds more than once and
// keep one copy: the biggest video with the best scripts (rules:
// scripts/scene-dedupe-lib.mjs).
//
//   npm run dedupe:scenes                  list duplicate groups and what would go
//   npm run dedupe:scenes -- --apply       delete the extra copies
//
// Every enabled library source is read (Discovery included). Video lengths,
// sizes and resolutions are cached in `<userData>/scene-probe-cache.json`,
// frame hashes in `<userData>/scene-frame-cache.json`; whether a video is VR
// comes from the app's stereo check (`<userData>/vr-stereo-cache.json`) when
// it has looked at the file.
// When the scripts that stay belong to another copy, that copy's main script
// and its axis files are copied next to the kept video under its name,
// replacing the kept video's own. An extra copy then goes the way the app's
// triage deletes a scene (electron/rejected-videos.js, reason `duplicate`),
// so the scouts don't download it again; a copy in the same folder as the
// kept one loses its video and its own scripts (scripts the folder shares
// stay).
// Playlists drop the deleted paths while the app is closed; with the app
// open they keep them, hidden (the app doesn't show missing files).
//
// Exit codes: 0 done, 1 error.

import { execFile, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { promisify } from 'node:util';

if (!process.execArgv.includes('--disable-warning=MODULE_TYPELESS_PACKAGE_JSON')) {
  const relaunch = spawnSync(process.execPath, [
    '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON',
    ...process.argv.slice(1),
  ], { stdio: 'inherit' });
  process.exit(relaunch.status ?? 1);
}

const lib = await import('./scene-dedupe-lib.mjs');
const { DEDUPE_KEEP_ORDER, DEDUPE_SCRIPT_ORDER } = await import('../renderer/js/content-preferences.js');
const { appRunning, readConfig, writeConfig } = await import('./discovery-io.mjs');
const { defaultUserDataDir } = await import('./eroscripts-sync-lib.mjs');
const { VIDEO_EXTS, groupScenes, readStrokeScript, sceneHandmadeAxes, actionsHash, isStrokeScriptName } = await import('./handmade-scenes.mjs');
const { probeVideo, readFrameCache, readProbeCache, readStereoCache, stereoVr, writeFrameCache, writeProbeCache } = await import('./scene-probe.mjs');
const require = createRequire(import.meta.url);
const rejectedVideos = require('../electron/rejected-videos.js');
const run = promisify(execFile);

const USAGE = `Usage: npm run dedupe:scenes -- [options]
Find videos the library holds more than once (same length and projection, and the same
picture or identical script) and keep one: the video by ${DEDUPE_KEEP_ORDER.join(', ')};
its scripts by ${DEDUPE_SCRIPT_ORDER.join(', ')} (moved over from another copy when needed).
  --apply                delete the extra copies (default: only list them)
  --quiet                print only the summary line (used after scout downloads)
  --user-data <dir>      app data folder (default: the desktop app's)`;

function parseArgs(argv) {
  const opts = { apply: false, quiet: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--apply') opts.apply = true;
    else if (a === '--quiet') opts.quiet = true;
    else if (a === '--user-data') { if (!argv[i + 1]) throw new Error('--user-data needs a value'); opts.userData = path.resolve(argv[++i]); }
    else if (a === '-h' || a === '--help') opts.help = true;
    else throw new Error(`Unknown option: ${a}`);
  }
  return opts;
}

// ── Frames ──────────────────────────────────────────────────────────────

// Bursts of frame hashes (left eye for VR) by video and time, kept in
// `<userData>/scene-frame-cache.json`; the centre frame keeps its fine grid.
let frameCache = {};
let frameCacheDir = null;
let unsaved = 0;
const failed = new Set();
const FRAME_BYTES = lib.FRAME_W * lib.FRAME_H;
const BURST_STEP = lib.BURST_OFFSETS_S[1] - lib.BURST_OFFSETS_S[0];

const encodeFrame = (f, centre) => (f ? [f.hash, f.detail, ...(centre ? [Buffer.from(f.cells).toString('base64')] : [])].join(':') : null);
function decodeFrame(s) {
  if (!s) return null;
  const [hash, detail, cells] = s.split(':');
  return { hash, detail: Number(detail), cells: cells ? new Uint8Array(Buffer.from(cells, 'base64')) : undefined };
}

async function burstAt(scene, at) {
  let entry = frameCache[scene.video];
  if (!entry || entry.size !== scene.size || entry.mtimeMs !== scene.mtimeMs) {
    entry = frameCache[scene.video] = { size: scene.size, mtimeMs: scene.mtimeMs, bursts: {} };
  }
  const vr = lib.projectionOf(scene).vr;
  const key = `${vr ? 'L' : 'F'}${at.toFixed(2)}`;
  if (entry.bursts[key]) return entry.bursts[key].map(decodeFrame);
  if (failed.has(`${scene.video}\n${key}`)) return null;
  const start = Math.max(0, at + lib.BURST_OFFSETS_S[0]);
  const vf = `fps=${1 / BURST_STEP},${vr ? 'crop=iw/2:ih:0:0,' : ''}scale=${lib.FRAME_W}:${lib.FRAME_H}:flags=area,format=gray`;
  let frames = [];
  let error = 'no frames';
  try {
    const { stdout } = await run('ffmpeg', ['-v', 'error', '-ss', start.toFixed(2), '-i', scene.video,
      '-vf', vf, '-frames:v', String(lib.BURST_OFFSETS_S.length), '-f', 'rawvideo', '-'], { encoding: 'buffer', maxBuffer: 1 << 22 });
    frames = lib.BURST_OFFSETS_S.map((_, i) => (stdout.length >= (i + 1) * FRAME_BYTES ? lib.frameHash(stdout.subarray(i * FRAME_BYTES, (i + 1) * FRAME_BYTES)) : null));
  } catch (err) {
    error = String(err.stderr || err.message).trim().split('\n')[0];
  }
  if (!frames[lib.BURST_CENTRE]) {
    // Not cached: tried again next run, not again this run.
    failed.add(`${scene.video}\n${key}`);
    console.error(`! ${scene.video}: no frame at ${at.toFixed(1)} s (${error}); compared as unknown`);
    return null;
  }
  entry.bursts[key] = frames.map((f, i) => encodeFrame(f, i === lib.BURST_CENTRE));
  if (++unsaved >= 25) { writeFrameCache(frameCacheDir, frameCache); unsaved = 0; }
  return frames;
}

function sameFrames(a, b, opts) {
  return lib.framesMatch(a, b, burstAt, opts);
}

// Axis files that go with a stroke script `<stem>.funscript`: `<stem>.<axis>.funscript`, generated ones too.
const AXIS_TAIL = /^\.(generated\.)?[a-z0-9]+\.funscript$/i;

/** A scene's script set: its main stroke script and that script's axis files (names in `scene.dir`). */
function scriptSet(scene) {
  const main = scene.scripts[0];
  const stem = main.slice(0, -'.funscript'.length);
  const names = fs.readdirSync(scene.dir).filter((n) => !n.startsWith('._'));
  return { stem, files: names.filter((n) => n === main || (n.startsWith(stem) && AXIS_TAIL.test(n.slice(stem.length)))) };
}

/** Every script of a scene's own stroke scripts with their axis files (none when it uses the folder's). */
function ownScriptFiles(scene) {
  if (!scene.ownScripts) return [];
  const names = fs.readdirSync(scene.dir).filter((n) => !n.startsWith('._'));
  const stems = scene.scripts.map((s) => s.slice(0, -'.funscript'.length));
  return names.filter((n) => stems.some((st) => n === `${st}.funscript` || (n.startsWith(st) && AXIS_TAIL.test(n.slice(st.length)))));
}

/**
 * Give the kept video the script set of `donor`: its own scripts go, the
 * donor's main script and axis files are copied in under the kept video's
 * name. Returns the copied names.
 */
function takeScripts(keep, donor) {
  const { stem, files } = scriptSet(donor);
  const keepStem = keep.name.slice(0, -path.extname(keep.name).length);
  const incoming = files.map((n) => [path.join(donor.dir, n), path.join(keep.dir, keepStem + n.slice(stem.length))]);
  const staged = incoming.map(([from, to]) => { const tmp = `${to}.dedupe-tmp`; fs.copyFileSync(from, tmp); return [tmp, to]; });
  for (const n of ownScriptFiles(keep)) fs.rmSync(path.join(keep.dir, n), { force: true });
  for (const [tmp, to] of staged) fs.renameSync(tmp, to);
  return staged.map(([, to]) => path.basename(to));
}

// ── Library ─────────────────────────────────────────────────────────────

/** Every scene video under the enabled library sources (VR by the app's stereo check when it has one). */
async function readScenes(roots, cache, stereo) {
  const scenes = [];
  const visitDir = async (dir, root) => {
    let names;
    try { names = fs.readdirSync(dir); } catch { return; }
    const visible = names.filter((n) => !n.startsWith('.'));
    const videos = visible.filter((n) => VIDEO_EXTS.has(path.extname(n).toLowerCase()));
    if (videos.length) {
      const grouped = groupScenes(visible);
      const strokeScripts = visible.filter(isStrokeScriptName);
      const parsed = new Map();
      const parse = async (n) => {
        if (!parsed.has(n)) parsed.set(n, await readStrokeScript(path.join(dir, n)));
        return parsed.get(n);
      };
      const source = visible.includes('faptap.json') ? 'faptap' : visible.includes('eroscripts.json') ? 'eroscripts' : 'library';
      const artist = path.relative(root, dir).split(path.sep)[0] || '';
      for (const v of videos) {
        // The video's own scripts, or the folder's when the scene shares them (encodes of one video).
        const own = grouped.find((s) => s.video === v)?.scripts || [];
        const scripts = own.length ? own : strokeScripts;
        if (!scripts.length) continue;
        const file = path.join(dir, v);
        const stat = fs.statSync(file);
        const info = await probeVideo(file, cache, stat);
        const main = await parse(scripts[0]);
        const { handmade } = await sceneHandmadeAxes(scripts, visible, parse);
        const stem = v.slice(0, -path.extname(v).length);
        scenes.push({
          video: file, dir, source, artist, ...info, vr: stereoVr(stereo, file, stat),
          handmadeAxes: handmade.length,
          passthrough: /passthrough/i.test(path.relative(root, file)),
          speed: lib.scriptSpeed(main?.actions),
          scriptHash: actionsHash(main),
          words: lib.titleWords(stem, artist),
          name: v,
          scripts,
          ownScripts: own.length > 0,
        });
      }
    }
    for (const n of visible) {
      const full = path.join(dir, n);
      try { if (fs.statSync(full).isDirectory()) await visitDir(full, root); } catch { /* gone */ }
    }
  };
  for (const root of roots) await visitDir(root, root);
  return scenes;
}

/** Drop deleted videos from every playlist (only while the app is closed). */
function prunePlaylists(userData, deleted) {
  if (!deleted.length || appRunning(userData)) return false;
  const config = readConfig(userData);
  const gone = new Set(deleted);
  let changed = 0;
  for (const pl of config.playlists || []) {
    const before = (pl.videoPaths || []).length;
    pl.videoPaths = (pl.videoPaths || []).filter((p) => !gone.has(p));
    changed += before - pl.videoPaths.length;
  }
  if (changed) writeConfig(userData, config);
  return true;
}

// ── Main ────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserDataDir();
  const config = readConfig(userData);
  const roots = (config.settings?.library?.sources || []).filter((s) => s.enabled !== false && s.path && fs.existsSync(s.path)).map((s) => s.path);
  if (!roots.length) throw new Error('No library source found.');

  const cache = readProbeCache(userData);
  const scenes = await readScenes(roots, cache, readStereoCache(userData));
  writeProbeCache(userData, cache);
  frameCache = readFrameCache(userData);
  frameCacheDir = userData;
  const groups = await lib.findDuplicates(scenes, sameFrames);
  writeFrameCache(userData, frameCache);

  const rel = (f) => {
    const r = roots.find((x) => f.startsWith(x + path.sep));
    return r ? path.join(path.basename(r), path.relative(r, f)) : f;
  };
  const gb = (bytes) => (bytes / 1e9).toFixed(1);
  const desc = (s) => `${s.width}x${s.height} ${Math.round(s.fps)}fps ${s.source}${s.handmadeAxes ? ` ${s.handmadeAxes} axes` : ''}${s.passthrough ? ' passthrough' : ''} speed ${Math.round(s.speed)}`;
  const removing = groups.flatMap((g) => g.remove);
  const bytes = removing.reduce((n, r) => n + (r.scene.size || 0), 0);
  if (!opts.quiet) {
    for (const g of groups) {
      console.log(`\nkeep   ${rel(g.keep.video)}  [${desc(g.keep)}]`);
      if (g.script !== g.keep) console.log(`  scripts from ${rel(g.script.video)}  (${g.scriptReason})`);
      for (const r of g.remove) console.log(`remove ${rel(r.scene.video)}  [${desc(r.scene)}]  (${r.reason})`);
    }
    console.log('');
  }

  const deleted = [];
  if (opts.apply) {
    for (const g of groups) {
      // The kept video gets the scripts that stay under its own name (also
      // when it only used the folder's, which a removed copy may own).
      if (g.script !== g.keep || !g.keep.ownScripts) {
        try {
          const copied = takeScripts(g.keep, g.script);
          if (!opts.quiet && g.script !== g.keep) console.log(`moved ${copied.length} script file(s) to ${rel(g.keep.dir)}`);
        } catch (err) {
          console.error(`! ${rel(g.keep.video)}: scripts not moved, copies kept: ${err.message}`);
          continue;
        }
      }
      for (const { scene } of g.remove) {
        try {
          if (scene.dir === g.keep.dir) {
            // A copy in the kept video's folder: its video and its own scripts go; scripts the folder shares stay.
            const keepStem = g.keep.name.slice(0, -path.extname(g.keep.name).length);
            for (const n of [scene.name, ...ownScriptFiles(scene).filter((f) => !f.startsWith(`${keepStem}.`))]) {
              for (const form of new Set([n, n.normalize('NFC')])) {
                fs.rmSync(path.join(scene.dir, form), { force: true });
                fs.rmSync(path.join(scene.dir, `._${form}`), { force: true });
              }
            }
          } else {
            rejectedVideos.deleteRejectedVideo(scene.video, { sources: roots, userDataDir: userData, reason: 'duplicate' });
          }
          deleted.push(scene.video);
        } catch (err) {
          console.error(`! ${rel(scene.video)}: ${err.message}`);
        }
      }
    }
  }
  const pruned = prunePlaylists(userData, deleted);
  const what = opts.apply ? `removed ${deleted.length}` : `would remove ${removing.length}`;
  console.log(`Duplicates: ${groups.length} group(s) among ${scenes.length} videos; ${what} (${gb(bytes)} GB)${opts.apply && deleted.length && !pruned ? '; the app is open, so playlists keep the deleted paths (hidden as missing)' : ''}`);
  return 0;
}

main().then((code) => process.exit(code), (err) => {
  console.error(`dedupe:scenes: ${err.message}`);
  process.exit(1);
});
