#!/usr/bin/env node
// npm run eroscripts:scout — find free EroScripts posts worth adding, then
// download them into a review folder and the app's "Discovery" playlist.
//
//   npm run eroscripts:scout                 scan new posts (+ a little backfill) → <userData>/eroscripts-scout.json
//   npm run eroscripts:scout -- --fetch      download the saved list
//   npm run eroscripts:scout -- --retitle    rename fetched scenes to `Artist - Title (metadata)`
//   npm run eroscripts:scout -- --recheck    judge saved candidates' sex by their first post (one-off)
//
// Rules and ranking: scripts/eroscripts-scout-lib.mjs (sex required for
// every post, favourites and picks too; monster, feet and 2D hentai posts
// rank lower). Scenes land in the library's EroScripts folder,
// `<library parent>/EroScripts/<creator>/<Artist - Title (metadata)>/`, with an
// `eroscripts.json` sidecar. Single-axis scenes get generated twist / roll / pitch companions
// (scripts/upgrade-multi-axis.mjs). Each finished video joins the Discovery
// playlist, and the EroScripts folder is a library source, so the
// scenes show in the library, the web remote and HereSphere. config.json is
// written only while the app is closed; otherwise the paths wait in
// `<userData>/eroscripts-scout-pending.json` for the next run.
//
// Exit codes: 0 done, 1 error, 2 rate limited / downloads left after the
// last pass / app running (--retitle), 3 not logged in / session expired.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
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
const { appRunning, applyMoves, dedupeAfterFetch, downloadFolder, flushPlaylist, librarySources, libraryState, libraryVideoCount, readConfig, removeIfEmpty, walk } = await import('./discovery-io.mjs');
const { createPacer, defaultUserDataDir, parseInterval, RateLimitedError, SessionExpiredError } = await import('./eroscripts-sync-lib.mjs');
const live = await import('./scout-live.mjs');
const prefs = await import('../renderer/js/content-preferences.js');
const require = createRequire(import.meta.url);
const rejectedVideos = require('../electron/rejected-videos.js');
const REPO = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const FOLDER = 'EroScripts';

const USAGE = `Usage: npm run eroscripts:scout -- [options]
Scan (default; only posts not judged yet, see <userData>/eroscripts-scout-state.json):
  --pages N              most pages of new posts on the multi-axis feed (30 posts each, default 6)
  --anim-pages N         most pages of new posts on each animation feed (${prefs.FEED_TAGS.filter((f) => f !== 'multi-axis').join(', ')}; default 3)
  --backfill N           older pages to read per feed, continuing from the last run (default 2; 0 = none)
  --feeds a,b            only these feeds (${prefs.FEED_TAGS.join(', ')}; default all)
  --since YYYY-MM-DD     backfill only down to posts last active on that date; a feed that gets there
                         isn't backfilled again unless a later run passes an earlier date
  --full                 judge every listed post again, not only new ones
  --top N                candidates to print (default 40)
Pick:
  --add 1,2,3            judge these post ids and put them on the list: only the exclusions, the sex rule,
                         posts the library has or the user turned down, and library duplicates count (the
                         animation and multi-axis rules don't; single-axis scenes get generated axes)
Download:
  --fetch                download the saved list (multi-axis first), add each scene to Discovery;
                         skips videos under ${lib.MIN_VIDEO_LABEL}, scenes rejected in the app (${prefs.DELETE_AT_OR_BELOW} stars or lower)
                         and waiting posts that break the content rules
  --ids 1,2,3            only these post ids
  --passes N             repeat the queue until no failures, at most N times (default 1)
  --wait 1h              pause between passes (default 1h; also 90m, 3d)
  --dry-run              list what would be downloaded (or, with --retitle, renamed)
Rename:
  --retitle              rename fetched scenes to "Artist - Title (metadata)" from their posts,
                         and the paths config.json keeps for them (app must be closed)
Recheck (one-off):
  --recheck              for saved candidates that fail only the sex rule and were saved without its
                         evidence, read the first post: sex there keeps them (sexual: text; monster / feet
                         there rank lower), excluded words there drop them for good; the rest stay skipped
Both:
  --user-data <dir>      app data folder (default: the desktop app's)
  --incoming <dir>       download folder (default: the EroScripts library source, else <library parent>/EroScripts)`;

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
      case '--recheck': opts.recheck = true; break;
      case '--dry-run': opts.dryRun = true; break;
      case '--pages': opts.pages = Number(value(i, a)); i++; break;
      case '--anim-pages': opts.animPages = Number(value(i, a)); i++; break;
      case '--backfill': opts.backfill = Number(value(i, a)); i++; break;
      case '--full': opts.full = true; break;
      case '--top': opts.top = Number(value(i, a)); i++; break;
      case '--passes': opts.passes = Number(value(i, a)); i++; break;
      case '--wait': opts.wait = parseInterval(value(i, a)); i++; break;
      case '--ids': opts.ids = new Set(value(i, a).split(',').map(Number)); i++; break;
      case '--add': opts.add = [...new Set(value(i, a).split(',').map((s) => Number(s.trim())).filter((n) => n > 0))]; i++; break;
      case '--feeds': opts.feeds = value(i, a).split(',').map((s) => s.trim()).filter(Boolean); i++; break;
      case '--since': opts.since = value(i, a); i++; break;
      case '--user-data': opts.userData = path.resolve(value(i, a)); i++; break;
      case '--incoming': opts.incoming = path.resolve(value(i, a)); i++; break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  for (const k of ['pages', 'animPages', 'backfill', 'top', 'passes']) if (!(opts[k] >= 0)) throw new Error(`--${k} must be 0 or more`);
  const unknownFeed = opts.feeds?.find((f) => !prefs.FEED_TAGS.includes(f));
  if (unknownFeed !== undefined) throw new Error(`Unknown feed "${unknownFeed}" (feeds: ${prefs.FEED_TAGS.join(', ')})`);
  if (opts.feeds && !opts.feeds.length) throw new Error('--feeds needs at least one feed');
  if (opts.since !== undefined && !(/^\d{4}-\d{2}-\d{2}$/.test(opts.since) && Number.isFinite(Date.parse(opts.since)))) throw new Error('--since must be a date like 2025-01-01');
  if (!opts.wait) throw new Error('--wait must be an interval like 1h, 90m or 3d');
  return opts;
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

/** Write a file through a temporary one, so a reader never sees half of it. */
function writeAtomic(target, text) {
  const tmp = `${target}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, text);
  fs.renameSync(tmp, target);
}

function writeScoutState(userData, state) {
  writeAtomic(statePath(userData), JSON.stringify(state));
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

/** What screening needs: the library, the posts it already links or the user turned down, favourites. */
function scoutContext(config, userData, incoming) {
  const roots = [...librarySources(config, incoming).map((s) => s.path), incoming].filter((p) => fs.existsSync(p));
  const library = libraryState(roots);
  const knownTopics = new Set(Object.values(config.settings?.library?.scriptTags || {}).map((r) => r?.topicId).filter((id) => id != null));
  try {
    const links = require('../electron/eroscripts-links.js').readLinksFile(userData);
    for (const l of Object.values(links?.links || {})) if (l.topicId != null) knownTopics.add(l.topicId);
    for (const l of Object.values(links?.links || {})) if (l.creator) library.creators.add(l.creator.toLowerCase());
  } catch { /* no links file yet */ }
  // Posts the user turned down in the app (2 stars or lower; formerly the No thank you playlist);
  // a short video or an extra copy of one (dedupe) doesn't turn down the post.
  const turnedDown = new Set(rejectedVideos.readRejected(userData).entries.filter((e) => e.reason !== 'short' && e.reason !== 'duplicate' && e.topicId != null).map((e) => e.topicId));
  for (const id of turnedDown) knownTopics.add(id);
  return { library, knownTopics, turnedDown, favourites: prefs.FAVOURITES };
}

/** A judged post as a saved candidate. */
function makeCandidate({ t, s, author, artistInLibrary, favourite, liveQuota, hosts, scenes, first, picked = false }) {
  const preferred = t.tags.filter((x) => prefs.PREFERRED_TAGS.has(x));
  const axisType = scenes.some((sc) => sc.axes.length) ? 'multi' : 'single';
  const c = {
    id: t.id, title: t.title, url: `https://discuss.eroscripts.com/t/${t.id}`, author,
    tags: t.tags, likes: t.likes,
    vr: s.vr, music: s.music, preferred, axisType, artistInLibrary, hosts, live: s.live, favourite, liveQuota, sexual: first.sexual,
    ...(first.monster ? { monster: true } : {}), ...(first.feet ? { feet: true } : {}), ...(picked ? { picked } : {}),
    scenes,
  };
  return { ...c, score: lib.candidateScore(c) };
}

async function scan(opts, { api, userData, config, incoming }) {
  const pacer = createPacer();
  const { library, knownTopics, turnedDown, favourites } = scoutContext(config, userData, incoming);
  const state = readScoutState(userData);
  const rejected = { known: 0, excluded: 0, blocked: 0, 'not-sexual': 0, 'in-library': 0, 'no-video': 0, 'single-axis': 0 };
  const fresh = [];
  let listed = 0;
  let judged = 0;
  try {
    const topics = new Map();
    let pages = 0;
    const carried = lib.pendingTopics(state).length;
    for (const tag of opts.feeds || prefs.FEED_TAGS) {
      const maxNew = tag === 'multi-axis' ? opts.pages : opts.animPages;
      const r = await lib.listFeed(state, tag, feedPage(api, pacer), { maxNew, backfill: opts.backfill, full: opts.full, since: opts.since, keepPending: true });
      pages += r.pagesRead;
      for (const t of r.topics) topics.set(t.id, t);
    }
    listed = topics.size;
    // Everything listed now or by an earlier run that stopped before judging it.
    const todo = opts.full ? [...topics.values()] : lib.pendingTopics(state);
    const bumped = todo.filter((t) => lib.seenTopic(state, t.id)).length;
    console.log(`Listed ${listed} posts on ${pages} pages; ${todo.length} to judge${carried ? ` (${carried} left from an earlier run)` : ''}${bumped && !opts.full ? `, ${bumped} of them bumped since their verdict` : ''}`);

    const creators = [...library.creators];
    for (const t of todo) {
      const verdict = (v) => { lib.markTopic(state, t.id, v); judged++; if (v !== 'candidate') rejected[v]++; };
      const s = lib.screenTopic(t, { knownTopics, creators, sceneNames: library.sceneNames, favourites });
      if (!s.ok) { verdict(s.reason); continue; }
      await pacer.wait();
      const data = await api.getTopicData(t.id);
      const op = data?.post_stream?.posts?.[0];
      if (!op) { process.stderr.write(`! ${t.id} unreadable\n`); continue; }
      if (lib.isBlockedCreator({ author: op.username })) { verdict('blocked'); continue; }
      const cooked = op.cooked || '';
      const text = cooked.replace(/<[^>]+>/g, ' ');
      const first = lib.judgeFirstPost(t, text);
      if (!first.ok) { verdict(first.reason); continue; }
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
      const favourite = s.favourite || live.favouriteIn([author], favourites);
      const liveQuota = lib.liveOnQuota({ live: s.live, favourite, music: s.music, artist: s.artist });
      const scenes = lib.wantedScenes({ scenes: plan.scenes });
      if (!scenes.length) { verdict('single-axis'); continue; }
      fresh.push(makeCandidate({ t, s, author, artistInLibrary, favourite, liveQuota, hosts, scenes, first }));
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
    const tags = [c.vr && 'VR', ...c.preferred, c.music && 'music', c.favourite && `favourite: ${c.favourite}`, c.liveQuota && 'live (quota)'].filter(Boolean).join(', ');
    console.log(`| ${i + 1} | ${freshIds.has(c.id) ? '✓' : ''} | [${c.title.replace(/[|[\]]/g, ' ').slice(0, 60)}](${c.url}) | ${axis} | ${tags} | ${c.scenes.length} | ${gb(c).toFixed(2)} | ${[...new Set(c.scenes.map((sc) => sc.video.host))].join('+')} |`);
  });
  const multi = candidates.filter((c) => c.axisType === 'multi').length;
  const feedStatus = (f) => (f?.exhausted ? 'done' : f?.doneSince ? `done to ${f.doneSince} (p${f.backfillPage})` : `p${f?.backfillPage ?? 0}`);
  const backfill = prefs.FEED_TAGS.map((tag) => `${tag} ${feedStatus(state.feeds[tag])}`).join(', ');
  const left = Object.keys(state.pending || {}).length;
  console.log(`\nListed ${listed} · judged ${judged} · rejected ${JSON.stringify(rejected)} · new ${fresh.length} · waiting to download ${candidates.length} (${multi} multi-axis, ${candidates.length - multi} single-axis), ${candidates.reduce((n, c) => n + c.scenes.length, 0)} videos, ${candidates.reduce((n, c) => n + gb(c), 0).toFixed(1)} GB`);
  console.log(`Backfill: ${backfill} · ${Object.keys(state.topics).length} posts judged so far${left ? ` · ${left} listed, not judged yet (next run goes on with them)` : ''}`);
  console.log(`Saved: ${out}`);
  return 0;
}

// ── Pick ────────────────────────────────────────────────────────────────

/** `--add`: judge posts the user picked by id and put them on the list (lib.screenPick). */
async function addPicked(opts, { api, userData, config, incoming }) {
  const pacer = createPacer();
  const { library, knownTopics, turnedDown, favourites } = scoutContext(config, userData, incoming);
  const state = readScoutState(userData);
  const fresh = [];
  try {
    for (const id of opts.add) {
      await pacer.wait();
      const data = await api.getTopicData(id);
      const op = data?.post_stream?.posts?.[0];
      if (!op) { console.log(`✗ ${id}: unreadable`); continue; }
      const t = { id, title: data.title || '', tags: (data.tags || []).map((x) => (typeof x === 'string' ? x : x?.name || '')).filter(Boolean), likes: data.like_count || 0 };
      if (data.category_id !== lib.FREE_SCRIPTS_CATEGORY) { console.log(`✗ ${id} ${t.title}: not in Free Scripts`); continue; }
      const s = lib.screenPick(t, { knownTopics, creators: [...library.creators], sceneNames: library.sceneNames, favourites });
      if (!s.ok) { console.log(`✗ ${id} ${t.title}: ${s.reason}`); continue; }
      if (lib.isBlockedCreator({ author: op.username })) { console.log(`✗ ${id} ${t.title}: blocked creator`); continue; }
      const cooked = op.cooked || '';
      const first = lib.judgeFirstPost(t, cooked.replace(/<[^>]+>/g, ' '));
      if (!first.ok) { console.log(`✗ ${id} ${t.title}: ${first.reason}`); continue; }
      let files, hosts;
      try { ({ files, hosts } = await topicFiles(api, pacer, id, cooked)); } catch (err) {
        if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
        console.log(`✗ ${id} ${t.title}: ${err.message}`);
        continue;
      }
      const plan = lib.planScenes({
        scripts: files.filter((f) => /\.funscript$/i.test(f.name)),
        videos: files.filter((f) => lib.isVideoName(f.name)),
        vr: s.vr,
        libraryVideos: library.videos,
      });
      if (!plan.scenes.length) { console.log(`✗ ${id} ${t.title}: ${plan.inLibrary > 0 ? 'in-library' : 'no-video'}`); continue; }
      const author = op.username;
      const artistInLibrary = s.artist || (library.creators.has(author.toLowerCase()) ? author : null);
      const favourite = s.favourite || live.favouriteIn([author], favourites);
      fresh.push(makeCandidate({ t, s, author, artistInLibrary, favourite, liveQuota: false, hosts, scenes: plan.scenes, first, picked: true }));
      lib.markTopic(state, id, 'candidate');
      console.log(`✓ ${id} ${t.title} (${plan.scenes.length} video${plan.scenes.length > 1 ? 's' : ''}, ${plan.scenes.some((sc) => sc.axes.length) ? 'multi-axis' : 'single-axis'})`);
    }
  } finally {
    writeScoutState(userData, state);
  }
  const out = path.join(userData, 'eroscripts-scout.json');
  let saved = { candidates: [] };
  try { saved = JSON.parse(fs.readFileSync(out, 'utf8')); } catch { /* no list yet */ }
  const done = fetchedScenes(incoming, userData);
  const candidates = lib.mergeCandidates(saved.candidates || [], fresh, (id, video) => done.has(lib.fetchedKey(id, video)), turnedDown);
  fs.writeFileSync(out, JSON.stringify({ ...saved, generatedAt: new Date().toISOString(), candidates }, null, 1));
  console.log(`\nAdded ${fresh.length} of ${opts.add.length}.${fresh.length ? ` Download them with: npm run eroscripts:scout -- --fetch --ids ${fresh.map((c) => c.id).join(',')}` : ''}`);
  return 0;
}

// ── Recheck ─────────────────────────────────────────────────────────────

/**
 * `--recheck`: saved candidates that fail only the sex rule and carry no
 * evidence (saved before it was kept) are judged again with their first
 * post (lib.recheckWithText). The list is re-read before writing, so only
 * the judged candidates change.
 */
async function recheck({ api, userData }) {
  const file = path.join(userData, 'eroscripts-scout.json');
  if (!fs.existsSync(file)) throw new Error(`No scout list at ${file}; run npm run eroscripts:scout first.`);
  const todo = (JSON.parse(fs.readFileSync(file, 'utf8')).candidates || []).filter(lib.needsSexRecheck);
  console.log(`Recheck: ${todo.length} waiting posts fail only the sex rule by tags and title`);
  if (!todo.length) return 0;
  if (!(await api.validateSession()).valid) throw new SessionExpiredError();
  const pacer = createPacer();
  const changed = new Map();
  const counts = { text: 0, 'not-sexual': 0, excluded: 0, unreadable: 0 };
  const examples = { text: [], 'not-sexual': [], excluded: [] };
  try {
    for (const c of todo) {
      await pacer.wait();
      const data = await api.getTopicData(c.id);
      if (data?.error_type === 'rate_limit') throw new RateLimitedError();
      const op = data?.post_stream?.posts?.[0];
      if (!op) {
        if (!(await api.validateSession()).valid) throw new SessionExpiredError();
        counts.unreadable++;
        process.stderr.write(`! ${c.id} unreadable\n`);
        continue;
      }
      const r = lib.recheckWithText(c, (op.cooked || '').replace(/<[^>]+>/g, ' '));
      counts[r.outcome]++;
      examples[r.outcome].push(`${c.title}${r.candidate.contentBreak ? ` (${r.candidate.contentBreak})` : ''}`);
      if (r.candidate !== c) changed.set(c.id, r.candidate);
    }
  } finally {
    // Progress survives a rate limit or an expired session.
    if (changed.size) {
      const saved = JSON.parse(fs.readFileSync(file, 'utf8'));
      saved.candidates = (saved.candidates || []).map((c) => {
        const r = changed.get(c.id);
        if (!r) return c;
        const { contentBreak, ...rest } = c;
        const judged = Object.fromEntries(['sexual', 'contentBreak', 'monster', 'feet'].filter((k) => r[k] !== undefined).map((k) => [k, r[k]]));
        return { ...rest, ...judged };
      });
      writeAtomic(file, JSON.stringify(saved, null, 1));
    }
    console.log(`\nKept by first-post text ${counts.text} · still not sexual ${counts['not-sexual']} · newly excluded ${counts.excluded}${counts.unreadable ? ` · unreadable ${counts.unreadable}` : ''}`);
    for (const [k, label] of [['text', 'Kept'], ['excluded', 'Excluded'], ['not-sexual', 'Still not sexual']]) {
      if (examples[k].length) console.log(`${label}: ${examples[k].slice(0, 8).join(' · ')}${examples[k].length > 8 ? ' …' : ''}`);
    }
  }
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

/**
 * Download `file` to `dest` through `<staging>/<name>.part`. Every attempt
 * starts from scratch, so a failed one deletes its `.part` rather than leave
 * it behind.
 */
async function download(api, file, dest, staging) {
  if (fs.existsSync(dest)) return 'have';
  fs.mkdirSync(staging, { recursive: true });
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = path.join(staging, `${path.basename(dest)}.part`);
  activePart = tmp;
  try {
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
    if (file.size && size !== file.size) throw new Error(`size ${size} ≠ ${file.size}`);
    fs.renameSync(tmp, dest);
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    throw err;
  } finally {
    activePart = null;
  }
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

/** One pass over the queue; downloads at most `room` videos (the library cap). */
async function fetchPass(queue, { api, userData, incoming, staging, room }) {
  const done = fetchedScenes(incoming, userData);
  let ok = 0; let failed = 0; let generated = 0; let short = 0; let waiting = 0; let capped = false;
  const now = () => new Date().toISOString().slice(11, 19);
  for (const c of queue) {
    if (capped) break;
    for (const sc of c.scenes) {
      if (done.has(lib.fetchedKey(c.id, sc.video.name))) continue;
      if (ok >= room) { capped = true; break; }
      // Live action by a non-favourite: one in every LIVE_QUOTA_EVERY downloads (both scouts).
      if (c.liveQuota && !live.liveAllowed(live.readQuota(userData))) { waiting++; continue; }
      const creator = lib.creatorFolder(c.title, c.author);
      const stem = lib.sceneTitle({ title: c.title, author: c.author, video: sc.video.name, shared: c.scenes.length > 1 });
      const dir = path.join(incoming, creator, stem);
      const video = path.join(dir, stem + path.extname(sc.video.name).toLowerCase());
      // Another post already brought this scene (same folder and video file): leave the
      // folder alone. Its scripts, sidecar and length were judged when it came, so this
      // post's files and the current length rule don't apply to it.
      if (fs.existsSync(video)) { console.log(`${now()} = ${creator}/${stem}: already in Discovery from another post; left as is`); continue; }
      try {
        for (const f of sc.scripts) await download(api, f, path.join(dir, lib.scriptFileName(stem, f.axis)), staging);
        let lengthMs = null;
        try { lengthMs = lib.scriptDurationMs(JSON.parse(fs.readFileSync(path.join(dir, lib.scriptFileName(stem, '')), 'utf8'))); } catch { /* unreadable script: download anyway */ }
        if (lengthMs !== null && lengthMs < prefs.MIN_VIDEO_MS) {
          for (const f of sc.scripts) fs.rmSync(path.join(dir, lib.scriptFileName(stem, f.axis)), { force: true });
          removeIfEmpty(dir);
          removeIfEmpty(path.dirname(dir));
          rejectedVideos.recordRejection(userData, {
            topicId: c.id, video: sc.video.name, title: c.title, url: c.url, reason: 'short', lengthMs, at: new Date().toISOString(),
          });
          console.log(`${now()} – ${creator}/${stem}: ${Math.round(lengthMs / 1000)} s, under ${lib.MIN_VIDEO_LABEL}; skipped`);
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
        const listed = flushPlaylist(userData, { name: FOLDER, path: incoming }, [video]);
        live.recordDownload(userData, !!c.liveQuota);
        const kind = c.liveQuota ? ' [live quota]' : c.favourite && c.live ? ` [favourite: ${c.favourite}]` : '';
        console.log(`${now()} ✓ ${creator}/${stem}${kind}${note}${listed ? '' : ' (playlist pending: app is open)'}`);
        ok++;
      } catch (err) {
        console.log(`${now()} ✗ ${creator}/${stem}: ${err.message}`);
        failed++;
      }
    }
  }
  return { ok, failed, generated, short, waiting, capped };
}

async function fetchAll(opts, { api, userData, incoming }) {
  const file = path.join(userData, 'eroscripts-scout.json');
  if (!fs.existsSync(file)) throw new Error(`No scout list at ${file}; run npm run eroscripts:scout first.`);
  const { candidates } = JSON.parse(fs.readFileSync(file, 'utf8'));
  const breaks = {};
  for (const c of candidates) {
    const rule = lib.contentRuleBreak(c);
    if (rule) breaks[rule] = (breaks[rule] || 0) + 1;
  }
  const dropped = Object.values(breaks).reduce((n, k) => n + k, 0);
  if (dropped) console.log(`Dropped ${dropped} waiting posts that break the content rules ${JSON.stringify(breaks)}`);
  const queue = lib.downloadQueue(candidates).filter((c) => !opts.ids || opts.ids.has(c.id));
  const gb = queue.reduce((n, c) => n + c.scenes.reduce((m, sc) => m + (sc.video.size || 0), 0), 0) / 1e9;
  console.log(`Queue: ${queue.length} posts, ${queue.reduce((n, c) => n + c.scenes.length, 0)} videos, ${gb.toFixed(1)} GB → ${incoming}`);
  if (opts.dryRun) {
    for (const c of queue) console.log(`  ${c.axisType.padEnd(6)} ${c.id} ${c.title}`);
    return 0;
  }
  const staging = path.join(path.dirname(incoming), '_staging');
  for (let pass = 1; pass <= Math.max(1, opts.passes); pass++) {
    const held = libraryVideoCount(readConfig(userData));
    if (held >= prefs.LIBRARY_MAX_VIDEOS) {
      console.log(`Library holds ${held} videos, the cap is ${prefs.LIBRARY_MAX_VIDEOS} (LIBRARY_MAX_VIDEOS): nothing more downloaded. Delete scenes or raise the cap.`);
      break;
    }
    console.log(`Pass ${pass}/${Math.max(1, opts.passes)} (library ${held}/${prefs.LIBRARY_MAX_VIDEOS} videos)`);
    const r = await fetchPass(queue, { api, userData, incoming, staging, room: prefs.LIBRARY_MAX_VIDEOS - held });
    console.log(`Pass ${pass}: ${r.ok} downloaded (${r.generated} with generated axes), ${r.short} under ${lib.MIN_VIDEO_LABEL}, ${r.failed} failed${r.waiting ? `, ${r.waiting} live waiting for the quota (one in every ${prefs.LIVE_QUOTA_EVERY} downloads)` : ''}${r.capped ? `; stopped at the library cap (${prefs.LIBRARY_MAX_VIDEOS} videos)` : ''}`);
    if (r.ok) dedupeAfterFetch(userData);
    if (r.capped) break;
    if (!r.failed) return 0;
    if (pass < opts.passes) {
      console.log(`Waiting ${opts.wait / 60} min for host quotas`);
      await new Promise((resolve) => setTimeout(resolve, opts.wait * 1000));
    }
  }
  flushPlaylist(userData, { name: FOLDER, path: incoming }, []);
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
  return applyMoves(moves, { userData, incoming, sidecar: 'eroscripts.json' });
}

// ── Main ────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserDataDir();
  const config = readConfig(userData);
  const incoming = downloadFolder(config, FOLDER, opts.incoming).path;
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
    if (opts.add) return await addPicked(opts, { api, userData, config, incoming });
    if (opts.recheck) return await recheck({ api, userData });
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
