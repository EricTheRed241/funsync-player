#!/usr/bin/env node
// Hand-made axis scripts always win. For every scene with hand-made axes,
// write embedded axes out as companion files and move its generated
// companions (`<stem>.generated.<axis>.funscript`) into an archive.
// See prune-generated-lib.mjs for the rules.
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
const { planPrune, applyPrune } = await import('./prune-generated-lib.mjs');

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

  const scenes = await planPrune(args.roots, archiveDir);
  const label = args.dryRun ? ' (dry run, nothing changed)' : '';
  for (const scene of scenes) {
    const embedded = scene.embedded.length ? `; embedded: ${scene.embedded.join(', ')}` : '';
    console.log(`scene    ${scene.video || scene.scripts[0]}`);
    console.log(`         hand-made: ${scene.handmade.join(', ')}${embedded}`);
    for (const e of scene.extract) console.log(`extract  ${e.path}`);
    for (const a of scene.archive) console.log(`archive  ${a.from}\n      →  ${a.to}`);
    if (scene.extract.length === 0 && scene.archive.length === 0) console.log('         nothing to do');
  }

  const toExtract = scenes.reduce((n, s) => n + s.extract.length, 0);
  const toArchive = scenes.reduce((n, s) => n + s.archive.length, 0);
  const changing = scenes.filter((s) => s.extract.length > 0 || s.archive.length > 0).length;
  const sceneLine = `${scenes.length} scenes with hand-made axes (${changing} with changes)`;
  if (args.dryRun) {
    console.log(`\n${sceneLine}, ${toExtract} embedded axes to extract, ${toArchive} generated files to archive${label}`);
    return;
  }

  const { extracted, archived, errors } = await applyPrune(scenes, archiveDir);
  for (const e of errors) console.error(`error    ${e}`);
  console.log(`\n${sceneLine}, ${extracted.length} embedded axes extracted, ${archived.length} generated files archived to ${archiveDir}`);
  if (archived.length > 0) console.log(`manifest ${path.join(archiveDir, 'MANIFEST.txt')}`);
  if (errors.length > 0) process.exit(1);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
