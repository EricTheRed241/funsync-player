#!/usr/bin/env node
// npm run faptap:scout — find faptap.net videos worth adding, then download
// them into the review folder and the app's "Discovery" playlist.
//
//   npm run faptap:scout                 scan new videos (+ some backfill) → <userData>/faptap-scout.json
//   npm run faptap:scout -- --fetch      download the saved list
//
// Rules: scripts/faptap-scout-lib.mjs. Scenes land in `<library parent>/
// _incoming/<creator>/<Artist - Title>/` with a `faptap.json` sidecar. FapTap
// scripts are single-axis, so every scene gets generated twist / roll /
// pitch companions (scripts/upgrade-multi-axis.mjs). Videos come from
// FapTap's HLS streams through ffmpeg. Finished videos join Discovery like
// the EroScripts scout's (scripts/discovery-io.mjs).
//
// No account is needed. faptap.net serves a script only with the
// `track_token` cookie its pages set (valid an hour); without it the script
// endpoint answers 401 unless Cloudflare happens to hold a copy. The CLI
// loads the home page for one and loads it again when a script is refused.
//
// Exit codes: 0 done, 1 error, 2 rate limited / downloads left after the
// last pass.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
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

const lib = await import('./faptap-scout-lib.mjs');
const scout = await import('./eroscripts-scout-lib.mjs');
const { appRunning, applyMoves, dedupeAfterFetch, flushPlaylist, incomingDir, librarySources, libraryState, readConfig, walk } = await import('./discovery-io.mjs');
const dedupe = await import('./scene-dedupe-lib.mjs');
const { libraryVideos, readProbeCache, writeProbeCache } = await import('./scene-probe.mjs');
const { createPacer, defaultUserDataDir, parseInterval } = await import('./eroscripts-sync-lib.mjs');
const live = await import('./scout-live.mjs');
const require = createRequire(import.meta.url);
const rejectedVideos = require('../electron/rejected-videos.js');
const REPO = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

const USAGE = `Usage: npm run faptap:scout -- [options]
Scan (default; only videos not judged before, see <userData>/faptap-scout-state.json):
  --pages N              most pages of new videos (42 each, default 5)
  --backfill N           older pages to read, continuing from the last run (default 10; 0 = none)
  --full                 judge every listed video again, not only new ones
  --top N                candidates to print (default 40)
Pick:
  --add a,b,c            add these FapTap video ids to the list, skipping the animation, focus and sex
                         rules (exclusions, audio roleplays, length and library duplicates still apply)
Download (needs ffmpeg and ffprobe on PATH):
  --fetch                download the saved list, best first, add each scene to Discovery;
                         skips scripts under a minute and scenes rejected in the app (No thank you)
  --ids a,b,c            only these FapTap video ids
                         (live action by non-favourites waits for the live quota: one in every 20 downloads)
  --passes N             repeat the queue until no failures, at most N times (default 1)
  --wait 1h              pause between passes (default 1h; also 90m, 3d)
  --dry-run              list what would be downloaded (or, with --retitle, renamed)
Rename:
  --retitle              rename fetched FapTap scenes to "Artist - Title (metadata)" (artist: favourite, the
                         title's own artist, a library creator, the performer, else the uploader), filling in
                         performers and the thumbnail URL from faptap.net; moves config.json's paths (app closed)
Both:
  --user-data <dir>      app data folder (default: the desktop app's)
  --incoming <dir>       download folder (default: the Discovery source, else <library parent>/_incoming)`;

function parseArgs(argv) {
  const opts = { pages: 5, backfill: 10, top: 40, passes: 1, wait: 3600, fetch: false, dryRun: false, full: false };
  const value = (i, flag) => {
    if (i + 1 >= argv.length) throw new Error(`${flag} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--fetch': opts.fetch = true; break;
      case '--dry-run': opts.dryRun = true; break;
      case '--retitle': opts.retitle = true; break;
      case '--pages': opts.pages = Number(value(i, a)); i++; break;
      case '--backfill': opts.backfill = Number(value(i, a)); i++; break;
      case '--full': opts.full = true; break;
      case '--top': opts.top = Number(value(i, a)); i++; break;
      case '--passes': opts.passes = Number(value(i, a)); i++; break;
      case '--wait': opts.wait = parseInterval(value(i, a)); i++; break;
      case '--ids': opts.ids = new Set(value(i, a).split(',').map((s) => s.trim()).filter(Boolean)); i++; break;
      case '--add': opts.add = [...new Set(value(i, a).split(',').map((s) => s.trim()).filter(Boolean))]; i++; break;
      case '--user-data': opts.userData = path.resolve(value(i, a)); i++; break;
      case '--incoming': opts.incoming = path.resolve(value(i, a)); i++; break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  for (const k of ['pages', 'backfill', 'top', 'passes']) if (!(opts[k] >= 0)) throw new Error(`--${k} must be 0 or more`);
  if (!opts.wait) throw new Error('--wait must be an interval like 1h, 90m or 3d');
  return opts;
}

// ── faptap.net ──────────────────────────────────────────────────────────

class RateLimitedError extends Error {
  constructor() {
    super('faptap.net is rate limiting requests (HTTP 429). Progress is saved; run the command again later to resume.');
    this.name = 'RateLimitedError';
  }
}

const pacer = createPacer();
const HEADERS = { 'User-Agent': 'Mozilla/5.0' };
// `track_token=…` from the last page load.
let trackCookie = '';

/** Load the home page for a fresh `track_token` cookie. */
async function refreshTrackToken() {
  await pacer.wait();
  const resp = await fetch(`${lib.FAPTAP_ORIGIN}/`, { headers: HEADERS });
  await resp.arrayBuffer();
  if (resp.status === 429) throw new RateLimitedError();
  trackCookie = lib.mergeCookies('', resp.headers.getSetCookie());
}

/** One paced GET on faptap.net; JSON bodies unwrap their `data`. */
async function faptap(pathname, { text = false, retried = false } = {}) {
  await pacer.wait();
  const headers = { ...HEADERS, Accept: text ? 'text/plain' : 'application/json' };
  if (trackCookie) headers.Cookie = trackCookie;
  const resp = await fetch(`${lib.FAPTAP_ORIGIN}/api${pathname}`, { headers });
  if (resp.status === 429) throw new RateLimitedError();
  // A refused script: the track token is missing or past its hour.
  if (resp.status === 401 && !retried) {
    await refreshTrackToken();
    return faptap(pathname, { text, retried: true });
  }
  if (!resp.ok) throw new Error(`faptap.net ${pathname}: HTTP ${resp.status}`);
  return text ? resp.text() : (await resp.json()).data;
}

// Videos per page; the API serves at most 42 whatever `limit` asks.
const PAGE = 42;

/** listFeed's page reader: page 0 is FapTap's page 1. */
async function listPage(_feed, page) {
  const data = await faptap(`/videos?limit=${PAGE}&page=${page + 1}&order_by=created_at.desc`);
  const topics = (data?.videos || []).map((v) => ({ ...v, id: String(v.id) }));
  return { topics, more: topics.length > 0 && (page + 1) * PAGE < Number(data?.total || 0) };
}

/** FapTap's models and pornstars, for spotting live videos tagged as animation. */
async function livePerformers() {
  const all = [];
  for (let page = 1; page <= 100; page++) {
    const data = await faptap(`/performers?limit=100&page=${page}`);
    all.push(...(data?.performers || []));
    if (!data?.performers?.length || all.length >= Number(data.total || 0)) break;
  }
  return lib.livePerformerNames(all);
}

/** The video's details, script url and best stream; a reason instead when there is none worth taking. */
async function videoPlan(id, vr) {
  const detail = await faptap(`/videos/${id}`);
  if (!detail?.script?.url) return { detail, reason: 'no-script' };
  // Videos only linked from another site have no FapTap stream.
  if (!detail.stream_url_selfhosted) return { detail, reason: 'no-stream' };
  const source = lib.pickSource(await faptap(`/videos/${id}/bunny-sources`), vr);
  if (!source) return { detail, reason: 'low-quality' };
  return { detail, script: detail.script.url, source };
}

// ── Scan ────────────────────────────────────────────────────────────────

const statePath = (userData) => path.join(userData, 'faptap-scout-state.json');
const listPath = (userData) => path.join(userData, 'faptap-scout.json');

function readState(userData) {
  try { return scout.normalizeScoutState(JSON.parse(fs.readFileSync(statePath(userData), 'utf8'))); } catch { return scout.emptyScoutState(); }
}

function writeState(userData, state) {
  const target = statePath(userData);
  const tmp = `${target}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(state));
  fs.renameSync(tmp, target);
}

/** Scenes not to fetch again: those with a sidecar and video, and every rejected one. */
function doneScenes(incoming, userData) {
  const done = new Set(rejectedVideos.readRejected(userData).entries.map((e) => e.key));
  walk(incoming, 0, (full, e) => {
    if (e.name !== lib.SIDECAR) return;
    try {
      const rec = JSON.parse(fs.readFileSync(full, 'utf8'));
      if (rec.file && fs.existsSync(path.join(path.dirname(full), rec.file))) done.add(lib.fetchedKey(rec.faptapId));
    } catch { /* unreadable sidecar: fetch again */ }
  });
  return done;
}

/** Titles the library has (Discovery included) and the EroScripts scout is about to fetch. */
function knownTitles(userData, library) {
  const titles = [...library.sceneNames, ...library.videos];
  try {
    const { candidates } = JSON.parse(fs.readFileSync(path.join(userData, 'eroscripts-scout.json'), 'utf8'));
    // As the scene will be named: without `[Multi-axis]`-style labels.
    for (const c of candidates || []) titles.push(scout.normName(scout.sceneTitle({ title: c.title, author: c.author, video: 'x.mp4' })));
  } catch { /* no EroScripts list */ }
  return titles.filter(Boolean);
}

/** What screening needs to know about the library. */
async function screenContext(config, userData, incoming) {
  const roots = [...librarySources(config, incoming).map((s) => s.path), incoming].filter((p) => fs.existsSync(p));
  const library = libraryState(roots);
  try {
    const links = require('../electron/eroscripts-links.js').readLinksFile(userData);
    for (const l of Object.values(links?.links || {})) if (l.creator) library.creators.add(l.creator.toLowerCase());
  } catch { /* no links file yet */ }
  return {
    creators: [...library.creators],
    knownTitles: knownTitles(userData, library),
    livePerformers: await livePerformers(),
    favourites: live.readFavourites(userData),
  };
}

/** What naming needs: favourites and the library's creator folders as spelled (no network). */
function namingContext(config, userData, incoming) {
  const roots = [...librarySources(config, incoming).map((s) => s.path), incoming].filter((p) => fs.existsSync(p));
  return { favourites: live.readFavourites(userData), creatorNames: libraryState(roots).creatorNames };
}

/** Save the merged list and print it. */
function saveList(userData, fresh, done, { top, listed = 0, judged = 0, rejected = {} }) {
  let previous = [];
  try { previous = JSON.parse(fs.readFileSync(listPath(userData), 'utf8')).candidates || []; } catch { /* first scan */ }
  const candidates = lib.mergeCandidates(previous, fresh, (id) => done.has(lib.fetchedKey(id)));
  fs.writeFileSync(listPath(userData), JSON.stringify({ generatedAt: new Date().toISOString(), listed, judged, rejected, candidates }, null, 1));
  const freshIds = new Set(fresh.map((c) => c.id));
  const mins = (s) => `${Math.floor(s / 60)}:${String(Math.round(s % 60)).padStart(2, '0')}`;
  console.log('\n| # | New | Video | Axis | Length | Focus | Quality |\n|---|---|---|---|---|---|---|');
  candidates.slice(0, top).forEach((c, i) => {
    const why = [
      ...lib.focusOf(c), c.favourite && `favourite: ${c.favourite}`, c.picked && 'picked', c.liveQuota && 'live (quota)',
      c.artistInLibrary && `artist: ${c.artistInLibrary}`,
    ].filter(Boolean).join(', ');
    console.log(`| ${i + 1} | ${freshIds.has(c.id) ? '✓' : ''} | [${c.title.replace(/[|[\]]/g, ' ').slice(0, 60)}](${c.url}) | Single | ${mins(c.duration)} | ${why} | ${c.quality}p |`);
  });
  return candidates;
}

async function scan(opts, { userData, config, incoming }) {
  const ctx = await screenContext(config, userData, incoming);
  const done = doneScenes(incoming, userData);
  // Every library video (Discovery included) with its length, for the duplicate check.
  const roots = [...new Set([...(config.settings?.library?.sources || []).filter((s) => s.enabled !== false && s.path).map((s) => s.path), incoming])].filter((p) => fs.existsSync(p));
  const probeCache = readProbeCache(userData);
  const libVideos = await libraryVideos(roots, probeCache);
  writeProbeCache(userData, probeCache);

  const state = readState(userData);
  const rejected = {
    known: 0, excluded: 0, 'audio-only': 0, 'off-focus': 0, 'not-sexual': 0, short: 0, 'in-library': 0, duplicate: 0,
    'no-script': 0, 'no-stream': 0, 'low-quality': 0,
  };
  const fresh = [];
  let listed = 0;
  let judged = 0;
  try {
    const { topics, pagesRead } = await scout.listFeed(state, lib.FEED, listPage, {
      maxNew: opts.pages, backfill: opts.backfill, full: opts.full, rules: lib.FAPTAP_RULES_VERSION,
    });
    const videos = new Map(topics.map((v) => [v.id, v]));
    listed = videos.size;
    const todo = [...videos.values()].filter((v) => opts.full || !scout.seenTopic(state, v.id, lib.FAPTAP_RULES_VERSION));
    console.log(`Listed ${listed} videos on ${pagesRead} pages; ${todo.length} not judged before`);

    for (const v of todo) {
      const verdict = (r) => {
        scout.markTopic(state, v.id, r, Date.now(), lib.FAPTAP_RULES_VERSION);
        judged++;
        if (r !== 'candidate') rejected[r]++;
      };
      if (done.has(lib.fetchedKey(v.id))) { verdict('known'); continue; }
      const s = lib.screenVideo(v, ctx);
      if (!s.ok) { verdict(s.reason); continue; }
      // Same title and length as a library video (an EroScripts scene always wins over FapTap).
      if (dedupe.firstDuplicate({ words: dedupe.titleWords(v.name || ''), duration: Number(v.duration) || 0, vr: s.vr }, libVideos)) { verdict('duplicate'); continue; }
      let plan;
      try { plan = await videoPlan(v.id, s.vr); } catch (err) {
        if (err instanceof RateLimitedError) throw err;
        process.stderr.write(`! ${v.id} ${err.message}\n`);
        continue;
      }
      if (plan.reason) { verdict(plan.reason); continue; }
      fresh.push({ ...lib.toCandidate(v, lib.withPerformers(s, plan.detail?.performers, ctx.favourites)), quality: plan.source.height });
      verdict('candidate');
      process.stderr.write(`✓ ${v.id} ${v.name.slice(0, 70)}\n`);
    }
  } finally {
    // Progress survives a rate limit: the next run goes on from here.
    state.lastScanAt = new Date().toISOString();
    writeState(userData, state);
  }

  const candidates = saveList(userData, fresh, done, { top: opts.top, listed, judged, rejected });
  const feed = state.feeds[lib.FEED];
  const quota = candidates.filter((c) => c.liveQuota).length;
  console.log(`\nListed ${listed} · judged ${judged} · rejected ${JSON.stringify(rejected)} · new ${fresh.length} · waiting to download ${candidates.length} (${quota} live on the quota)`);
  console.log(`Backfill: ${feed?.exhausted ? 'done' : `page ${feed?.backfillPage ?? 0}`} · ${Object.keys(state.topics).length} videos judged so far`);
  console.log(`Saved: ${listPath(userData)}`);
  return 0;
}

/** `--add`: put videos the user picked on the list, past the animation, focus and sex rules. */
async function addPicked(opts, { userData, config, incoming }) {
  const ctx = await screenContext(config, userData, incoming);
  const done = doneScenes(incoming, userData);
  const fresh = [];
  for (const id of opts.add) {
    if (done.has(lib.fetchedKey(id))) { console.log(`– ${id}: already fetched or turned down`); continue; }
    const plan = await videoPlan(id, false);
    const v = plan.detail;
    if (!v) { console.log(`✗ ${id}: not found`); continue; }
    const s = lib.screenVideo({ ...v, id: String(v.id) }, { ...ctx, picked: true });
    if (!s.ok) { console.log(`✗ ${id} ${v.name}: ${s.reason}`); continue; }
    const p = s.vr ? await videoPlan(id, true) : plan;
    if (p.reason) { console.log(`✗ ${id} ${v.name}: ${p.reason}`); continue; }
    fresh.push({ ...lib.toCandidate({ ...v, id: String(v.id) }, lib.withPerformers(s, v.performers, ctx.favourites)), quality: p.source.height });
    console.log(`✓ ${id} ${v.name}`);
  }
  saveList(userData, fresh, done, { top: fresh.length });
  console.log(`\nAdded ${fresh.length} of ${opts.add.length}. Download them with: npm run faptap:scout -- --fetch --ids ${fresh.map((c) => c.id).join(',')}`);
  return 0;
}

// ── Fetch ───────────────────────────────────────────────────────────────

/** The `.part` being written; removed if the run is stopped (Ctrl-C, kill). */
let activePart = null;
for (const [sig, code] of [['SIGINT', 130], ['SIGTERM', 143]]) {
  process.on(sig, () => {
    if (activePart) fs.rmSync(activePart, { force: true });
    process.exit(code);
  });
}

function run(cmd, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    let err = '';
    child.stdout.on('data', (d) => { out += d; });
    child.stderr.on('data', (d) => { err += d; });
    child.on('error', reject);
    child.on('close', (status) => (status === 0 ? resolve(out) : reject(new Error(`${cmd}: ${(err.trim().split('\n').pop() || `exit ${status}`)}`))));
  });
}

/**
 * Copy an HLS stream into `dest` through `<staging>/<name>.part`, then check
 * its length against FapTap's (2 % or 5 s). A failed attempt deletes its `.part`.
 */
async function downloadStream(url, dest, staging, expectedSec) {
  fs.mkdirSync(staging, { recursive: true });
  const tmp = path.join(staging, `${path.basename(dest)}.part`);
  activePart = tmp;
  try {
    await run('ffmpeg', ['-nostdin', '-v', 'error', '-y', '-i', url, '-c', 'copy', '-bsf:a', 'aac_adtstoasc', '-movflags', '+faststart', '-f', 'mp4', tmp]);
    const seconds = Number((await run('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', tmp])).trim());
    if (expectedSec && !(Math.abs(seconds - expectedSec) <= Math.max(5, expectedSec * 0.02))) {
      throw new Error(`length ${Math.round(seconds)} s ≠ ${expectedSec} s`);
    }
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.renameSync(tmp, dest);
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    throw err;
  } finally {
    activePart = null;
  }
}

/** `<parent>/<stem>`, or `<stem> (2)` … when another scene holds that folder. */
function sceneDir(parent, stem, id) {
  for (let n = 1; ; n++) {
    const dir = path.join(parent, n === 1 ? stem : `${stem} (${n})`);
    if (!fs.existsSync(dir)) return dir;
    try {
      if (String(JSON.parse(fs.readFileSync(path.join(dir, lib.SIDECAR), 'utf8')).faptapId) === String(id)) return dir;
    } catch { /* another scene's folder */ }
  }
}

async function fetchPass(queue, { userData, incoming, staging, naming }) {
  const done = doneScenes(incoming, userData);
  let ok = 0; let failed = 0; let short = 0; let waiting = 0;
  const now = () => new Date().toISOString().slice(11, 19);
  for (const c of queue) {
    if (done.has(lib.fetchedKey(c.id))) continue;
    if (c.liveQuota && !live.liveAllowed(live.readQuota(userData))) { waiting++; continue; }
    let creator = c.author;
    let stem = c.title;
    try {
      const plan = await videoPlan(c.id, c.vr);
      if (plan.reason) throw new Error(plan.reason);
      const performers = (plan.detail?.performers || []).filter((p) => p?.name).map((p) => ({ name: p.name, category: p.category || '' }));
      const name = lib.faptapSceneName({ title: c.title, uploader: c.author, performers, tags: c.tags, vr: c.vr, projection: c.projection }, naming);
      creator = name.creator;
      const dir = sceneDir(path.join(incoming, creator), name.stem, c.id);
      stem = path.basename(dir);
      const video = path.join(dir, `${stem}.mp4`);
      const script = lib.csvToFunscript(await faptap(`/assets/${plan.script}`, { text: true }));
      const lengthMs = scout.scriptDurationMs(script);
      if (lengthMs < scout.MIN_VIDEO_MS) {
        rejectedVideos.recordRejection(userData, {
          source: 'faptap', faptapId: c.id, title: c.title, url: c.url, reason: 'short', lengthMs, at: new Date().toISOString(),
        });
        console.log(`${now()} – ${creator}/${stem}: script ${Math.round(lengthMs / 1000)} s, under a minute; skipped`);
        short++;
        continue;
      }
      // The script goes in once the video is there, so a failed download
      // leaves no half scene behind.
      await downloadStream(plan.source.url, video, staging, c.duration);
      fs.writeFileSync(path.join(dir, scout.scriptFileName(stem, '')), JSON.stringify(script));
      fs.writeFileSync(path.join(dir, lib.SIDECAR), JSON.stringify({
        faptapId: c.id, url: c.url, title: c.title, author: c.author, artist: name.artist, performers, tags: c.tags,
        thumbnailUrl: lib.thumbnailUrl(plan.detail), vr: !!c.vr, projection: c.projection || null,
        file: path.basename(video), quality: plan.source.height, at: new Date().toISOString(),
      }, null, 1));
      const up = spawnSync(process.execPath, [path.join(REPO, 'scripts/upgrade-multi-axis.mjs'), dir], { cwd: REPO, encoding: 'utf8' });
      const note = up.status === 0 ? ' + generated axes' : ` (axis generation failed: ${(up.stderr || up.stdout).trim().split('\n').pop()})`;
      const listed = flushPlaylist(userData, incoming, [video]);
      live.recordDownload(userData, !!c.liveQuota);
      const kind = c.liveQuota ? ' [live quota]' : c.favourite && c.live ? ` [favourite: ${c.favourite}]` : '';
      console.log(`${now()} ✓ ${creator}/${stem} (${plan.source.height}p)${kind}${note}${listed ? '' : ' (playlist pending: app is open)'}`);
      ok++;
    } catch (err) {
      if (err instanceof RateLimitedError) throw err;
      console.log(`${now()} ✗ ${creator}/${stem}: ${err.message}`);
      failed++;
    }
  }
  return { ok, failed, short, waiting };
}

async function fetchAll(opts, { userData, config, incoming }) {
  if (!fs.existsSync(listPath(userData))) throw new Error(`No FapTap list at ${listPath(userData)}; run npm run faptap:scout first.`);
  const { candidates } = JSON.parse(fs.readFileSync(listPath(userData), 'utf8'));
  // Live action on the quota goes last, so this run's downloads can earn it a slot.
  const byScore = (a, b) => b.score - a.score;
  const wanted = candidates.filter(lib.inFocus).filter((c) => !opts.ids || opts.ids.has(String(c.id)));
  const queue = [...wanted.filter((c) => !c.liveQuota).sort(byScore), ...wanted.filter((c) => c.liveQuota).sort(byScore)];
  console.log(`Queue: ${queue.length} videos → ${incoming}`);
  if (opts.dryRun) {
    for (const c of queue) console.log(`  ${c.id} ${c.quality}p ${c.title}`);
    return 0;
  }
  for (const tool of ['ffmpeg', 'ffprobe']) {
    if (spawnSync(tool, ['-version']).status !== 0) throw new Error(`${tool} not found on PATH; install it (brew install ffmpeg) and re-run.`);
  }
  const staging = path.join(path.dirname(incoming), '_staging');
  for (let pass = 1; pass <= Math.max(1, opts.passes); pass++) {
    console.log(`Pass ${pass}/${Math.max(1, opts.passes)}`);
    const r = await fetchPass(queue, { userData, incoming, staging, naming: namingContext(config, userData, incoming) });
    console.log(`Pass ${pass}: ${r.ok} downloaded with generated axes, ${r.short} under a minute, ${r.failed} failed${r.waiting ? `, ${r.waiting} live waiting for the quota (one in every ${live.LIVE_QUOTA_EVERY} downloads)` : ''}`);
    if (r.ok) dedupeAfterFetch(userData);
    if (!r.failed) return 0;
    if (pass < opts.passes) {
      console.log(`Waiting ${opts.wait / 60} min before the next pass`);
      await new Promise((resolve) => setTimeout(resolve, opts.wait * 1000));
    }
  }
  flushPlaylist(userData, incoming, []);
  return 2;
}

// ── Retitle ─────────────────────────────────────────────────────────────

/**
 * Rename fetched FapTap scenes to the current naming, after filling in what
 * older sidecars lack (performers, thumbnail URL, VR projection) from
 * faptap.net. config.json's paths move with them, so the app must be closed.
 */
async function retitle(opts, { userData, config, incoming }) {
  if (!opts.dryRun && appRunning(userData)) {
    console.error('FunSync is running. Close it, then re-run --retitle: config.json holds the scene paths.');
    return 2;
  }
  const scenes = [];
  walk(incoming, 0, (full, e) => {
    if (e.name !== lib.SIDECAR) return;
    const dir = path.dirname(full);
    try {
      scenes.push({ dir, file: full, rec: JSON.parse(fs.readFileSync(full, 'utf8')), files: fs.readdirSync(dir).filter((n) => !n.startsWith('._')) });
    } catch { process.stderr.write(`! unreadable ${full}\n`); }
  });
  let filled = 0;
  for (const s of scenes) {
    if (Array.isArray(s.rec.performers) && s.rec.thumbnailUrl && 'projection' in s.rec) continue;
    try {
      const d = await faptap(`/videos/${s.rec.faptapId}`);
      s.rec = {
        ...s.rec, performers: (d?.performers || []).filter((p) => p?.name).map((p) => ({ name: p.name, category: p.category || '' })), thumbnailUrl: lib.thumbnailUrl(d),
        vr: !!d?.vr, projection: d?.projection || null,
      };
      filled++;
      if (!opts.dryRun) fs.writeFileSync(s.file, JSON.stringify(s.rec, null, 1));
    } catch (err) {
      if (err instanceof RateLimitedError) throw err;
      process.stderr.write(`! ${s.rec.faptapId} ${err.message}\n`);
    }
  }
  const naming = namingContext(config, userData, incoming);
  const nameOf = (s) => {
    const n = lib.faptapSceneName({ title: s.rec.title, uploader: s.rec.author, performers: s.rec.performers, tags: s.rec.tags, vr: s.rec.vr, projection: s.rec.projection }, naming);
    if (!opts.dryRun && s.rec.artist !== n.artist) fs.writeFileSync(s.file, JSON.stringify({ ...s.rec, artist: n.artist }, null, 1));
    return { creator: n.creator, stem: n.stem };
  };
  const moves = scout.retitlePlan(scenes, incoming, (p) => fs.existsSync(p), nameOf);
  const rel = (p) => path.relative(incoming, p);
  for (const m of moves) console.log(`${rel(m.from)}\n  → ${rel(m.to)}`);
  console.log(`${filled} sidecars ${opts.dryRun ? 'would get' : 'got'} performers / thumbnail from faptap.net; ${moves.length} of ${scenes.length} scenes ${opts.dryRun ? 'would be renamed' : 'to rename'}`);
  if (opts.dryRun || !moves.length) return 0;
  return applyMoves(moves, { userData, incoming, sidecar: lib.SIDECAR });
}

// ── Main ────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserDataDir();
  const config = readConfig(userData);
  const incoming = incomingDir(config, opts.incoming);
  try {
    if (opts.add) return await addPicked(opts, { userData, config, incoming });
    if (opts.retitle) return await retitle(opts, { userData, config, incoming });
    return opts.fetch ? await fetchAll(opts, { userData, config, incoming }) : await scan(opts, { userData, config, incoming });
  } catch (err) {
    if (err instanceof RateLimitedError) { console.error(err.message); return 2; }
    throw err;
  }
}

main().then((code) => process.exit(code), (err) => {
  console.error(`faptap:scout: ${err.message}`);
  process.exit(1);
});
