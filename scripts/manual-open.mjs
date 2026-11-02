#!/usr/bin/env node
// npm run manual:open — open the download list's pages in the browser, a few
// posts at a time: each post's forum page next to its pixeldrain pages, so a
// post can be looked at before its files are downloaded by hand.
//
// Reads ~/Downloads/download-for-fs.txt (`npm run eroscripts:scout --
// --manual-list`) and makes it again before every batch (same posts), so
// scenes the scout fetched meanwhile drop out; files already saved in
// ~/Downloads/for-fs or ~/Downloads are skipped. Logic: scripts/manual-open-lib.mjs.
//
// Exit codes: 0 done, 1 error.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import readline from 'node:readline/promises';
import { fileURLToPath } from 'node:url';
import { batchUrls, listIds, parseManualList, postsToOpen } from './manual-open-lib.mjs';

const SCOUT = path.join(path.dirname(fileURLToPath(import.meta.url)), 'eroscripts-scout.mjs');

const DOWNLOADS = path.join(os.homedir(), 'Downloads');
const USAGE = `Usage: npm run manual:open -- [options]
Opens each post of ~/Downloads/download-for-fs.txt with its download pages, N posts at a time;
press Enter for the next batch, q to stop. Files already saved are skipped.
  --batch N          posts per batch (default 5)
  --from N           start at post number N of the list
  --list <file>      the download list (default ~/Downloads/download-for-fs.txt)
  --dir <dir>        where files are saved (default ~/Downloads/for-fs; ~/Downloads is checked too)
  --dry-run          print the batches instead of opening them
  --no-refresh       open the list as it is (don't make it again first)`;

function parseArgs(argv) {
  const opts = { batch: 5, from: 1, list: path.join(DOWNLOADS, 'download-for-fs.txt'), dir: path.join(DOWNLOADS, 'for-fs'), dryRun: false, refresh: true };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const value = () => { if (i + 1 >= argv.length) throw new Error(`${a} needs a value`); return argv[++i]; };
    switch (a) {
      case '--batch': opts.batch = Number(value()); break;
      case '--from': opts.from = Number(value()); break;
      case '--list': opts.list = path.resolve(value()); break;
      case '--dir': opts.dir = path.resolve(value()); break;
      case '--dry-run': opts.dryRun = true; break;
      case '--no-refresh': opts.refresh = false; break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  if (!(opts.batch >= 1) || !(opts.from >= 1)) throw new Error('--batch and --from must be 1 or more');
  return opts;
}

/** File names in the download folders, read fresh so files saved meanwhile count. */
function savedNames(dirs) {
  const out = [];
  for (const d of dirs) { try { out.push(...fs.readdirSync(d)); } catch { /* no such folder */ } }
  return out;
}

/**
 * The list's posts, made again first (eroscripts:scout --manual-list for the
 * same post ids) unless refreshing is off or the list names no ids. The scout
 * writes the default list only, so a --list elsewhere is read as it is.
 */
function readList(opts) {
  const defaultList = opts.list === path.join(DOWNLOADS, 'download-for-fs.txt');
  const ids = listIds(fs.readFileSync(opts.list, 'utf8'));
  if (opts.refresh && defaultList && ids.length) {
    const r = spawnSync(process.execPath, [SCOUT, '--manual-list', '--ids', ids.join(','), '--manual-dir', opts.dir], { encoding: 'utf8' });
    if (r.status !== 0) console.warn(`(list not refreshed: ${(r.stderr || r.stdout || '').trim().split('\n').pop()})`);
  }
  return parseManualList(fs.readFileSync(opts.list, 'utf8'));
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  if (!fs.existsSync(opts.list)) { console.error(`No download list at ${opts.list}; ask omp for one (eroscripts:scout --manual-list).`); return 1; }
  // --from counts in the list as it stands now; later batches go by post id.
  const skipped = new Set(parseManualList(fs.readFileSync(opts.list, 'utf8')).filter((p) => p.n < opts.from).map((p) => p.id));
  const rl = opts.dryRun ? null : readline.createInterface({ input: process.stdin, output: process.stdout });
  const opened = new Set();
  try {
    for (;;) {
      const left = postsToOpen(readList(opts), savedNames([opts.dir, DOWNLOADS])).filter((p) => !opened.has(p.id) && !skipped.has(p.id));
      if (!left.length) { console.log('Nothing left to open: every file on the list is saved, fetched, or opened this run.'); return 0; }
      const batch = left.slice(0, opts.batch);
      for (const p of batch) {
        opened.add(p.id);
        console.log(`${p.n}. ${p.title}\n   ${p.url}${p.files.map((f) => `\n   - ${f.name}`).join('')}`);
      }
      const urls = batchUrls(batch);
      if (!opts.dryRun) spawnSync('open', urls, { stdio: 'inherit' });
      const rest = left.length - batch.length;
      if (!rest) { console.log(`\nOpened ${urls.length} tabs; that was the last batch. Save files into ${opts.dir}.`); return 0; }
      if (opts.dryRun) continue;
      const answer = await rl.question(`\nOpened ${urls.length} tabs. Save files into ${opts.dir}. Enter = next ${Math.min(opts.batch, rest)} of ${rest} posts, q = stop: `);
      if (/^q/i.test(answer.trim())) return 0;
    }
  } finally {
    rl?.close();
  }
}

main().then((code) => process.exit(code), (err) => {
  console.error(`manual:open: ${err.message}`);
  process.exit(1);
});
