#!/usr/bin/env node
// Hand-made axis scripts always win, per folder. For every folder with
// hand-made axes, write embedded axes out as companion files, copy the
// hand-made axes to sibling videos whose timing provably matches, and move
// every generated companion (`<stem>.generated.<axis>.funscript`) in the
// folder into an archive. See prune-generated-lib.mjs for the rules.
//
//   npm run prune:generated -- --dry-run --archive /path/to/archive "/lib/Multi-Axis"
//   npm run prune:generated -- --archive /path/to/archive "/lib/Multi-Axis" "/lib/Other"

import { spawnSync } from 'node:child_process';
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
const { planPrune, applyPrune, folderChanges } = await import('./prune-generated-lib.mjs');

const USAGE = 'Usage: npm run prune:generated -- [--dry-run] --archive <dir> <library folder>...';

function parseArgs(argv) {
  const out = { dryRun: false, archive: null, roots: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--dry-run') out.dryRun = true;
    else if (a === '--archive') out.archive = argv[++i] || null;
    else if (a.startsWith('--archive=')) out.archive = a.slice('--archive='.length);
    else if (a === '--help' || a === '-h') out.help = true;
    else if (a.startsWith('--')) throw new Error(`unknown option: ${a}`);
    else out.roots.push(a);
  }
  return out;
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`${err.message}\n${USAGE}`);
    process.exit(1);
  }
  if (args.help || !args.archive || args.roots.length === 0) {
    console.log(USAGE);
    process.exit(args.help ? 0 : 1);
  }
  const archiveDir = path.resolve(args.archive);

  const folders = await planPrune(args.roots, archiveDir);
  const label = args.dryRun ? ' (dry run, nothing changed)' : '';
  const MATCH_LABEL = { identical: 'identical actions', near: 'same timing within ±50 ms' };
  for (const folder of folders) {
    const refused = folder.siblings.filter((s) => !s.source);
    if (!folderChanges(folder) && refused.length === 0) continue;
    console.log(`folder   ${folder.dir}`);
    for (const scene of folder.scenes) {
      const embedded = scene.embedded.length ? `; embedded: ${scene.embedded.join(', ')}` : '';
      console.log(`scene    ${scene.video || scene.scripts[0]}`);
      console.log(`         hand-made: ${scene.handmade.join(', ')}${embedded}`);
    }
    for (const e of folder.extract) console.log(`extract  ${e.path}`);
    for (const s of folder.siblings) {
      console.log(`sibling  ${s.video}`);
      if (!s.source) {
        console.log('         sibling not timing-compatible, nothing copied');
        continue;
      }
      console.log(`         from ${path.basename(s.source)} (${MATCH_LABEL[s.match]})`);
      for (const c of s.copy) console.log(`copy     ${c.path}`);
    }
    for (const a of folder.archive) console.log(`archive  ${a.from}\n      →  ${a.to}`);
  }

  const sum = (f) => folders.reduce((n, x) => n + f(x), 0);
  const scenes = sum((f) => f.scenes.length);
  const toExtract = sum((f) => f.extract.length);
  const toArchive = sum((f) => f.archive.length);
  const toCopy = sum((f) => f.siblings.reduce((n, s) => n + s.copy.length, 0));
  const copyTo = sum((f) => f.siblings.filter((s) => s.copy.length > 0).length);
  const refused = sum((f) => f.siblings.filter((s) => !s.source).length);
  const changing = folders.filter(folderChanges).length;
  const head = `${folders.length} folders with hand-made axes (${changing} with changes), ${scenes} scenes with hand-made axes`;
  const refusedLine = `${refused} siblings not timing-compatible`;
  if (args.dryRun) {
    console.log(`\n${head}, ${toExtract} embedded axes to extract, ${toCopy} hand-made axes to copy to ${copyTo} siblings, ${refusedLine}, ${toArchive} generated files to archive${label}`);
    return;
  }

  const { extracted, copied, archived, errors } = await applyPrune(folders, archiveDir);
  for (const e of errors) console.error(`error    ${e}`);
  console.log(`\n${head}, ${extracted.length} embedded axes extracted, ${copied.length} hand-made axes copied to siblings, ${refusedLine}, ${archived.length} generated files archived to ${archiveDir}`);
  if (archived.length > 0) console.log(`manifest ${path.join(archiveDir, 'MANIFEST.txt')}`);
  if (errors.length > 0) process.exit(1);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
