#!/usr/bin/env node
// One-shot: stroke funscript (+ optional video) → twist / roll / pitch companions.
//
// Each axis is scaled to 0 / 25 / 50 / 75 / 100% around centre. Twist's
// odds rise later; roll and pitch fall. Smooth noise then changes that
// scaled motion by 10–60%, so the build stays visible.
// A 0% rest sits at centre and cannot last more than 10 seconds.
//
// The stroke file is never overwritten. An existing companion for an axis is
// skipped. Companions are named `<stem>.generated.<axis>.funscript` and
// written next to the video when one is known, otherwise next to the funscript.
//
//   node scripts/upgrade-multi-axis.mjs ./clip.mp4 ./clip.funscript
//   node scripts/upgrade-multi-axis.mjs ./library-folder
//   node scripts/upgrade-multi-axis.mjs --dry-run ./clip.funscript
//   node scripts/upgrade-multi-axis.mjs --surge --sway ./clip.mp4
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
const {
  planUpgrade,
  serialiseUpgradeWrites,
  companionPathForScript,
} = await import('../renderer/js/upgrade-multi-axis.js');

const VIDEO_EXTS = new Set(['.mp4', '.m4v', '.mkv', '.webm', '.avi', '.mov']);
const MODE_LABEL = {
  link: 'stroke, quieter early',
  random: 'smooth noise, quieter late',
  pattern: 'slow sine, quieter late',
};

const args = process.argv.slice(2);
const dryRun = args.includes('--dry-run');
const flags = new Set(args.filter((a) => a.startsWith('--')));
const paths = args.filter((a) => !a.startsWith('--'));
const seedArg = args.find((a) => a.startsWith('--seed='));
const seed = seedArg ? Number(seedArg.slice('--seed='.length)) : 1337;

const enabled = new Set();
if (flags.has('--surge')) enabled.add('L1');
if (flags.has('--sway')) enabled.add('L2');
if (flags.has('--valve')) enabled.add('A0');

function normStem(name) {
  return name.toLowerCase().replace(/[_.\-]/g, ' ').replace(/\s+/g, ' ').trim();
}

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
  for (const root of roots) {
    let st;
    try {
      st = await fs.stat(root);
    } catch {
      console.error(`missing: ${root}`);
      continue;
    }
    if (st.isDirectory()) await walk(root, videos, scripts);
    else classify(path.resolve(root), videos, scripts);
  }
  const known = new Set(scripts.map((s) => path.resolve(s)));
  for (const video of videos) {
    const sib = await siblingStroke(video);
    if (sib && !known.has(path.resolve(sib))) {
      scripts.push(sib);
      known.add(path.resolve(sib));
    }
  }
  return pair(videos, scripts);
}

function pair(videos, scripts) {
  if (videos.length === 1 && scripts.length === 1) {
    return { jobs: [{ video: videos[0], script: scripts[0] }], unmatchedVideos: [] };
  }
  const byStem = new Map();
  for (const script of scripts) {
    const stem = normStem(path.basename(script, '.funscript'));
    if (!byStem.has(stem)) byStem.set(stem, []);
    byStem.get(stem).push(script);
  }
  const used = new Set();
  const jobs = [];
  const unmatchedVideos = [];
  for (const video of videos) {
    const stem = normStem(path.basename(video, path.extname(video)));
    const hit = (byStem.get(stem) || []).find((s) => !used.has(s));
    if (!hit) {
      unmatchedVideos.push(video);
      continue;
    }
    used.add(hit);
    jobs.push({ video, script: hit });
  }
  for (const script of scripts) {
    if (!used.has(script)) jobs.push({ video: null, script });
  }
  return { jobs, unmatchedVideos };
}

function outputAnchor(video, script) {
  if (!video) return script;
  const stem = path.basename(video, path.extname(video));
  return path.join(path.dirname(video), `${stem}.funscript`);
}

async function existingSuffixes(anchor) {
  const dir = path.dirname(anchor);
  let names = [];
  try {
    names = await fs.readdir(dir);
  } catch {
    return new Set();
  }
  const suffixes = new Set();
  const base = getBaseName(anchor).toLowerCase();
  for (const name of names) {
    const axis = parseAxisSuffix(name);
    if (!axis) continue;
    if (getBaseName(name).toLowerCase() === base) suffixes.add(axis.suffix);
  }
  return suffixes;
}

async function main() {
  if (paths.length === 0 || flags.has('--help') || flags.has('-h')) {
    console.log(`Usage: node scripts/upgrade-multi-axis.mjs [--dry-run] [--seed=N] [--surge] [--sway] [--valve] <video|funscript|dir>...

Twist is a fraction of the stroke, quieter early. Roll and pitch are quieter late.
A 0% rest sits at centre and cannot last more than 10 seconds.
Does not overwrite the stroke file. Skips an axis whose companion already exists.`);
    process.exit(paths.length === 0 ? 1 : 0);
  }
  if (!Number.isFinite(seed)) {
    console.error(`bad --seed: ${seedArg}`);
    process.exit(1);
  }

  const { jobs, unmatchedVideos } = await collect(paths);
  for (const video of unmatchedVideos) {
    console.log(`no stroke script: ${video}`);
  }
  if (jobs.length === 0) {
    console.error('no stroke funscripts found');
    process.exit(1);
  }

  const noise = createNoise1D(seed);
  const planned = [];
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
    const plan = planUpgrade({
      primaryPath: anchor,
      strokeActions: parsed.actions,
      existingSuffixes: existing,
      enabledTcodes: enabled,
      noise,
    });
    if (plan.error) {
      console.log(`skip ${job.script}: ${plan.error}`);
      continue;
    }
    planned.push({ ...job, anchor, plan });
  }

  const writes = planned.flatMap((p) => p.plan.writes);
  const skipped = planned.flatMap((p) => p.plan.skipped);
  if (writes.length === 0 && skipped.length === 0) {
    console.error('nothing to write');
    process.exit(1);
  }

  console.log('Twist rises on average, roll and pitch fall. Noise is 10–60% of that motion.');
  console.log('Stroke files are not modified.\n');
  for (const w of writes) {
    console.log(`write  ${w.path}  (${MODE_LABEL[w.mode] || w.mode})`);
  }
  for (const s of skipped) {
    console.log(`skip   ${s.path}  (already exists)`);
  }
  if (dryRun) {
    console.log(`\n${writes.length} to write, ${skipped.length} skipped (dry run)`);
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
      const mode = p.plan.writes.find((w) => w.suffix === file.suffix)?.mode;
      byPath.set(target, {
        ...file.content,
        inverted: false,
        range: 100,
        metadata: {
          creator: 'funsync upgrade-multi-axis',
          type: 'generated',
          mode,
          seed,
        },
      });
    }
  }

  let wrote = 0;
  for (const [target, content] of byPath) {
    try {
      await fs.access(target);
      console.log(`skip   ${target}  (appeared before write)`);
      continue;
    } catch { /* absent */ }
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, JSON.stringify(content, null, 2) + '\n', 'utf8');
    wrote++;
  }
  console.log(`\nwrote ${wrote}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
