#!/usr/bin/env node
// npm run eroscripts:scout — find free EroScripts posts worth adding, then
// download them into a review folder and the app's "Discovery" playlist.
//
//   npm run eroscripts:scout                 scan new posts (+ a little backfill) → <userData>/eroscripts-scout.json
//   npm run eroscripts:scout -- --fetch      download the saved list
//   npm run eroscripts:scout -- --retitle    rename fetched scenes to `Artist - Title (metadata)`
//
// Rules and ranking: scripts/eroscripts-scout-lib.mjs. Scenes land in
// `<library parent>/_incoming/<creator>/<Artist - Title (metadata)>/` with an
// `eroscripts.json` sidecar. Single-axis scenes get generated twist / roll / pitch companions
// (scripts/upgrade-multi-axis.mjs). Each finished video joins the Discovery
// playlist, and `_incoming` becomes a library source named Discovery, so the
// scenes show in the library, the web remote and HereSphere. config.json is
// written only while the app is closed; otherwise the paths wait in
// `<userData>/eroscripts-scout-pending.json` for the next run.
//
// Exit codes: 0 done, 1 error, 2 rate limited / downloads left after the
// last pass / app running (--retitle), 3 not logged in / session expired.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { createRequire } from 'node:module';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
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

const lib = await import('./eroscripts-scout-lib.mjs');
const { createPacer, defaultUserDataDir, parseInterval, RateLimitedError, SessionExpiredError } = await import('./eroscripts-sync-lib.mjs');
const require = createRequire(import.meta.url);
const rejectedVideos = require('../electron/rejected-videos.js');
const REPO = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

const USAGE = `Usage: npm run eroscripts:scout -- [options]
Scan (default; only posts not judged before, see <userData>/eroscripts-scout-state.json):
  --pages N              most pages of new posts on the multi-axis feed (30 posts each, default 6)
  --anim-pages N         most pages of new posts on each animation feed (animation, hentai, 3d, vam, sfm, mmd; default 3)
  --backfill N           older pages to read per feed, continuing from the last run (default 2; 0 = none)
  --full                 judge every listed post again, not only new ones
  --top N                candidates to print (default 40)
Download:
  --fetch                download the saved list (multi-axis first), add each scene to Discovery;
                         skips videos under a minute and scenes rejected in the app (No thank you)
  --ids 1,2,3            only these post ids
  --passes N             repeat the queue until no failures, at most N times (default 1)
  --wait 1h              pause between passes (default 1h; also 90m, 3d)
  --dry-run              list what would be downloaded (or, with --retitle, renamed)
Rename:
  --retitle              rename fetched scenes to "Artist - Title (metadata)" from their posts,
                         and the paths config.json keeps for them (app must be closed)
Both:
  --user-data <dir>      app data folder (default: the desktop app's)
  --incoming <dir>       download folder (default <library parent>/_incoming)`;

function parseArgs(argv) {
  const opts = { pages: 6, animPages: 3, backfill: 2, top: 40, passes: 1, wait: 3600, fetch: false, dryRun: false, full: false };
  const value = (i, flag) => {
    if (i + 1 >= argv.length) throw new Error(`${flag} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--fetch': opts.fetch = true; break;
      case '--retitle': opts.retitle = true; break;
      case '--dry-run': opts.dryRun = true; break;
      case '--pages': opts.pages = Number(value(i, a)); i++; break;
      case '--anim-pages': opts.animPages = Number(value(i, a)); i++; break;
      case '--backfill': opts.backfill = Number(value(i, a)); i++; break;
      case '--full': opts.full = true; break;
      case '--top': opts.top = Number(value(i, a)); i++; break;
      case '--passes': opts.passes = Number(value(i, a)); i++; break;
      case '--wait': opts.wait = parseInterval(value(i, a)); i++; break;
      case '--ids': opts.ids = new Set(value(i, a).split(',').map(Number)); i++; break;
      case '--user-data': opts.userData = path.resolve(value(i, a)); i++; break;
      case '--incoming': opts.incoming = path.resolve(value(i, a)); i++; break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  for (const k of ['pages', 'animPages', 'backfill', 'top', 'passes']) if (!(opts[k] >= 0)) throw new Error(`--${k} must be 0 or more`);
  if (!opts.wait) throw new Error('--wait must be an interval like 1h, 90m or 3d');
  return opts;
}

// ── App state ───────────────────────────────────────────────────────────

const configPath = (userData) => path.join(userData, 'config.json');
const readConfig = (userData) => JSON.parse(fs.readFileSync(configPath(userData), 'utf8'));

/** The app holds Chromium's SingletonLock (`<host>-<pid>`) in userData while it runs. */
function appRunning(userData) {
  let target;
  try { target = fs.readlinkSync(path.join(userData, 'SingletonLock')); } catch { return false; }
  const pid = Number(target.split('-').pop());
  if (!pid) return true;
  try { process.kill(pid, 0); return true; } catch (err) { return err.code === 'EPERM'; }
}

function writeConfig(userData, config) {
  const file = configPath(userData);
  fs.copyFileSync(file, `${file}.bak-discovery`);
  const tmp = `${file}.tmp-discovery`;
  fs.writeFileSync(tmp, JSON.stringify(config, null, '\t'));
  fs.renameSync(tmp, file);
}

/** Rename `from` to `to`, with its AppleDouble `._` sidecar if the volume keeps one. */
function renameWithSidecar(from, to) {
  fs.renameSync(from, to);
  const side = (p) => path.join(path.dirname(p), `._${path.basename(p)}`);
  if (fs.existsSync(side(from)) && !fs.existsSync(side(to))) fs.renameSync(side(from), side(to));
}

/**
 * Remove a folder that holds nothing but `._*` / `.DS_Store` leftovers. exFAT
 * lists names decomposed (NFD) but finds some `._` files only by their
 * composed (NFC) spelling, so both are tried; a folder that won't go stays.
 */
function removeIfEmpty(dir) {
  let names;
  try { names = fs.readdirSync(dir); } catch { return; }
  if (names.some((n) => !n.startsWith('._') && n !== '.DS_Store')) return;
  for (const n of names) for (const form of [n, n.normalize('NFC')]) fs.rmSync(path.join(dir, form), { force: true });
  try { fs.rmdirSync(dir); } catch { return; }
  const side = path.join(path.dirname(dir), `._${path.basename(dir)}`);
  for (const form of [side, side.normalize('NFC')]) fs.rmSync(form, { force: true });
}

function librarySources(config, incoming) {
  return (config.settings?.library?.sources || [])
    .filter((s) => s.enabled !== false && s.path && path.resolve(s.path) !== path.resolve(incoming || '/nonexistent'));
}

function walk(dir, depth, visit) {
  let entries = [];
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const e of entries) {
    if (e.name.startsWith('.')) continue;
    const full = path.join(dir, e.name);
    if (e.isDirectory()) { visit(full, e, depth); if (depth < 3) walk(full, depth + 1, visit); } else visit(full, e, depth);
  }
}

/** What the library already has: creators, scene folder names, video names. */
function libraryState(roots) {
  const creators = new Set();
  const sceneNames = [];
  const videos = [];
  for (const root of roots) {
    walk(root, 0, (full, e, depth) => {
      if (e.isDirectory()) {
        if (depth === 0) creators.add(e.name.toLowerCase());
        if (depth === 1) sceneNames.push(lib.normName(e.name));
      } else if (lib.isVideoName(e.name)) videos.push(lib.coreName(e.name));
    });
  }
  return { creators, sceneNames, videos };
}

// ── Hosts ───────────────────────────────────────────────────────────────

async function megaFiles(url) {
  const { File } = await import('megajs');
  const f = File.fromURL(url);
  await f.loadAttributes();
  const out = [];
  const visit = (n) => {
    for (const c of n.children || []) {
      if (c.directory) visit(c);
      else out.push({ name: c.name, size: c.size, host: 'mega', url, remote: c.name });
    }
  };
  if (f.directory) visit(f); else out.push({ name: f.name, size: f.size, host: 'mega', url, remote: f.name });
  return out;
}

// No forum cookie goes to pixeldrain.
async function pixeldrainFiles(kind, id) {
  const headers = { 'User-Agent': 'Mozilla/5.0' };
  const fileUrl = (fid) => `https://pixeldrain.com/api/file/${fid}?download`;
  if (kind === 'l') {
    const d = await (await fetch(`https://pixeldrain.com/api/list/${id}`, { headers })).json();
    return (d.files || []).map((f) => ({ name: f.name, size: f.size, host: 'pixeldrain', url: fileUrl(f.id) }));
  }
  const d = await (await fetch(`https://pixeldrain.com/api/file/${id}/info`, { headers })).json();
  return d?.name ? [{ name: d.name, size: d.size, host: 'pixeldrain', url: fileUrl(id) }] : [];
}

/** Every script and video a post offers: author's forum uploads first, then MEGA and pixeldrain. */
async function topicFiles(api, pacer, topicId, cooked) {
  await pacer.wait();
  const att = await api.getTopicAttachments(topicId);
  if (att.rateLimited) throw new RateLimitedError();
  if (att.authExpired) throw new SessionExpiredError();
  const files = att.attachments.filter((a) => a.byAuthor).map((a) => ({ name: a.name, host: 'forum', url: a.url }));
  const hosts = new Set(att.external.map((e) => e.host));
  const hrefs = [...new Set([...cooked.matchAll(/href="(https?:\/\/[^"]+)"/g)].map((m) => m[1].replace(/&amp;/g, '&')))];
  for (const u of hrefs.filter((h) => /mega\.(nz|io)\//.test(h))) {
    try { files.push(...await megaFiles(u)); } catch { hosts.add('MEGA (unreadable)'); }
  }
  for (const m of cooked.matchAll(/https?:\/\/(?:www\.)?pixeldrain\.(?:com|net)\/(u|l)\/([A-Za-z0-9]+)/g)) {
    try { files.push(...await pixeldrainFiles(m[1], m[2])); } catch { hosts.add('pixeldrain (unreadable)'); }
  }
  const byName = new Map();
  for (const f of files) if (!byName.has(f.name)) byName.set(f.name, f);
  return { files: [...byName.values()], hosts: [...hosts] };
}

// ── Scan ────────────────────────────────────────────────────────────────

const statePath = (userData) => path.join(userData, 'eroscripts-scout-state.json');

function readScoutState(userData) {
  try { return lib.normalizeScoutState(JSON.parse(fs.readFileSync(statePath(userData), 'utf8'))); } catch { return lib.emptyScoutState(); }
}

function writeScoutState(userData, state) {
  const target = statePath(userData);
  const tmp = `${target}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(state));
  fs.renameSync(tmp, target);
}

/** One page of a Free Scripts tag feed, paced; forum errors become exceptions. */
function feedPage(api, pacer) {
  return async (tag, page) => {
    await pacer.wait();
    const r = await api.listFreeTagTopics(tag, page);
    if (r.rateLimited) throw new RateLimitedError();
    if (r.authExpired) throw new SessionExpiredError();
    if (r.error) throw new Error(r.error);
    return r;
  };
}

async function scan(opts, { api, userData, config, incoming }) {
  const pacer = createPacer();
  const roots = librarySources(config, incoming).map((s) => s.path).filter((p) => fs.existsSync(p));
  const library = libraryState(roots);
  const knownTopics = new Set(Object.values(config.settings?.library?.scriptTags || {}).map((r) => r?.topicId).filter((id) => id != null));
  try {
    const links = require('../electron/eroscripts-links.js').readLinksFile(userData);
    for (const l of Object.values(links?.links || {})) if (l.topicId != null) knownTopics.add(l.topicId);
    for (const l of Object.values(links?.links || {})) if (l.creator) library.creators.add(l.creator.toLowerCase());
  } catch { /* no links file yet */ }
  // Posts the user turned down in the app (No thank you, 2 stars or lower).
  const turnedDown = new Set(rejectedVideos.readRejected(userData).entries.filter((e) => e.reason !== 'short').map((e) => e.topicId));
  for (const id of turnedDown) knownTopics.add(id);

  const state = readScoutState(userData);
  const rejected = { known: 0, excluded: 0, 'not-animated': 0, 'not-sexual': 0, 'in-library': 0, 'no-video': 0, 'single-axis': 0 };
  const fresh = [];
  let listed = 0;
  let judged = 0;
  try {
    const topics = new Map();
    let pages = 0;
    for (const tag of lib.FEED_TAGS) {
      const maxNew = tag === 'multi-axis' ? opts.pages : opts.animPages;
      const r = await lib.listFeed(state, tag, feedPage(api, pacer), { maxNew, backfill: opts.backfill, full: opts.full });
      pages += r.pagesRead;
      for (const t of r.topics) topics.set(t.id, t);
    }
    listed = topics.size;
    const todo = [...topics.values()].filter((t) => opts.full || !lib.seenTopic(state, t.id));
    console.log(`Listed ${listed} posts on ${pages} pages; ${todo.length} not judged before`);

    const creators = [...library.creators];
    for (const t of todo) {
      const verdict = (v) => { lib.markTopic(state, t.id, v); judged++; if (v !== 'candidate') rejected[v]++; };
      const s = lib.screenTopic(t, { knownTopics, creators, sceneNames: library.sceneNames });
      if (!s.ok) { verdict(s.reason); continue; }
      await pacer.wait();
      const data = await api.getTopicData(t.id);
      const op = data?.post_stream?.posts?.[0];
      if (!op) { process.stderr.write(`! ${t.id} unreadable\n`); continue; }
      const cooked = op.cooked || '';
      const text = cooked.replace(/<[^>]+>/g, ' ');
      if (lib.hasExcludedText(text)) { verdict('excluded'); continue; }
      if (!lib.isSexual({ tags: t.tags, title: t.title, text })) { verdict('not-sexual'); continue; }
      let files, hosts;
      try { ({ files, hosts } = await topicFiles(api, pacer, t.id, cooked)); } catch (err) {
        if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
        process.stderr.write(`! ${t.id} ${err.message}\n`);
        continue;
      }
      const plan = lib.planScenes({
        scripts: files.filter((f) => /\.funscript$/i.test(f.name)),
        videos: files.filter((f) => lib.isVideoName(f.name)),
        vr: s.vr,
        libraryVideos: library.videos,
      });
      if (!plan.scenes.length) { verdict(plan.inLibrary > 0 ? 'in-library' : 'no-video'); continue; }
      const author = op.username;
      const artistInLibrary = s.artist || (library.creators.has(author.toLowerCase()) ? author : null);
      const scenes = lib.wantedScenes({ tags: t.tags, title: t.title, vr: s.vr, artistInLibrary: !!artistInLibrary, scenes: plan.scenes });
      if (!scenes.length) { verdict('single-axis'); continue; }
      const preferred = t.tags.filter((x) => lib.PREFERRED_TAGS.has(x));
      const axisType = scenes.some((sc) => sc.axes.length) ? 'multi' : 'single';
      fresh.push({
        id: t.id, title: t.title, url: `https://discuss.eroscripts.com/t/${t.id}`, author,
        tags: t.tags, likes: t.likes,
        vr: s.vr, music: s.music, preferred, axisType, artistInLibrary, hosts,
        score: +lib.scoreCandidate({ vr: s.vr, music: s.music, artistInLibrary: !!artistInLibrary, preferred, multiAxis: axisType === 'multi', likes: t.likes }).toFixed(2),
        scenes,
      });
      verdict('candidate');
      process.stderr.write(`✓ ${t.id} ${t.title.slice(0, 70)}\n`);
    }
  } finally {
    // Progress survives a rate limit or an expired session: the next run
    // goes on from here.
    state.lastScanAt = new Date().toISOString();
    writeScoutState(userData, state);
  }

  const out = path.join(userData, 'eroscripts-scout.json');
  let previous = [];
  try { previous = JSON.parse(fs.readFileSync(out, 'utf8')).candidates || []; } catch { /* first scan */ }
  const done = fetchedScenes(incoming, userData);
  const candidates = lib.mergeCandidates(previous, fresh, (id, video) => done.has(lib.fetchedKey(id, video)), turnedDown);
  fs.writeFileSync(out, JSON.stringify({ generatedAt: new Date().toISOString(), listed, judged, rejected, candidates }, null, 1));

  const gb = (c) => c.scenes.reduce((n, sc) => n + (sc.video.size || 0), 0) / 1e9;
  const freshIds = new Set(fresh.map((c) => c.id));
  console.log('\n| # | New | Post | Axis | Tags | Videos | GB | Host |\n|---|---|---|---|---|---|---|---|');
  candidates.slice(0, opts.top).forEach((c, i) => {
    const axis = c.axisType === 'multi' ? `Multi (${1 + new Set(c.scenes.flatMap((sc) => sc.axes)).size})` : 'Single';
    const tags = [c.vr && 'VR', ...c.preferred, c.music && 'music'].filter(Boolean).join(', ');
    console.log(`| ${i + 1} | ${freshIds.has(c.id) ? '✓' : ''} | [${c.title.replace(/[|[\]]/g, ' ').slice(0, 60)}](${c.url}) | ${axis} | ${tags} | ${c.scenes.length} | ${gb(c).toFixed(2)} | ${[...new Set(c.scenes.map((sc) => sc.video.host))].join('+')} |`);
  });
  const multi = candidates.filter((c) => c.axisType === 'multi').length;
  const backfill = lib.FEED_TAGS.map((tag) => `${tag} ${state.feeds[tag]?.exhausted ? 'done' : `p${state.feeds[tag]?.backfillPage ?? 0}`}`).join(', ');
  console.log(`\nListed ${listed} · judged ${judged} · rejected ${JSON.stringify(rejected)} · new ${fresh.length} · waiting to download ${candidates.length} (${multi} multi-axis, ${candidates.length - multi} single-axis), ${candidates.reduce((n, c) => n + c.scenes.length, 0)} videos, ${candidates.reduce((n, c) => n + gb(c), 0).toFixed(1)} GB`);
  console.log(`Backfill: ${backfill} · ${Object.keys(state.topics).length} posts judged so far`);
  console.log(`Saved: ${out}`);
  return 0;
}

// ── Fetch ───────────────────────────────────────────────────────────────

async function download(api, file, dest, staging) {
  if (fs.existsSync(dest)) return 'have';
  fs.mkdirSync(staging, { recursive: true });
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = path.join(staging, `${path.basename(dest)}.part`);
  if (file.host === 'forum') {
    const r = await api.downloadFile(file.url, tmp, { overwrite: true });
    if (!r.success) throw new Error(`forum: ${r.error}`);
  } else if (file.host === 'mega') {
    const { File } = await import('megajs');
    const root = File.fromURL(file.url);
    await root.loadAttributes();
    const find = (n) => {
      for (const c of n.children || []) {
        if (c.directory) { const hit = find(c); if (hit) return hit; } else if (c.name === file.remote) return c;
      }
      return null;
    };
    const node = root.directory ? find(root) : root;
    if (!node) throw new Error(`mega: ${file.remote} is gone`);
    await pipeline(node.download({}), fs.createWriteStream(tmp));
  } else if (file.host === 'pixeldrain') {
    const resp = await fetch(file.url, { headers: { 'User-Agent': 'Mozilla/5.0' } });
    if (!resp.ok) throw new Error(`pixeldrain: HTTP ${resp.status}`);
    await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp));
  } else {
    throw new Error(`unknown host ${file.host}`);
  }
  const size = fs.statSync(tmp).size;
  if (file.size && size !== file.size) { fs.rmSync(tmp, { force: true }); throw new Error(`size ${size} ≠ ${file.size}`); }
  fs.renameSync(tmp, dest);
  return 'ok';
}

/** Scenes not to fetch: those with a sidecar and video, and every rejected one. */
function fetchedScenes(incoming, userData) {
  const done = new Set(rejectedVideos.readRejected(userData).entries.map((e) => e.key));
  walk(incoming, 0, (full, e) => {
    if (e.name !== 'eroscripts.json') return;
    try {
      const rec = JSON.parse(fs.readFileSync(full, 'utf8'));
      const dir = path.dirname(full);
      const file = rec.file || fs.readdirSync(dir).find((n) => !n.startsWith('._') && lib.isVideoName(n));
      if (file && fs.existsSync(path.join(dir, file))) done.add(lib.fetchedKey(rec.topicId, rec.video));
    } catch { /* unreadable sidecar: fetch again */ }
  });
  return done;
}

/**
 * `p` spelled the way the drive lists it. Names from MEGA come in one Unicode
 * form (NFC) while some volumes list another, a folder may exist in another
 * case (`bubukuin` for `Bubukuin`), and the playlist and headset match the
 * library scan's paths byte for byte.
 */
function onDiskPath(p) {
  let cur = path.sep;
  for (const seg of path.resolve(p).split(path.sep).filter(Boolean)) {
    let hit = seg;
    try {
      const names = fs.readdirSync(cur);
      const nfc = seg.normalize('NFC');
      hit = names.find((n) => n.normalize('NFC') === nfc) ?? names.find((n) => n.normalize('NFC').toLowerCase() === nfc.toLowerCase()) ?? seg;
    } catch { /* keep as given */ }
    cur = path.join(cur, hit);
  }
  return cur;
}

/** Add finished videos (and any left from earlier runs) to Discovery; false when the app is open. */
function flushPlaylist(userData, incoming, videoPaths) {
  const pendingFile = path.join(userData, 'eroscripts-scout-pending.json');
  let pending = [];
  try { pending = JSON.parse(fs.readFileSync(pendingFile, 'utf8')); } catch { /* none */ }
  const all = [...new Set([...pending, ...videoPaths].filter((p) => fs.existsSync(p)).map(onDiskPath))];
  if (!all.length) return true;
  if (appRunning(userData)) { fs.writeFileSync(pendingFile, JSON.stringify(all, null, 1)); return false; }
  const config = readConfig(userData);
  const changes = lib.addToDiscovery(config, all, { incomingDir: incoming, newId: randomUUID });
  if (changes.sourceAdded || changes.playlistCreated || changes.added.length) writeConfig(userData, config);
  fs.rmSync(pendingFile, { force: true });
  if (changes.sourceAdded) console.log(`  added library source "${lib.DISCOVERY_NAME}" → ${incoming}`);
  if (changes.playlistCreated) console.log(`  created playlist "${lib.DISCOVERY_NAME}"`);
  return true;
}

async function fetchPass(queue, { api, userData, incoming, staging }) {
  const done = fetchedScenes(incoming, userData);
  let ok = 0; let failed = 0; let generated = 0; let short = 0;
  const now = () => new Date().toISOString().slice(11, 19);
  for (const c of queue) {
    for (const sc of c.scenes) {
      if (done.has(lib.fetchedKey(c.id, sc.video.name))) continue;
      const creator = lib.creatorFolder(c.title, c.author);
      const stem = lib.sceneTitle({ title: c.title, author: c.author, video: sc.video.name, shared: c.scenes.length > 1 });
      const dir = path.join(incoming, creator, stem);
      const video = path.join(dir, stem + path.extname(sc.video.name).toLowerCase());
      try {
        for (const f of sc.scripts) await download(api, f, path.join(dir, lib.scriptFileName(stem, f.axis)), staging);
        let lengthMs = null;
        try { lengthMs = lib.scriptDurationMs(JSON.parse(fs.readFileSync(path.join(dir, lib.scriptFileName(stem, '')), 'utf8'))); } catch { /* unreadable script: download anyway */ }
        if (lengthMs !== null && lengthMs < lib.MIN_VIDEO_MS) {
          for (const f of sc.scripts) fs.rmSync(path.join(dir, lib.scriptFileName(stem, f.axis)), { force: true });
          removeIfEmpty(dir);
          removeIfEmpty(path.dirname(dir));
          rejectedVideos.recordRejection(userData, {
            topicId: c.id, video: sc.video.name, title: c.title, url: c.url, reason: 'short', lengthMs, at: new Date().toISOString(),
          });
          console.log(`${now()} – ${creator}/${stem}: ${Math.round(lengthMs / 1000)} s, under a minute; skipped`);
          short++;
          continue;
        }
        await download(api, sc.video, video, staging);
        fs.writeFileSync(path.join(dir, 'eroscripts.json'), JSON.stringify({
          topicId: c.id, url: c.url, title: c.title, author: c.author, tags: c.tags,
          video: sc.video.name, file: path.basename(video), scenes: c.scenes.length, at: new Date().toISOString(),
        }, null, 1));
        let note = '';
        if (!sc.axes.length) {
          const up = spawnSync(process.execPath, [path.join(REPO, 'scripts/upgrade-multi-axis.mjs'), dir], { cwd: REPO, encoding: 'utf8' });
          if (up.status === 0) { generated++; note = ' + generated axes'; } else note = ` (axis generation failed: ${(up.stderr || up.stdout).trim().split('\n').pop()})`;
        }
        const listed = flushPlaylist(userData, incoming, [video]);
        console.log(`${now()} ✓ ${creator}/${stem}${note}${listed ? '' : ' (playlist pending: app is open)'}`);
        ok++;
      } catch (err) {
        console.log(`${now()} ✗ ${creator}/${stem}: ${err.message}`);
        failed++;
      }
    }
  }
  return { ok, failed, generated, short };
}

async function fetchAll(opts, { api, userData, incoming }) {
  const file = path.join(userData, 'eroscripts-scout.json');
  if (!fs.existsSync(file)) throw new Error(`No scout list at ${file}; run npm run eroscripts:scout first.`);
  const { candidates } = JSON.parse(fs.readFileSync(file, 'utf8'));
  const queue = lib.downloadQueue(candidates).filter((c) => !opts.ids || opts.ids.has(c.id));
  const gb = queue.reduce((n, c) => n + c.scenes.reduce((m, sc) => m + (sc.video.size || 0), 0), 0) / 1e9;
  console.log(`Queue: ${queue.length} posts, ${queue.reduce((n, c) => n + c.scenes.length, 0)} videos, ${gb.toFixed(1)} GB → ${incoming}`);
  if (opts.dryRun) {
    for (const c of queue) console.log(`  ${c.axisType.padEnd(6)} ${c.id} ${c.title}`);
    return 0;
  }
  const staging = path.join(path.dirname(incoming), '_staging');
  for (let pass = 1; pass <= Math.max(1, opts.passes); pass++) {
    console.log(`Pass ${pass}/${Math.max(1, opts.passes)}`);
    const r = await fetchPass(queue, { api, userData, incoming, staging });
    console.log(`Pass ${pass}: ${r.ok} downloaded (${r.generated} with generated axes), ${r.short} under a minute, ${r.failed} failed`);
    if (!r.failed) return 0;
    if (pass < opts.passes) {
      console.log(`Waiting ${opts.wait / 60} min for host quotas`);
      await new Promise((resolve) => setTimeout(resolve, opts.wait * 1000));
    }
  }
  flushPlaylist(userData, incoming, []);
  return 2;
}

// ── Retitle ─────────────────────────────────────────────────────────────

/** Rename fetched scenes to `Artist - Title (metadata)` and move config.json's paths with them. */
function retitle(opts, { userData, incoming }) {
  if (!opts.dryRun && appRunning(userData)) {
    console.error('FunSync is running. Close it, then re-run --retitle: config.json holds the scene paths.');
    return 2;
  }
  const scenes = [];
  walk(incoming, 0, (full, e) => {
    if (e.name !== 'eroscripts.json') return;
    const dir = path.dirname(full);
    try {
      scenes.push({ dir, rec: JSON.parse(fs.readFileSync(full, 'utf8')), files: fs.readdirSync(dir).filter((n) => !n.startsWith('._')) });
    } catch { process.stderr.write(`! unreadable ${full}\n`); }
  });
  const moves = lib.retitlePlan(scenes, incoming, (p) => fs.existsSync(p));
  const rel = (p) => path.relative(incoming, p);
  for (const m of moves) console.log(`${rel(m.from)}\n  → ${rel(m.to)}`);
  console.log(`${moves.length} of ${scenes.length} scenes ${opts.dryRun ? 'would be renamed' : 'to rename'}`);
  if (opts.dryRun || !moves.length) return 0;

  const done = [];
  let failed = 0;
  for (const m of moves) {
    const renamed = [];
    try {
      for (const f of m.files) {
        renameWithSidecar(path.join(m.from, f.from), path.join(m.from, f.to));
        renamed.push(f);
      }
      if (path.resolve(m.to) !== path.resolve(m.from)) {
        fs.mkdirSync(path.dirname(m.to), { recursive: true });
        renameWithSidecar(m.from, m.to);
      }
    } catch (err) {
      for (const f of renamed.reverse()) {
        try { renameWithSidecar(path.join(m.from, f.to), path.join(m.from, f.from)); } catch { /* reported below */ }
      }
      console.log(`✗ ${rel(m.from)}: ${err.message}`);
      failed++;
      continue;
    }
    const sidecar = path.join(m.to, 'eroscripts.json');
    fs.writeFileSync(sidecar, JSON.stringify({ ...JSON.parse(fs.readFileSync(sidecar, 'utf8')), file: m.video }, null, 1));
    removeIfEmpty(path.dirname(m.from));
    const to = onDiskPath(m.to);
    done.push({ from: m.from, to, files: m.files.map((f) => ({ from: f.from, to: path.basename(onDiskPath(path.join(to, f.to))) })) });
  }

  if (done.length) {
    const { value, changed } = lib.remapPaths(readConfig(userData), done);
    writeConfig(userData, value);
    const pendingFile = path.join(userData, 'eroscripts-scout-pending.json');
    if (fs.existsSync(pendingFile)) {
      fs.writeFileSync(pendingFile, JSON.stringify(lib.remapPaths(JSON.parse(fs.readFileSync(pendingFile, 'utf8')), done).value, null, 1));
    }
    console.log(`Renamed ${done.length} scenes; ${changed} paths updated in config.json`);
  }
  return failed ? 1 : 0;
}

// ── Main ────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserDataDir();
  const config = readConfig(userData);
  const sources = librarySources(config, opts.incoming);
  const discovery = (config.settings?.library?.sources || []).find((s) => s.name === lib.DISCOVERY_NAME);
  const incoming = opts.incoming || discovery?.path || (sources[0] && path.join(path.dirname(sources[0].path), '_incoming'));
  if (!incoming) throw new Error('No library source to put _incoming next to; pass --incoming <dir>.');
  if (opts.retitle) return retitle(opts, { userData, incoming });
  const session = config.settings?.eroscripts?.session || {};
  if (!session.cookie) { console.error('Not logged in to EroScripts. Log in from the app (EroScripts panel), then re-run.'); return 3; }

  // The API module logs through electron-log; keep the CLI's output clean.
  const log = require('../electron/logger.js');
  log.transports.file.level = false;
  log.transports.console.level = false;
  const { EroScriptsAPI } = require('../electron/eroscripts-api.js');
  const api = new EroScriptsAPI();
  api.restoreSession(session.cookie, session.username);

  try {
    return opts.fetch ? await fetchAll(opts, { api, userData, incoming }) : await scan(opts, { api, userData, config, incoming });
  } catch (err) {
    if (err instanceof RateLimitedError) { console.error(err.message); return 2; }
    if (err instanceof SessionExpiredError) { console.error(err.message); return 3; }
    throw err;
  }
}

main().then((code) => process.exit(code), (err) => {
  console.error(`eroscripts:scout: ${err.message}`);
  process.exit(1);
});
