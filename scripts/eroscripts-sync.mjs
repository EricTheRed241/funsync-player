#!/usr/bin/env node
// npm run eroscripts:sync — link library videos to their EroScripts posts.
//
// Reads the desktop app's config (library sources, EroScripts session) but
// never writes it: the app owns config.json and may be running. Links go to
// `<userData>/eroscripts-links.json`, which the app merges into
// `library.scriptTags` on its next library scan; the EroScripts preview
// batch then covers the linked videos. See scripts/eroscripts-sync-lib.mjs.
//
//   npm run eroscripts:sync -- [--dry-run] [--refresh] [--limit N]
//       [--stale-days N] [--install-axes] [--archive <dir>] [--report <file>]
//       [--user-data <dir>]
//   npm run eroscripts:sync -- --install-schedule 3d
//   npm run eroscripts:sync -- --uninstall-schedule
//
// Exit codes: 0 done, 1 error, 2 rate limited (progress saved), 3 not logged
// in / session expired.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

// renderer/*.js is ESM inside a CommonJS package. Re-exec once so Node does
// not print MODULE_TYPELESS_PACKAGE_JSON on every run.
if (!process.execArgv.includes('--disable-warning=MODULE_TYPELESS_PACKAGE_JSON')) {
  const relaunch = spawnSync(process.execPath, [
    '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON',
    ...process.argv.slice(1),
  ], { stdio: 'inherit' });
  process.exit(relaunch.status ?? 1);
}

const lib = await import('./eroscripts-sync-lib.mjs');
const require = createRequire(import.meta.url);

const USAGE = `Usage: npm run eroscripts:sync -- [options]
  --dry-run              search and report; write no links file, install nothing
  --refresh              re-check every video, confirmed ones included
  --limit N              check at most N videos this run
  --stale-days N         re-check unconfirmed results older than N days (default ${lib.DEFAULT_STALE_DAYS})
  --install-axes         download missing hand-made axes into scene folders
  --archive <dir>        where --install-axes archives generated companions
                         (default <library parent>/Archive/_pruned-<date>)
  --report <file>        report path (.md or .json; default <userData>/eroscripts-sync-report.md)
  --user-data <dir>      app data folder (default: the desktop app's)
  --install-schedule 3d  run every 3 days via launchd (macOS); also 12h, 90m
  --uninstall-schedule   remove the launchd schedule`;

function parseArgs(argv) {
  const opts = { dryRun: false, refresh: false, limit: Infinity, staleDays: lib.DEFAULT_STALE_DAYS, installAxes: false };
  const value = (i, flag) => {
    if (i + 1 >= argv.length) throw new Error(`${flag} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--dry-run': opts.dryRun = true; break;
      case '--refresh': opts.refresh = true; break;
      case '--install-axes': opts.installAxes = true; break;
      case '--uninstall-schedule': opts.uninstallSchedule = true; break;
      case '--limit': opts.limit = Number(value(i, a)); i++; break;
      case '--stale-days': opts.staleDays = Number(value(i, a)); i++; break;
      case '--archive': opts.archive = path.resolve(value(i, a)); i++; break;
      case '--report': opts.report = path.resolve(value(i, a)); i++; break;
      case '--user-data': opts.userData = path.resolve(value(i, a)); i++; break;
      case '--install-schedule': opts.installSchedule = value(i, a); i++; break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  if (!(opts.limit > 0)) throw new Error('--limit must be a positive number');
  if (!(opts.staleDays >= 0)) throw new Error('--stale-days must be 0 or more');
  return opts;
}

const plistPath = () => path.join(os.homedir(), 'Library', 'LaunchAgents', `${lib.LAUNCHD_LABEL}.plist`);
const launchctl = (...args) => spawnSync('launchctl', args, { encoding: 'utf8' });

function installSchedule(interval, userData) {
  if (process.platform !== 'darwin') throw new Error('--install-schedule uses launchd and works on macOS only.');
  const seconds = lib.parseInterval(interval);
  if (!seconds) throw new Error(`--install-schedule: "${interval}" is not an interval like 3d, 12h or 90m.`);
  const scriptPath = fileURLToPath(import.meta.url);
  const logDir = path.join(userData, 'logs');
  fs.mkdirSync(logDir, { recursive: true });
  const plist = lib.launchdPlist({
    nodePath: process.execPath,
    scriptPath,
    workingDirectory: path.dirname(path.dirname(scriptPath)),
    intervalSeconds: seconds,
    logPath: path.join(logDir, 'eroscripts-sync.log'),
  });
  const target = plistPath();
  fs.mkdirSync(path.dirname(target), { recursive: true });
  const domain = `gui/${process.getuid()}`;
  launchctl('bootout', `${domain}/${lib.LAUNCHD_LABEL}`); // replace an older schedule
  fs.writeFileSync(target, plist);
  const r = launchctl('bootstrap', domain, target);
  if (r.status !== 0) throw new Error(`launchctl bootstrap failed: ${(r.stderr || r.stdout).trim()}`);
  console.log(`Scheduled every ${interval}: ${target}\nLogs: ${path.join(logDir, 'eroscripts-sync.log')}`);
}

function uninstallSchedule() {
  if (process.platform !== 'darwin') throw new Error('--uninstall-schedule uses launchd and works on macOS only.');
  launchctl('bootout', `gui/${process.getuid()}/${lib.LAUNCHD_LABEL}`);
  const target = plistPath();
  if (fs.existsSync(target)) { fs.unlinkSync(target); console.log(`Removed ${target}`); }
  else console.log('No schedule installed.');
}

function readConfig(userData) {
  const file = path.join(userData, 'config.json');
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (err) {
    throw new Error(`Cannot read the app config at ${file}: ${err.message}`);
  }
}

const today = () => new Date().toISOString().slice(0, 10);

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || lib.defaultUserDataDir();
  if (opts.installSchedule) { installSchedule(opts.installSchedule, userData); return 0; }
  if (opts.uninstallSchedule) { uninstallSchedule(); return 0; }

  console.log(`[${new Date().toISOString()}] EroScripts sync${opts.dryRun ? ' (dry run)' : ''}`);
  const config = readConfig(userData);
  const settings = config.settings || {};
  const roots = (settings.library?.sources || []).filter((s) => s.enabled !== false && s.path).map((s) => s.path);
  if (roots.length === 0) throw new Error('No enabled library sources in the app config.');
  const session = settings.eroscripts?.session || {};
  if (!session.cookie) {
    console.error('Not logged in to EroScripts. Log in from the app (EroScripts panel), then re-run.');
    return 3;
  }

  // The API module logs through electron-log; keep the CLI's output clean.
  const log = require('../electron/logger.js');
  log.transports.file.level = false;
  log.transports.console.level = false;
  const { EroScriptsAPI } = require('../electron/eroscripts-api.js');
  const { scanLibrary } = require('../electron/library-scan.js');
  const { readLinksFile, writeLinksFile, linksFilePath } = require('../electron/eroscripts-links.js');

  const api = new EroScriptsAPI();
  api.restoreSession(session.cookie, session.username);
  if (!(await api.validateSession()).valid) {
    console.error(new lib.SessionExpiredError().message);
    return 3;
  }

  const available = roots.filter((r) => fs.existsSync(r));
  for (const r of roots) if (!available.includes(r)) console.warn(`Source unavailable, skipped: ${r}`);
  const scan = await scanLibrary(available);
  const videos = scan.videos.map((v) => ({ path: v.path, funscriptPath: v.funscriptPath, creator: lib.creatorOf(v.path, available) }));
  console.log(`Library: ${videos.length} videos (${videos.filter((v) => v.funscriptPath).length} with a script) in ${available.length} source(s)`);

  const state = readLinksFile(userData);
  if (!state) throw new Error(`${linksFilePath(userData)} is unreadable or from another version; move it aside and re-run.`);
  state.cache ||= {};

  // Posts the app already links (the user downloaded the script from them).
  const knownTopics = {};
  for (const [p, rec] of Object.entries(settings.library?.scriptTags || {})) {
    if (rec?.topicId != null && rec.source !== 'eroscripts-sync') knownTopics[p] = [rec.topicId];
  }

  const forum = lib.createForum(api, { cache: state.cache, staleDays: opts.staleDays });
  const persist = opts.dryRun ? () => {} : (s) => { s.updatedAt = new Date().toISOString(); writeLinksFile(userData, s); };
  const marks = { confirmed: '✓', probable: '~', review: '?', none: '·', error: '!' };
  let exitCode = 0;
  let summary;
  try {
    summary = await lib.runSync({
      videos, forum, state, persist, knownTopics,
      refresh: opts.refresh, limit: opts.limit, staleDays: opts.staleDays,
      onVideo: (video, outcome, detail) => {
        const what = detail?.topicId ? ` → ${detail.topicId} ${detail.title || ''} (${detail.basis})` : outcome === 'error' ? ` ${detail}` : '';
        console.log(`  ${marks[outcome] || ' '} ${path.basename(video.path)}${what}`);
      },
    });
  } catch (err) {
    if (err instanceof lib.SessionExpiredError) { console.error(err.message); return 3; }
    throw err;
  }
  if (summary.stopped === 'rate-limited') { console.error(new lib.RateLimitedError().message); exitCode = 2; }

  // Hand-made axes the linked posts offer.
  const axes = [];
  for (const video of videos) {
    const link = state.links[video.path];
    if (!link || !video.funscriptPath || !fs.existsSync(video.funscriptPath)) continue;
    const local = lib.loadLocalScene(video);
    const plan = lib.availableAxes(link, local);
    if (!plan.missing.length && !plan.ambiguous) continue;
    const entry = { video: video.path, link, plan };
    if (opts.installAxes && plan.missing.some((m) => m.installable) && exitCode === 0) {
      const root = available.find((r) => video.path.startsWith(`${r}/`));
      const relativeTo = path.dirname(root);
      const archiveDir = opts.archive || path.join(relativeTo, 'Archive', `_pruned-${today()}`);
      try {
        entry.result = await lib.installAxes({ forum, local, plan, archiveDir, relativeTo, dryRun: opts.dryRun });
      } catch (err) {
        if (err instanceof lib.RateLimitedError) { console.error(err.message); exitCode = 2; }
        else if (err instanceof lib.SessionExpiredError) { console.error(err.message); exitCode = 3; }
        else summary.errors.push({ video: video.path, error: `install axes: ${err.message}` });
      }
    }
    axes.push(entry);
  }

  // Previews: every linked video can get its post's image in the app.
  const custom = settings.library?.customThumbnails || {};
  const linked = Object.entries(state.links).filter(([, l]) => l.confidence === 'confirmed' || l.confidence === 'probable');
  const previews = {
    available: linked.length,
    withUrl: linked.filter(([, l]) => l.previewUrl).length,
    withoutThumb: linked.filter(([p]) => !custom[p]).length,
  };

  if (!opts.dryRun) persist(state);

  const reportPath = opts.report || path.join(userData, 'eroscripts-sync-report.md');
  const report = reportPath.endsWith('.json')
    ? JSON.stringify({ generatedAt: new Date().toISOString(), dryRun: opts.dryRun, summary, previews, links: state.links, review: state.review, axes }, null, 2)
    : lib.renderReport({ state, summary, axes, previews, roots: available, dryRun: opts.dryRun });
  fs.writeFileSync(reportPath, report);

  const confirmed = linked.filter(([, l]) => l.confidence === 'confirmed').length;
  const axisScenes = axes.filter((a) => a.plan.missing.length);
  const installed = axes.reduce((n, a) => n + (a.result?.installed.length || 0), 0);
  console.log([
    '',
    `Checked ${summary.checked} videos (${summary.skipped} up to date, ${summary.noScript} without a script${summary.errors.length ? `, ${summary.errors.length} errors` : ''}); ${forum.pacer.count} forum requests`,
    `This run: ${summary.outcomes.confirmed} confirmed, ${summary.outcomes.probable} probable, ${summary.outcomes.review} for review, ${summary.outcomes.none} not found`,
    `Linked: ${linked.length} (confirmed ${confirmed}, probable ${linked.length - confirmed})`,
    `Review list: ${Object.keys(state.review).length}`,
    `Previews available: ${previews.available} (${previews.withoutThumb} without a custom thumbnail)`,
    `Hand-made axes available: ${axisScenes.reduce((n, a) => n + a.plan.missing.length, 0)} files in ${axisScenes.length} scenes${opts.installAxes ? `; installed ${installed}` : ''}`,
    opts.dryRun ? 'Dry run: links file not written.' : `Links: ${linksFilePath(userData)}`,
    `Report: ${reportPath}`,
  ].join('\n'));
  return exitCode;
}

main().then((code) => process.exit(code), (err) => {
  console.error(`eroscripts:sync: ${err.message}`);
  process.exit(1);
});
