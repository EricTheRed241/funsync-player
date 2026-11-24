#!/usr/bin/env node
// One-shot: stroke funscript (+ optional video) → twist / roll / pitch companions.
//
// Twist and roll are guided by the stroke and shaped like the hand-made axis
// of the scenes you like, with seeded randomness (renderer/js/guided-axis.js;
// VR and flat have their own settings). VR is decided as in `dedupe:scenes`:
// the backend's stereo check when it has looked at the file, else the
// filename or a VR-shaped frame (ffprobe). Each scene gets its own seed (from
// its name and --seed), so a re-run writes the same axes.
// Pitch is scaled to 0 / 25 / 50 / 75 / 100% around centre, with odds that
// fall later. Smooth noise then changes that scaled motion by 10–60%, so the
// build stays visible. A 0% rest sits at centre and cannot last more than 10
// seconds.
//
// The stroke file is never overwritten. An existing companion for an axis is
// skipped, unless --replace: then an existing `.generated.` companion is
// written again, the previous file kept as `<file>.<its generator>`
// (`v0-random` when it doesn't name one). --only=roll,… limits the axes.
// A folder where any stroke script has hand-made axes (a companion
// that isn't `.generated.`, or axes embedded in the stroke script) is
// skipped entirely, sibling videos included: hand-made axis scripts always
// win (`npm run prune:generated` copies them to siblings whose timing
// matches). Companions are named `<stem>.generated.<axis>.funscript` and
// written next to the video when one is known, otherwise next to the
// funscript.
//
//   node scripts/upgrade-multi-axis.mjs ./clip.mp4 ./clip.funscript
//   node scripts/upgrade-multi-axis.mjs ./library-folder
//   node scripts/upgrade-multi-axis.mjs --dry-run ./clip.funscript
//   node scripts/upgrade-multi-axis.mjs --surge --sway ./clip.mp4
//   node scripts/upgrade-multi-axis.mjs --only=roll --replace ./library-folder
//
// Prints the files it will write, then writes them. --dry-run stops after the list.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';

// renderer/*.js is ESM inside a CommonJS package. Re-exec once so Node does
// not print MODULE_TYPELESS_PACKAGE_JSON on every run.
if (!process.execArgv.includes('--disable-warning=MODULE_TYPELESS_PACKAGE_JSON')) {
  const relaunch = spawnSync(process.execPath, [
    '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON',
    ...process.argv.slice(1),
  ], { stdio: 'inherit' });
  process.exit(relaunch.status ?? 1);
}
const { createNoise1D } = await import('../renderer/js/motion-source.js');
const { parseAxisSuffix, getBaseName } = await import('../renderer/js/multi-axis.js');
const { handmadeAxisSuffixes } = await import('../renderer/js/handmade-axes.js');
const {
  planUpgrade,
  serialiseUpgradeWrites,
  companionPathForScript,
  resolveUpgradeAxes,
} = await import('../renderer/js/upgrade-multi-axis.js');
const { pairUpgradeJobs } = await import('./upgrade-pairing.mjs');
const { VIDEO_EXTS, groupScenes, folderStrokeScripts, readStrokeScript, sceneHandmadeAxes } = await import('./handmade-scenes.mjs');
const { readProbeCache, readStereoCache, stereoVr, probeVideo } = await import('./scene-probe.mjs');
const { projectionOf } = await import('./scene-dedupe-lib.mjs');
const { defaultUserDataDir } = await import('./host-env.mjs');

const MODE_LABEL = {
  guided: 'guided by the stroke, styled like your liked scenes',
  random: 'smooth noise, quieter late',
  pattern: 'slow sine, quieter late',
};

const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');
const flags = new Set(args.filter((a) => a.startsWith('--')));
const paths = args.filter((a) => !a.startsWith('--'));
const seedArg = args.find((a) => a.startsWith('--seed='));
const seed = seedArg ? Number(seedArg.slice('--seed='.length)) : 1337;
const onlyArg = args.find((a) => a.startsWith('--only='));
const only = onlyArg ? onlyArg.slice('--only='.length).split(',').map((s) => s.trim()).filter(Boolean) : null;
const replace = flags.has('--replace');

const enabled = new Set();
if (flags.has('--surge')) enabled.add('L1');
if (flags.has('--sway')) enabled.add('L2');
if (flags.has('--valve')) enabled.add('A0');

function isFunscript(p) {
  return p.toLowerCase().endsWith('.funscript');
}

function isVideo(p) {
  return VIDEO_EXTS.has(path.extname(p).toLowerCase());
}

async function walk(dir, videos, scripts) {
  const entries = await fs.readdir(dir, { withFileTypes: true });
  for (const ent of entries) {
    if (ent.name.startsWith('.')) continue;
    const full = path.join(dir, ent.name);
    if (ent.isDirectory()) {
      await walk(full, videos, scripts);
    } else if (ent.isFile()) {
      classify(full, videos, scripts);
    }
  }
}

function classify(file, videos, scripts) {
  if (isVideo(file)) videos.push(file);
  else if (isFunscript(file) && !parseAxisSuffix(path.basename(file))) scripts.push(file);
}

async function siblingStroke(video) {
  const dir = path.dirname(video);
  const want = path.basename(video, path.extname(video)) + '.funscript';
  let names;
  try {
    names = await fs.readdir(dir);
  } catch {
    return null;
  }
  const hit = names.find((n) => n.toLowerCase() === want.toLowerCase());
  if (!hit || parseAxisSuffix(hit)) return null;
  return path.join(dir, hit);
}

async function collect(roots) {
  const videos = [];
  const scripts = [];
  const explicit = new Set();
  for (const root of roots) {
    let st;
    try {
      st = await fs.stat(root);
    } catch {
      console.error(`missing: ${root}`);
      continue;
    }
    if (st.isDirectory()) await walk(root, videos, scripts);
    else {
      classify(path.resolve(root), videos, scripts);
      explicit.add(path.resolve(root));
    }
  }
  const known = new Set(scripts.map((s) => path.resolve(s)));
  for (const video of videos) {
    const sib = await siblingStroke(video);
    if (sib && !known.has(path.resolve(sib))) {
      scripts.push(sib);
      known.add(path.resolve(sib));
    }
  }
  return pairUpgradeJobs(videos, scripts, explicit);
}

function outputAnchor(video, script) {
  if (!video) return script;
  const stem = path.basename(video, path.extname(video));
  return path.join(path.dirname(video), `${stem}.funscript`);
}

/** A scene's seed: FNV-1a of its file name (NFC) mixed with --seed, so a re-run writes the same twist. */
function sceneSeed(anchor) {
  let h = 0x811c9dc5;
  for (const ch of path.basename(anchor).normalize('NFC')) {
    h = Math.imul(h ^ ch.codePointAt(0), 0x01000193) >>> 0;
  }
  return (h ^ seed) >>> 0;
}

/** VR as `dedupe:scenes` decides it: the backend's stereo check, else the name or a VR-shaped frame. */
async function sceneVr(file, caches) {
  let stat = null;
  try { stat = await fs.stat(file); } catch { /* script without a video, or gone */ }
  const isVideo = stat && VIDEO_EXTS.has(path.extname(file).toLowerCase());
  const info = isVideo ? await probeVideo(file, caches.probe, stat) : {};
  return projectionOf({ name: path.basename(file), ...info, vr: isVideo ? stereoVr(caches.stereo, file, stat) : undefined }).vr;
}

async function listDir(dir) {
  try {
    return await fs.readdir(dir);
  } catch {
    return [];
  }
}

async function existingSuffixes(anchor) {
  const names = await listDir(path.dirname(anchor));
  const suffixes = new Set();
  const base = getBaseName(anchor).toLowerCase();
  for (const name of names) {
    if (name.startsWith('._')) continue;
    const axis = parseAxisSuffix(name);
    if (!axis) continue;
    if (getBaseName(name).toLowerCase() === base) suffixes.add(axis.suffix);
  }
  return suffixes;
}

async function fileExists(p) {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

/** The generator a companion file names in its metadata; `v0-random` for files from before it was recorded. */
async function generatorTag(file) {
  try {
    const tag = JSON.parse((await fs.readFile(file, 'utf8')).replace(/^\uFEFF/, ''))?.metadata?.generator;
    if (typeof tag === 'string' && /^[\w.-]+$/.test(tag)) return tag;
  } catch { /* unreadable: treat as the old generator */ }
  return 'v0-random';
}

/** Hand-made axes that keep a job from generating anything.
 *  `own`: its scene's (every stroke script of the same video in the script's
 *  folder, variants included, plus companions of the output anchor).
 *  `elsewhere`: other stroke scripts with hand-made axes in the script's or
 *  the anchor's folder (sibling videos), script path → axes. */
async function jobHandmadeAxes(anchor, script, parsed) {
  const scriptDir = path.dirname(script);
  const own = path.basename(script);
  const ownAxes = new Set();
  const elsewhere = new Map();
  for (const dir of new Set([scriptDir, path.dirname(anchor)])) {
    const names = await listDir(dir);
    const here = dir === scriptDir;
    const scene = here ? groupScenes(names).find((s) => s.scripts.includes(own)) : null;
    const sceneScripts = new Set(here ? (scene ? scene.scripts : [own]) : []);
    const parse = (name) => (here && name === own ? parsed : readStrokeScript(path.join(dir, name)));
    const scripts = [...new Set([...sceneScripts, ...folderStrokeScripts(names)])];
    const { byScript } = await sceneHandmadeAxes(scripts, names, parse);
    for (const [name, axes] of byScript) {
      if (sceneScripts.has(name)) axes.forEach((s) => ownAxes.add(s));
      else elsewhere.set(path.join(dir, name), axes);
    }
  }
  if (path.dirname(anchor) !== scriptDir || getBaseName(anchor) !== getBaseName(script)) {
    for (const s of handmadeAxisSuffixes({ primaryPath: anchor, allFiles: await listDir(path.dirname(anchor)) })) {
      ownAxes.add(s);
    }
  }
  const all = new Set(ownAxes);
  for (const axes of elsewhere.values()) axes.forEach((s) => all.add(s));
  return { own: [...ownAxes].sort(), elsewhere, all: [...all].sort() };
}

async function main() {
  if (paths.length === 0 || flags.has('--help') || flags.has('-h')) {
    console.log(`Usage: node scripts/upgrade-multi-axis.mjs [--dry-run] [--seed=N] [--only=twist,roll,…] [--replace] [--surge] [--sway] [--valve] <video|funscript|dir>...

Twist and roll are guided by the stroke and styled like the hand-made axis of your liked
scenes (VR and flat settings of their own), with randomness seeded per scene. Pitch is
quieter late; a 0% rest sits at centre and cannot last more than 10 seconds.
Does not overwrite the stroke file. Skips an axis whose companion already exists, unless
--replace: an existing .generated. companion is written again, the old file kept as
<file>.<its generator>. --only limits the axes written.
Skips a folder where any script has hand-made axes (companion files or axes embedded in the script).`);
    process.exit(paths.length === 0 ? 1 : 0);
  }
  if (!Number.isFinite(seed)) {
    console.error(`bad --seed: ${seedArg}`);
    process.exit(1);
  }
  const known = new Set(resolveUpgradeAxes(enabled).map((a) => a.suffix));
  const unknown = (only || []).filter((s) => !known.has(s));
  if (only && (only.length === 0 || unknown.length > 0)) {
    console.error(`bad --only: ${unknown.join(', ') || '(empty)'} (axes: ${[...known].join(', ')})`);
    process.exit(1);
  }

  const { jobs, unmatchedVideos, orphanScripts } = await collect(paths);
  for (const video of unmatchedVideos) {
    console.log(`no stroke script: ${video}`);
  }
  for (const script of orphanScripts) {
    console.log(`no video, skipped: ${script}`);
  }
  if (jobs.length === 0) {
    console.error('no stroke funscripts found');
    process.exit(1);
  }

  const noise = createNoise1D(seed);
  const userData = defaultUserDataDir();
  const caches = { probe: readProbeCache(userData), stereo: readStereoCache(userData) };
  const planned = [];
  let handmadeSkips = 0;
  for (const job of jobs) {
    let raw;
    try {
      raw = await fs.readFile(job.script, 'utf8');
    } catch (err) {
      console.error(`read failed: ${job.script} (${err.message})`);
      continue;
    }
    let parsed;
    try {
      parsed = JSON.parse(raw.replace(/^\uFEFF/, ''));
    } catch (err) {
      console.error(`not JSON: ${job.script} (${err.message})`);
      continue;
    }
    const anchor = outputAnchor(job.video, job.script);
    const existing = await existingSuffixes(anchor);
    // --replace: an axis whose only companion is our own generated file is planned again.
    const replaced = new Set();
    if (replace) {
      for (const suffix of existing) {
        if (only && !only.includes(suffix)) continue;
        // A hand-made companion of this axis means hand-made axes: the job is skipped below.
        if (await fileExists(companionPathForScript(anchor, suffix))) {
          existing.delete(suffix);
          replaced.add(suffix);
        }
      }
    }
    const handmade = await jobHandmadeAxes(anchor, job.script, parsed);
    const vr = await sceneVr(job.video || job.script, caches);
    const plan = planUpgrade({
      primaryPath: anchor,
      strokeActions: parsed.actions,
      existingSuffixes: existing,
      handmadeAxes: handmade.all,
      enabledTcodes: enabled,
      noise,
      vr,
      seed: sceneSeed(anchor),
      only,
    });
    if (plan.error === 'handmade-axes') {
      if (handmade.own.length > 0) {
        console.log(`skip ${job.script}: has hand-made axes (${handmade.own.join(', ')}), nothing generated`);
      } else {
        const where = [...handmade.elsewhere].map(([p, axes]) => `${path.basename(p)}: ${axes.join(', ')}`).join('; ');
        console.log(`skip ${job.script}: hand-made axes in its folder (${where}), nothing generated`);
      }
      handmadeSkips++;
      continue;
    }
    if (plan.error) {
      console.log(`skip ${job.script}: ${plan.error}`);
      continue;
    }
    planned.push({ ...job, anchor, plan, replaced });
  }

  const writes = planned.flatMap((p) => p.plan.writes);
  const skipped = planned.flatMap((p) => p.plan.skipped);
  if (writes.length === 0 && skipped.length === 0) {
    if (handmadeSkips > 0) {
      console.log('\nnothing to write');
      return;
    }
    console.error('nothing to write');
    process.exit(1);
  }

  console.log('Twist and roll are guided by the stroke and styled like your liked scenes. Pitch falls; noise is 10–60% of its motion.');
  console.log('Stroke files are not modified.\n');
  const replacing = new Set(planned.flatMap((p) => p.plan.writes.filter((w) => p.replaced.has(w.suffix)).map((w) => w.path)));
  for (const w of writes) {
    console.log(`${replacing.has(w.path) ? 'replace' : 'write'}  ${w.path}  (${MODE_LABEL[w.mode] || w.mode}${w.generator ? `: ${w.generator}` : ''})`);
  }
  for (const s of skipped) {
    console.log(`skip   ${s.path}  (${s.reason === 'exists' ? 'already exists' : 'stroke too short'})`);
  }
  if (dryRun) {
    console.log(`\n${writes.length} to write (${replacing.size} replacing), ${skipped.length} skipped (dry run)`);
    return;
  }
  if (writes.length === 0) {
    console.log('\nnothing to write');
    return;
  }

  const byPath = new Map();
  for (const p of planned) {
    for (const file of serialiseUpgradeWrites(p.plan.writes)) {
      const target = companionPathForScript(p.anchor, file.suffix);
      const write = p.plan.writes.find((w) => w.suffix === file.suffix);
      byPath.set(target, {
        ...file.content,
        inverted: false,
        range: 100,
        metadata: {
          creator: 'funsync upgrade-multi-axis',
          type: 'generated',
          mode: write?.mode,
          ...(write?.generator ? { generator: write.generator } : {}),
          seed: write?.generator ? sceneSeed(p.anchor) : seed,
        },
        replaces: p.replaced.has(file.suffix),
      });
    }
  }

  let wrote = 0;
  let backedUp = 0;
  for (const [target, { replaces, ...content }] of byPath) {
    if (await fileExists(target)) {
      if (!replaces) {
        console.log(`skip   ${target}  (appeared before write)`);
        continue;
      }
      // Keep the previous generated file, named after what made it (the first backup of a tag stays).
      const backup = `${target}.${await generatorTag(target)}`;
      if (!(await fileExists(backup))) {
        await fs.rename(target, backup);
        backedUp++;
      }
    }
    await fs.mkdir(path.dirname(target), { recursive: true });
    const tmp = `${target}.tmp`;
    await fs.writeFile(tmp, JSON.stringify(content, null, 2) + '\n', 'utf8');
    await fs.rename(tmp, target);
    wrote++;
  }
  console.log(`\nwrote ${wrote}${replace ? ` (${backedUp} previous files kept as <file>.<generator>)` : ''}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
