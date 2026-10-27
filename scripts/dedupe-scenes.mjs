#!/usr/bin/env node
// npm run dedupe:scenes — find videos the library holds more than once and
// keep the best copy (rules: scripts/scene-dedupe-lib.mjs).
//
//   npm run dedupe:scenes                  list duplicate groups and what would go
//   npm run dedupe:scenes -- --apply       delete the extra copies
//
// Every enabled library source is read (Discovery included). Video lengths,
// sizes and resolutions are cached in `<userData>/scene-probe-cache.json`.
// An extra copy goes the way the app's triage deletes a scene
// (electron/rejected-videos.js, reason `duplicate`), so the scouts don't
// download it again; a second encode in the same folder as the copy that
// stays loses only its video file, since the two share their scripts.
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
const { appRunning, readConfig, writeConfig } = await import('./discovery-io.mjs');
const { defaultUserDataDir } = await import('./eroscripts-sync-lib.mjs');
const { VIDEO_EXTS, groupScenes, readStrokeScript, sceneHandmadeAxes, actionsHash, isStrokeScriptName } = await import('./handmade-scenes.mjs');
const { probeVideo, readProbeCache, writeProbeCache } = await import('./scene-probe.mjs');
const require = createRequire(import.meta.url);
const rejectedVideos = require('../electron/rejected-videos.js');
const run = promisify(execFile);

const USAGE = `Usage: npm run dedupe:scenes -- [options]
Find videos the library holds more than once (same length, same title, same picture or
identical script) and keep one: EroScripts over FapTap, hand-made axes over generated,
passthrough, within VR caps (8K, 60 fps), resolution, fps, faster script, bitrate.
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

// Five small grey frames per video (left eye for VR), kept for the run.
const frameCache = new Map();
async function frames(scene) {
  if (frameCache.has(scene.video)) return frameCache.get(scene.video);
  const vf = `${lib.projectionOf(scene).vr ? 'crop=iw/2:ih:0:0,' : ''}scale=48:27:flags=area,format=gray`;
  const out = [];
  for (let k = 1; k <= 5; k++) {
    try {
      const { stdout } = await run('ffmpeg', ['-v', 'error', '-ss', (scene.duration * k / 6).toFixed(2), '-i', scene.video,
        '-frames:v', '1', '-vf', vf, '-f', 'rawvideo', '-'], { encoding: 'buffer', maxBuffer: 1 << 20 });
      out.push(stdout);
    } catch { out.push(Buffer.alloc(0)); }
  }
  frameCache.set(scene.video, out);
  return out;
}

async function sameFrames(a, b) {
  const fa = await frames(a);
  const fb = await frames(b);
  let total = 0; let count = 0;
  for (let i = 0; i < fa.length; i++) {
    if (!fa[i].length || fa[i].length !== fb[i].length) return false;
    for (let p = 0; p < fa[i].length; p++) total += Math.abs(fa[i][p] - fb[i][p]);
    count += fa[i].length;
  }
  return count > 0 && total / count < lib.SAME_FRAMES_MAX;
}

// ── Library ─────────────────────────────────────────────────────────────

/** Every scene video under the enabled library sources. */
async function readScenes(roots, cache) {
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
        const info = await probeVideo(file, cache);
        const main = await parse(scripts[0]);
        const { handmade } = await sceneHandmadeAxes(scripts, visible, parse);
        const stem = v.slice(0, -path.extname(v).length);
        scenes.push({
          video: file, dir, source, artist, ...info,
          handmadeAxes: handmade.length,
          passthrough: /passthrough/i.test(path.relative(root, file)),
          speed: lib.scriptSpeed(main?.actions),
          scriptHash: actionsHash(main),
          words: lib.titleWords(stem, artist),
          name: v,
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
  const scenes = await readScenes(roots, cache);
  writeProbeCache(userData, cache);
  const groups = await lib.findDuplicates(scenes, sameFrames);

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
      for (const r of g.remove) console.log(`remove ${rel(r.scene.video)}  [${desc(r.scene)}]  (${r.reason})`);
    }
    console.log('');
  }

  const deleted = [];
  if (opts.apply) {
    for (const g of groups) {
      for (const { scene } of g.remove) {
        try {
          if (scene.dir === g.keep.dir) {
            // Another encode of the kept video, sharing its scripts: only the video goes.
            for (const n of [scene.name, `._${scene.name}`]) for (const form of new Set([n, n.normalize('NFC')])) fs.rmSync(path.join(scene.dir, form), { force: true });
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
