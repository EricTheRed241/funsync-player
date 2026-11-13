#!/usr/bin/env node
// npm run eroscripts:scout — find free EroScripts posts worth adding, then
// download them into the library's EroScripts folder.
//
//   npm run eroscripts:scout                 scan new posts (+ a little backfill) → <userData>/eroscripts-scout.json
//   npm run eroscripts:scout -- --fetch      download the saved list
//   npm run eroscripts:scout -- --retitle    rename fetched scenes to `Artist - Title (metadata)`
//   npm run eroscripts:scout -- --manual-list  write ~/Downloads/download-for-fs.txt: files to download by hand
//   npm run eroscripts:scout -- --status     backfill progress and the waiting list, offline
//   npm run eroscripts:scout -- --titles     title, author, dates, image for posts judged before verdicts kept them
//   npm run eroscripts:scout -- --daily      scan, then download the waiting multi-axis posts scoring AUTO_FETCH_MIN_SCORE+
//   npm run vrbangers:scout                  the same, VRBangers posts only (--vrbangers): vrbangers.com
//                                            scenes come from the paid account (scripts/vrbangers-host.mjs)
//
// Rules and ranking: scripts/eroscripts-scout-lib.mjs (sex required for
// every post, favourites and picks too; monster, feet and 2D hentai posts
// rank lower). Scenes land in the library's EroScripts folder,
// `<library parent>/EroScripts/<creator>/<Artist - Title (metadata)>/`, with an
// `eroscripts.json` sidecar. Single-axis scenes get generated twist / roll / pitch companions
// (scripts/upgrade-multi-axis.mjs). The EroScripts folder is a library
// source, so the scenes show in the library, the web remote and HereSphere;
// config.json is written (to add the source) only while the app is closed.
//
// Exit codes: 0 done, 1 error, 2 rate limited / downloads left after the
// last pass / app running (--retitle) / another scout run going, 3 not logged
// in / session expired.

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
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
const { appRunning, applyMoves, dedupeAfterFetch, downloadFolder, ensureLibrarySource, librarySources, libraryState, libraryVideoCount, readConfig, removeIfEmpty, walk } = await import('./discovery-io.mjs');
const { createPacer, parseInterval, RateLimitedError, SessionExpiredError } = await import('./eroscripts-sync-lib.mjs');
const { defaultUserDataDir, describeHolder, takePidLock } = await import('./host-env.mjs');
const live = await import('./scout-live.mjs');
const streams = await import('./stream-hosts.mjs');
const vrbHost = await import('./vrbangers-host.mjs');
const { DEFAULT_CREDENTIALS: VRB_CREDENTIALS, LoginError, browserOptions } = await import('./vrbangers-browser.mjs');
const { redactUrl } = await import('./vrbangers-lib.mjs');
const prefs = await import('../renderer/js/content-preferences.js');
const require = createRequire(import.meta.url);
const rejectedVideos = require('../electron/rejected-videos.js');
const REPO = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
// One per run: every rule34video request goes through its pacing (≥ 400 ms apart, 60 s after a 429).
const rule34video = streams.createRule34video();
// Set in main (it needs the data folder and the credentials file): vrbangers.com reads, and Chrome for --fetch.
let vrbangers = null;
const FOLDER = 'EroScripts';
const DOWNLOADS = path.join(os.homedir(), 'Downloads');

const USAGE = `Usage: npm run eroscripts:scout -- [options]
       npm run vrbangers:scout -- [options]   (the same, VRBangers posts only: --vrbangers)
Scan (default; only posts not judged yet, see <userData>/eroscripts-scout-state.json):
  --pages N              most pages of new posts on the multi-axis feed (30 posts each, default 6)
  --anim-pages N         most pages of new posts on each animation feed (${prefs.FEED_TAGS.filter((f) => f !== 'multi-axis').join(', ')}; default 3)
  --backfill N           older pages to read per feed, continuing from the last run (default 2; 0 = none)
  --feeds a,b            only these feeds (${prefs.FEED_TAGS.join(', ')}; default all), or the search feed
                         ${Object.keys(prefs.SEARCH_FEEDS).join(', ')} (Free Scripts first posts naming it; not in the default)
  --since YYYY-MM-DD     backfill only down to posts last active on that date; a feed that gets there
                         isn't backfilled again unless a later run passes an earlier date
  --full                 judge every listed post again, not only new ones
  --rejudge v1,v2        no listing: judge again every post with one of these verdicts (e.g. no-video) that was
                         judged under older rules (feeds only judge those again when they list them)
  --top N                candidates to print (default 40)
  --vrbangers            VRBangers posts only: the scan reads the vrbangers feed (unless --feeds is given) and
                         prints only posts with a vrbangers.com video; --fetch and --manual-list take only those
Pick:
  --add 1,2,3            judge these post ids and put them on the list: only the exclusions, the sex rule,
                         posts the library has or the user turned down, and library duplicates count (the
                         live-not-POV rule doesn't); single-axis scenes get generated axes
Download:
  --fetch                download the saved list (multi-axis first) into the EroScripts library folder;
                         skips videos under ${lib.MIN_VIDEO_LABEL}, scenes rejected in the app (${prefs.DELETE_AT_OR_BELOW} stars or lower)
                         and waiting posts that break the content rules
  --ids 1,2,3            only these post ids
  --auto                 only posts with hand-made multi-axis scripts scoring ${prefs.AUTO_FETCH_MIN_SCORE}+ (AUTO_FETCH_MIN_SCORE)
  --passes N             repeat the queue until no failures, at most N times (default 1)
  --wait 1h              pause between passes (default 1h; also 90m, 3d)
  --dry-run              list what would be downloaded (or, with --retitle, renamed)
                         Files saved by hand (any host) are taken from --manual-dir or ~/Downloads first.
                         Videos on rule34video.com are read from their pages (paced, ≥ 400 ms apart) and must
                         fit the main script's length (checked again with ffprobe; needs ffprobe on PATH).
                         vrbangers.com scenes come from the paid account through headless Google Chrome (logged in
                         once per pass; ≤ 8K, the planned file); with --dry-run the signed link is resolved and
                         probed for 1 byte instead
  --vrb-credentials <f>  VRBangers account file (default: $FUNSYNC_VRB_CREDENTIALS, else ${VRB_CREDENTIALS}; lines "Username: …", "password: …")
By hand (pixeldrain's hotlink protection needs a CAPTCHA in a browser):
  --manual-list          write ~/Downloads/download-for-fs.txt: the waiting posts' pixeldrain files with
                         their pages and instructions (with --ids: only those posts); no login needed
  --manual-dir <dir>     where files downloaded by hand are saved (default ~/Downloads/for-fs)
Rename:
  --retitle              rename fetched scenes to "Artist - Title (metadata)" from their posts,
                         and the paths config.json keeps for them (app must be closed);
                         with --ids, only those posts' scenes
Daily (run once a day by the watchdog, see README):
  --daily                scan, then --fetch --auto (takes --passes, --wait); writes <userData>/eroscripts-scout-daily.json
Status:
  --status               print the scan state without going online: backfill per feed (page and oldest last
                         activity reached), verdicts, posts judged under older rules, the waiting list
  --titles               fill in title, author, dates and image of judged posts that lack them (judged before
                         verdicts kept them), newest first, one read per post at the usual pace; verdicts stay.
                         For npm run scout:report. A stopped run goes on next time
Both:
  --user-data <dir>      app data folder (default: $FUNSYNC_USER_DATA, else the desktop app's)
  --incoming <dir>       download folder (default: the EroScripts library source, else <library parent>/EroScripts)`;

function parseArgs(argv) {
  const opts = { pages: 6, animPages: 3, backfill: 2, top: 40, passes: 1, wait: 3600, fetch: false, dryRun: false, full: false, manualDir: path.join(DOWNLOADS, 'for-fs'), vrbCredentials: browserOptions().credentialsFile };
  const value = (i, flag) => {
    if (i + 1 >= argv.length) throw new Error(`${flag} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--fetch': opts.fetch = true; break;
      case '--auto': opts.auto = true; break;
      case '--daily': opts.daily = true; break;
      case '--status': opts.status = true; break;
      case '--titles': opts.titles = true; break;
      case '--rejudge': opts.rejudge = value(i, a).split(',').map((s) => s.trim()).filter(Boolean); i++; break;
      case '--retitle': opts.retitle = true; break;
      case '--dry-run': opts.dryRun = true; break;
      case '--manual-list': opts.manualList = true; break;
      case '--manual-dir': opts.manualDir = path.resolve(value(i, a)); i++; break;
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
      case '--vrbangers': opts.vrbangers = true; break;
      case '--vrb-credentials': opts.vrbCredentials = path.resolve(value(i, a)); i++; break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  for (const k of ['pages', 'animPages', 'backfill', 'top', 'passes']) if (!(opts[k] >= 0)) throw new Error(`--${k} must be 0 or more`);
  const unknownFeed = opts.feeds?.find((f) => !lib.ALL_FEEDS.includes(f));
  if (unknownFeed !== undefined) throw new Error(`Unknown feed "${unknownFeed}" (feeds: ${lib.ALL_FEEDS.join(', ')})`);
  if (opts.feeds && !opts.feeds.length) throw new Error('--feeds needs at least one feed');
  if (opts.vrbangers && !opts.feeds) opts.feeds = ['vrbangers'];
  if (opts.since !== undefined && !(/^\d{4}-\d{2}-\d{2}$/.test(opts.since) && Number.isFinite(Date.parse(opts.since)))) throw new Error('--since must be a date like 2025-01-01');
  if (!opts.wait) throw new Error('--wait must be an interval like 1h, 90m or 3d');
  if (opts.auto && !opts.fetch) throw new Error('--auto goes with --fetch');
  if (opts.rejudge && !opts.rejudge.length) throw new Error('--rejudge needs at least one verdict');
  return opts;
}

// ── Hosts ───────────────────────────────────────────────────────────────

async function megaFiles(url) {
  if (!lib.megaKeyOk(url)) throw new Error(`MEGA link with a broken key: ${url}`);
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

/**
 * Every script and video a post offers: author's forum uploads first, then MEGA, pixeldrain, rule34video pages
 * (one pseudo file per quality) and VRBangers scenes (one pseudo file each, the best quality within the VR caps:
 * a vrbangers.com/video link, a `VRBANGERS_<folder>…` file name, else the parts of a title naming VRBangers).
 * `hostDown`: a rule34video page couldn't be read for a reason other than being gone (403, 429, 5xx, timeout),
 * so a post without a video now may have one once the site answers again.
 */
async function topicFiles(api, pacer, topicId, cooked, title) {
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
  const r34 = streams.rule34videoLinks(hrefs);
  if (r34.length) hosts.add(streams.RULE34VIDEO_HOST);
  let hostDown = false;
  for (const u of r34) {
    try { files.push(...await rule34video.files(u)); } catch (err) {
      hosts.add('rule34video (unreadable)');
      if (err.status !== 404 && err.status !== 410) hostDown = true;
      process.stderr.write(`! ${topicId} ${err.message}\n`);
    }
  }
  const leads = vrbHost.vrbangersLeads({ hrefs, names: files.map((f) => f.name), title });
  if (leads.length) {
    const { files: scenes, missed } = await vrbangers.filesFor(leads);
    if (scenes.length) hosts.add(vrbHost.VRBANGERS_HOST);
    files.push(...scenes);
    for (const { lead, why } of missed) {
      if (lead.from !== 'title') process.stderr.write(`! ${topicId} vrbangers ${lead.from} ${lead.folder || lead.slugs[0]}: ${why}\n`);
    }
  }
  const byName = new Map();
  for (const f of files) if (!byName.has(f.name)) byName.set(f.name, f);
  return { files: [...byName.values()], hosts: [...hosts], hostDown };
}

/** A script file's contents (forum, MEGA or pixeldrain), or null when it can't be read; forum errors become exceptions. */
async function scriptBytes(api, pacer, file) {
  if (file.host === 'forum') {
    await pacer.wait();
    const r = await api.fetchScript(file.url);
    if (r.rateLimited) throw new RateLimitedError();
    if (r.authExpired) throw new SessionExpiredError();
    return r.success ? r.bytes : null;
  }
  if (file.host === 'mega') {
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
    if (!node) return null;
    const chunks = [];
    for await (const chunk of node.download({})) chunks.push(chunk);
    return Buffer.concat(chunks);
  }
  if (file.host === 'pixeldrain') {
    const resp = await fetch(file.url, { headers: { 'User-Agent': 'Mozilla/5.0' } });
    return resp.ok ? Buffer.from(await resp.arrayBuffer()) : null;
  }
  return null;
}

const clock = (ms) => `${Math.floor(ms / 60_000)}:${String(Math.round(ms / 1000) % 60).padStart(2, '0')}`;

/**
 * A post's scenes (lib.planWithStreams): its scripts paired with its video
 * files; a scene with a streaming page (rule34video, vrbangers.com) is
 * checked by the length of its main script (read now); a rule34video one
 * gets the chosen quality's size (a vrbangers.com page gives it).
 */
async function planTopic(api, pacer, topicId, files, { vr, libraryVideos }) {
  const lengths = new Map();
  const scriptMs = async (f) => {
    const key = `${f.url}|${f.remote || ''}`;
    if (!lengths.has(key)) {
      let ms = null;
      try {
        const bytes = await scriptBytes(api, pacer, f);
        if (bytes) ms = lib.scriptDurationMs(JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/, ''))) || null;
      } catch (err) {
        if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
      }
      lengths.set(key, ms);
    }
    return lengths.get(key);
  };
  const plan = await lib.planWithStreams({
    scripts: files.filter((f) => /\.funscript$/i.test(f.name)),
    videos: files.filter((f) => lib.isVideoName(f.name)),
    vr, libraryVideos, scriptMs,
  });
  for (const d of plan.dropped) {
    process.stderr.write(`! ${topicId} ${d.url}: video ${clock(d.durationSec * 1000)}, script ${clock(d.scriptMs)}; not this scene's video\n`);
  }
  for (const sc of plan.scenes) {
    if (sc.video.host !== streams.RULE34VIDEO_HOST) continue;
    let size = 0;
    try { size = await rule34video.probeSize(sc.video); } catch { /* size unknown */ }
    sc.video = { ...sc.video, size };
  }
  return plan;
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

/** What screening needs: the library, the posts it already links or the user turned down, favourites, scoring. */
function scoutContext(config, userData, incoming) {
  const roots = [...librarySources(config, incoming).map((s) => s.path), incoming].filter((p) => fs.existsSync(p));
  const library = libraryState(roots);
  const knownTopics = new Set(Object.values(config.settings?.library?.scriptTags || {}).map((r) => r?.topicId).filter((id) => id != null));
  try {
    const links = require('../electron/eroscripts-links.js').readLinksFile(userData);
    for (const l of Object.values(links?.links || {})) if (l.topicId != null) knownTopics.add(l.topicId);
    for (const l of Object.values(links?.links || {})) if (l.creator) library.creators.add(lib.canonicalArtist(l.creator).toLowerCase());
  } catch { /* no links file yet */ }
  // Posts the user turned down in the app (2 stars or lower; formerly the No thank you playlist);
  // a short video or an extra copy of one (dedupe) doesn't turn down the post.
  const turnedDown = new Set(rejectedVideos.readRejected(userData).entries.filter((e) => e.reason !== 'short' && e.reason !== 'duplicate' && e.topicId != null).map((e) => e.topicId));
  for (const id of turnedDown) knownTopics.add(id);
  return { library, knownTopics, turnedDown, ...live.scoutPreferences(config) };
}

/** A judged post as a saved candidate, scored with the user's tag scores and favourite artist bonus. */
function makeCandidate({ t, s, author, artistInLibrary, favourite, hosts, scenes, first, picked = false }, scoring) {
  const axisType = scenes.some((sc) => sc.axes.length || sc.packed) ? 'multi' : 'single';
  const c = {
    id: t.id, title: t.title, url: `https://discuss.eroscripts.com/t/${t.id}`, author,
    tags: t.tags, likes: t.likes,
    vr: s.vr, music: s.music, axisType, artistInLibrary, hosts, live: s.live, favourite, sexual: first.sexual,
    ...(first.monster ? { monster: true } : {}), ...(first.feet ? { feet: true } : {}), ...(first.ffm ? { ffm: true } : {}), ...(picked ? { picked } : {}),
    scenes,
  };
  return { ...c, score: lib.candidateScore(c, scoring) };
}

async function scan(opts, { api, userData, config, incoming }) {
  const pacer = createPacer();
  const { library, knownTopics, turnedDown, favourites, scoring } = scoutContext(config, userData, incoming);
  const state = readScoutState(userData);
  const rejected = { known: 0, excluded: 0, blocked: 0, 'not-sexual': 0, 'live-not-pov': 0, 'in-library': 0, 'no-video': 0 };
  const fresh = [];
  let listed = 0;
  let judged = 0;
  try {
    let todo;
    if (opts.rejudge) {
      // No listing: the posts come from the state, each read once below (its listing fields from the same read).
      todo = lib.staleTopicIds(state, opts.rejudge);
      listed = todo.length;
      console.log(`${todo.length} posts judged ${opts.rejudge.join(' / ')} under rules older than v${lib.RULES_VERSION} to judge again`);
    } else {
      const topics = new Map();
      let pages = 0;
      const carried = lib.pendingTopics(state).length;
      const listPage = lib.feedPage(api, { pacer });
      for (const tag of opts.feeds || prefs.FEED_TAGS) {
        const maxNew = tag === 'multi-axis' || Object.hasOwn(prefs.SEARCH_FEEDS, tag) ? opts.pages : opts.animPages;
        const r = await lib.listFeed(state, tag, listPage, { maxNew, backfill: opts.backfill, full: opts.full, since: opts.since, keepPending: true });
        pages += r.pagesRead;
        for (const t of r.topics) topics.set(t.id, t);
      }
      listed = topics.size;
      // Everything listed now or by an earlier run that stopped before judging it.
      todo = opts.full ? [...topics.values()] : lib.pendingTopics(state);
      const bumped = todo.filter((t) => lib.seenTopic(state, t.id)).length;
      console.log(`Listed ${listed} posts on ${pages} pages; ${todo.length} to judge${carried ? ` (${carried} left from an earlier run)` : ''}${bumped && !opts.full ? `, ${bumped} of them bumped since their verdict` : ''}`);
    }

    const creators = [...library.creators];
    for (const item of todo) {
      let t = item;
      let data = null;
      if (opts.rejudge) {
        await pacer.wait();
        data = await api.getTopicData(item);
        if (!data?.post_stream?.posts?.[0]) { process.stderr.write(`! ${item} unreadable\n`); continue; }
        // Moved out of Free Scripts since: nothing to download either way, so the verdict stands under the current rules.
        if (data.category_id !== lib.FREE_SCRIPTS_CATEGORY) { lib.markTopic(state, item, state.topics[item].verdict, Date.now(), lib.RULES_VERSION, lib.postInfo(null, data)); continue; }
        t = lib.listedTopic(item, data);
      }
      const verdict = (v) => { lib.markTopic(state, t.id, v, Date.now(), lib.RULES_VERSION, lib.postInfo(t, data)); judged++; if (v !== 'candidate') rejected[v] = (rejected[v] || 0) + 1; };
      const s = lib.screenTopic(t, { knownTopics, creators, sceneNames: library.sceneNames, favourites });
      if (!s.ok) { verdict(s.reason); continue; }
      if (!data) {
        await pacer.wait();
        data = await api.getTopicData(t.id);
      }
      const op = data?.post_stream?.posts?.[0];
      if (!op) { process.stderr.write(`! ${t.id} unreadable\n`); continue; }
      if (lib.isBlockedCreator({ author: op.username })) { verdict('blocked'); continue; }
      const cooked = op.cooked || '';
      const text = cooked.replace(/<[^>]+>/g, ' ');
      const first = lib.judgeFirstPost(t, text);
      if (!first.ok) { verdict(first.reason); continue; }
      let files, hosts, hostDown;
      try { ({ files, hosts, hostDown } = await topicFiles(api, pacer, t.id, cooked, t.title)); } catch (err) {
        if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
        process.stderr.write(`! ${t.id} ${err.message}\n`);
        continue;
      }
      const plan = await planTopic(api, pacer, t.id, files, { vr: s.vr, libraryVideos: library.videos });
      // Scripts and videos packed in archives: the post's archives are scenes (unpacked at --fetch).
      const scenes = plan.scenes.length ? plan.scenes : lib.archiveScenes(files);
      // rule34video refused or failed (not 404): no verdict, so the post stays pending and the next run reads it again.
      if (!scenes.length && hostDown) { process.stderr.write(`! ${t.id} rule34video unreachable: left for the next run\n`); continue; }
      if (!scenes.length) { verdict(plan.inLibrary > 0 ? 'in-library' : 'no-video'); continue; }
      const author = op.username;
      const creator = lib.canonicalArtist(author);
      const artistInLibrary = s.artist || (library.creators.has(creator.toLowerCase()) ? creator : null);
      const favourite = s.favourite || live.favouriteIn([author], favourites);
      fresh.push(makeCandidate({ t, s, author, artistInLibrary, favourite, hosts, scenes, first }, scoring));
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
  const candidates = lib.mergeCandidates(previous, fresh, (id, video) => done.has(lib.fetchedKey(id, video)), turnedDown, scoring, favourites);
  fs.writeFileSync(out, JSON.stringify({ generatedAt: new Date().toISOString(), listed, judged, rejected, candidates }, null, 1));

  // npm run vrbangers:scout: only the posts with a vrbangers.com video (the saved list keeps all).
  const shown = opts.vrbangers ? candidates.filter(lib.isVrbangersCandidate) : candidates;
  const gb = (c) => c.scenes.reduce((n, sc) => n + (sc.video.size || 0), 0) / 1e9;
  const freshIds = new Set(fresh.map((c) => c.id));
  const tagScore = lib.effectiveTagScores(scoring.tagScores);
  console.log('\n| # | New | Post | Axis | Tags | Videos | GB | Host |\n|---|---|---|---|---|---|---|---|');
  shown.slice(0, opts.top).forEach((c, i) => {
    const axis = c.axisType !== 'multi' ? 'Single' : c.scenes.every((sc) => sc.packed) ? 'Multi (packed)' : `Multi (${1 + new Set(c.scenes.flatMap((sc) => sc.axes)).size})`;
    const tags = [c.vr && 'VR', ...(c.tags || []).filter((x) => (tagScore[x] ?? 0) > 0), c.favourite && `favourite: ${c.favourite}`, c.live && 'live action'].filter(Boolean).join(', ');
    console.log(`| ${i + 1} | ${freshIds.has(c.id) ? '✓' : ''} | [${c.title.replace(/[|[\]]/g, ' ').slice(0, 60)}](${c.url}) | ${axis} | ${tags} | ${c.scenes.length} | ${gb(c).toFixed(2)} | ${[...new Set(c.scenes.map((sc) => sc.video.host))].join('+')} |`);
  });
  const multi = shown.filter((c) => c.axisType === 'multi').length;
  const backfill = lib.ALL_FEEDS.filter((tag) => prefs.FEED_TAGS.includes(tag) || state.feeds[tag]).map((tag) => `${tag} ${lib.feedProgress(state.feeds[tag])}`).join(', ');
  const left = Object.keys(state.pending || {}).length;
  const waiting = `waiting to download ${shown.length}${opts.vrbangers ? ` with a vrbangers.com video (of ${candidates.length})` : ''}`;
  console.log(`\nListed ${listed} · judged ${judged} · rejected ${JSON.stringify(rejected)} · new ${fresh.length} · ${waiting} (${multi} multi-axis, ${shown.length - multi} single-axis), ${shown.reduce((n, c) => n + c.scenes.length, 0)} videos, ${shown.reduce((n, c) => n + gb(c), 0).toFixed(1)} GB`);
  console.log(`Backfill: ${backfill} · ${Object.keys(state.topics).length} posts judged so far${left ? ` · ${left} listed, not judged yet (next run goes on with them)` : ''}`);
  console.log(`Saved: ${out}`);
  return 0;
}

// ── Pick ────────────────────────────────────────────────────────────────

/** `--add`: judge posts the user picked by id and put them on the list (lib.screenPick). */
async function addPicked(opts, { api, userData, config, incoming }) {
  const pacer = createPacer();
  const { library, knownTopics, turnedDown, favourites, scoring } = scoutContext(config, userData, incoming);
  const state = readScoutState(userData);
  const fresh = [];
  try {
    for (const id of opts.add) {
      await pacer.wait();
      const data = await api.getTopicData(id);
      const op = data?.post_stream?.posts?.[0];
      if (!op) { console.log(`✗ ${id}: unreadable`); continue; }
      const t = lib.listedTopic(id, data);
      if (data.category_id !== lib.FREE_SCRIPTS_CATEGORY) { console.log(`✗ ${id} ${t.title}: not in Free Scripts`); continue; }
      const s = lib.screenPick(t, { knownTopics, creators: [...library.creators], sceneNames: library.sceneNames, favourites });
      if (!s.ok) { console.log(`✗ ${id} ${t.title}: ${s.reason}`); continue; }
      if (lib.isBlockedCreator({ author: op.username })) { console.log(`✗ ${id} ${t.title}: blocked creator`); continue; }
      const cooked = op.cooked || '';
      const first = lib.judgeFirstPost(t, cooked.replace(/<[^>]+>/g, ' '));
      if (!first.ok) { console.log(`✗ ${id} ${t.title}: ${first.reason}`); continue; }
      let files, hosts, hostDown;
      try { ({ files, hosts, hostDown } = await topicFiles(api, pacer, id, cooked, t.title)); } catch (err) {
        if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
        console.log(`✗ ${id} ${t.title}: ${err.message}`);
        continue;
      }
      const plan = await planTopic(api, pacer, id, files, { vr: s.vr, libraryVideos: library.videos });
      const scenes = plan.scenes.length ? plan.scenes : lib.archiveScenes(files);
      if (!scenes.length) { console.log(`✗ ${id} ${t.title}: ${hostDown ? 'rule34video unreachable, try again later' : plan.inLibrary > 0 ? 'in-library' : 'no-video'}`); continue; }
      const author = op.username;
      const creator = lib.canonicalArtist(author);
      const artistInLibrary = s.artist || (library.creators.has(creator.toLowerCase()) ? creator : null);
      const favourite = s.favourite || live.favouriteIn([author], favourites);
      fresh.push(makeCandidate({ t, s, author, artistInLibrary, favourite, hosts, scenes, first, picked: true }, scoring));
      lib.markTopic(state, id, 'candidate', Date.now(), lib.RULES_VERSION, lib.postInfo(t, data));
      const kind = scenes.every((sc) => sc.packed) ? 'packed in archives' : scenes.some((sc) => sc.axes.length) ? 'multi-axis' : 'single-axis';
      const on = [...new Set(scenes.map((sc) => sc.video.host).filter(Boolean))].join('+');
      console.log(`✓ ${id} ${t.title} (${scenes.length} ${scenes.every((sc) => sc.packed) ? 'archive' : 'video'}${scenes.length > 1 ? 's' : ''}, ${kind}${on ? `, ${on}` : ''})`);
    }
  } finally {
    writeScoutState(userData, state);
  }
  const out = path.join(userData, 'eroscripts-scout.json');
  let saved = { candidates: [] };
  try { saved = JSON.parse(fs.readFileSync(out, 'utf8')); } catch { /* no list yet */ }
  const done = fetchedScenes(incoming, userData);
  const candidates = lib.mergeCandidates(saved.candidates || [], fresh, (id, video) => done.has(lib.fetchedKey(id, video)), turnedDown, scoring, favourites);
  fs.writeFileSync(out, JSON.stringify({ ...saved, generatedAt: new Date().toISOString(), candidates }, null, 1));
  console.log(`\nAdded ${fresh.length} of ${opts.add.length}.${fresh.length ? ` Download them with: npm run ${opts.vrbangers ? 'vrbangers' : 'eroscripts'}:scout -- --fetch --ids ${fresh.map((c) => c.id).join(',')}` : ''}`);
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
 * Files saved by hand: the files directly in each of `dirs` (AppleDouble
 * sidecars left out), as `[{name, path, size}]`.
 */
function localDownloads(dirs) {
  const out = [];
  for (const dir of dirs) {
    let entries = [];
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { continue; }
    for (const e of entries) {
      if (!e.isFile() || e.name.startsWith('._')) continue;
      const full = path.join(dir, e.name);
      try { out.push({ name: e.name, path: full, size: fs.statSync(full).size }); } catch { /* gone meanwhile */ }
    }
  }
  return out;
}

/** Length of a video file in seconds (ffprobe), 0 when it can't be read. */
function videoSeconds(file) {
  const r = spawnSync('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', file], { encoding: 'utf8' });
  if (r.error) throw new Error(`ffprobe not found (brew install ffmpeg): ${r.error.message}`);
  return Number(String(r.stdout).trim()) || 0;
}

/**
 * Download `file` to `dest` through `<staging>/<name>.part`. Every attempt
 * starts from scratch, so a failed one deletes its `.part` rather than leave
 * it behind. A copy the user saved by hand (`local`, see localDownloads) is
 * used instead of the network and removed from Downloads once it's in place.
 * A rule34video page is read again for a fresh link, and the video must fit
 * the main script (`scriptMs`, else the page's length) before and after
 * the download (streams.durationMatches; ffprobe afterwards). A vrbangers.com
 * scene's signed link is read right now (logged-in Chrome), and the host
 * checks size and length itself (vrbangers-host.mjs download).
 */
async function download(api, file, dest, staging, local = [], { scriptMs = null } = {}) {
  if (fs.existsSync(dest)) return 'have';
  fs.mkdirSync(staging, { recursive: true });
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = path.join(staging, `${path.basename(dest)}.part`);
  const byHand = lib.findLocalCopy(file, local);
  activePart = tmp;
  try {
    if (byHand) {
      fs.copyFileSync(byHand.path, tmp);
    } else if (file.host === 'forum') {
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
      if (!resp.ok) throw new Error(`pixeldrain: HTTP ${resp.status}${resp.status === 403 ? ' (download it by hand: --manual-list)' : ''}`);
      await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp));
    } else if (file.host === streams.RULE34VIDEO_HOST) {
      const { direct, durationSec } = await rule34video.fresh(file);
      if (scriptMs && durationSec && !streams.durationMatches(durationSec, scriptMs)) {
        throw new Error(`rule34video: video ${clock(durationSec * 1000)}, script ${clock(scriptMs)}; not this scene's video`);
      }
      const resp = await rule34video.stream(direct);
      await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp));
    } else if (file.host === vrbHost.VRBANGERS_HOST) {
      await vrbangers.download(file, tmp, { scriptMs, videoSeconds });
    } else {
      throw new Error(`unknown host ${file.host}`);
    }
    const size = fs.statSync(tmp).size;
    if (file.size && size !== file.size && (byHand || file.host !== vrbHost.VRBANGERS_HOST)) throw new Error(`size ${size} ≠ ${file.size}`);
    if (file.host === streams.RULE34VIDEO_HOST) {
      const want = scriptMs || (file.duration ? file.duration * 1000 : null);
      const got = videoSeconds(tmp);
      if (!got) throw new Error('rule34video: the download isn\'t a readable video (ffprobe)');
      if (want && !streams.durationMatches(got, want)) throw new Error(`rule34video: downloaded video ${clock(got * 1000)}, script ${clock(want)}; not this scene's video`);
    }
    fs.renameSync(tmp, dest);
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    throw err;
  } finally {
    activePart = null;
  }
  if (byHand) {
    fs.rmSync(byHand.path, { force: true });
    local.splice(local.indexOf(byHand), 1);
    return 'local';
  }
  return 'ok';
}

/** Scenes not to fetch: those with a sidecar and video, and every rejected one; `topics` (optional Set) gets the posts with a sidecar. */
function fetchedScenes(incoming, userData, topics = null) {
  const done = new Set(rejectedVideos.readRejected(userData).entries.map((e) => e.key));
  walk(incoming, 0, (full, e) => {
    if (e.name !== 'eroscripts.json') return;
    try {
      const rec = JSON.parse(fs.readFileSync(full, 'utf8'));
      const dir = path.dirname(full);
      const file = rec.file || fs.readdirSync(dir).find((n) => !n.startsWith('._') && lib.isVideoName(n));
      if (file && fs.existsSync(path.join(dir, file))) {
        done.add(lib.fetchedKey(rec.topicId, rec.video));
        topics?.add(rec.topicId);
      }
    } catch { /* unreadable sidecar: fetch again */ }
  });
  return done;
}

/** One pass over the queue; downloads at most `room` videos (the library cap). */
async function fetchPass(queue, { api, userData, incoming, staging, room, manualDir }) {
  const done = fetchedScenes(incoming, userData);
  // Files saved by hand: posts with one go first, and the folders are read
  // again before every scene, so a file saved during a long pass is taken
  // when its scene comes up, not a pass later.
  const readLocal = () => localDownloads([manualDir, DOWNLOADS]);
  const saved = readLocal();
  const byHand = (c) => c.scenes.some((sc) => [sc.video, ...sc.scripts].some((f) => lib.findLocalCopy(f, saved)));
  const order = [...queue].sort((a, b) => byHand(b) - byHand(a));
  let ok = 0; let failed = 0; let generated = 0; let short = 0; let capped = false;
  const now = () => new Date().toISOString().slice(11, 19);
  for (const c of order) {
    if (capped) break;
    for (const sc of c.scenes) {
      const local = readLocal();
      if (done.has(lib.fetchedKey(c.id, sc.video.name))) continue;
      if (ok >= room) { capped = true; break; }
      if (sc.packed) {
        try {
          const r = await fetchPacked(c, sc, { api, userData, incoming, staging, local, now });
          ok += r.ok; short += r.short; generated += r.generated;
        } catch (err) {
          console.log(`${now()} ✗ ${lib.creatorFolder(c.title, c.author)}/${sc.video.name}: ${err.message}`);
          failed++;
        }
        continue;
      }
      const { creator, stem } = lib.sceneNameOf(c, sc);
      const dir = path.join(incoming, creator, stem);
      const video = path.join(dir, stem + path.extname(sc.video.name).toLowerCase());
      // Another post already brought this scene (same folder and video file): leave the
      // folder alone. Its scripts, sidecar and length were judged when it came, so this
      // post's files and the current length rule don't apply to it.
      if (fs.existsSync(video)) { console.log(`${now()} = ${creator}/${stem}: already in the library from another post; left as is`); continue; }
      try {
        for (const f of sc.scripts) await download(api, f, path.join(dir, lib.scriptFileName(stem, f.axis)), staging, local);
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
        const got = await download(api, sc.video, video, staging, local, { scriptMs: lengthMs });
        fs.writeFileSync(path.join(dir, 'eroscripts.json'), JSON.stringify(
          lib.sceneSidecar(c, sc.video, path.basename(video), new Date().toISOString()), null, 1));
        let note = '';
        if (!sc.axes.length) {
          const up = spawnSync(process.execPath, [path.join(REPO, 'scripts/upgrade-multi-axis.mjs'), dir], { cwd: REPO, encoding: 'utf8' });
          if (up.status === 0) { generated++; note = ' + generated axes'; } else note = ` (axis generation failed: ${(up.stderr || up.stdout).trim().split('\n').pop()})`;
        }
        const kind = c.favourite && c.live ? ` [favourite: ${c.favourite}]` : '';
        console.log(`${now()} ✓ ${creator}/${stem}${kind}${note}${got === 'local' ? ' (from Downloads)' : ''}`);
        ok++;
      } catch (err) {
        // Not logged in to vrbangers.com: every later VRBangers scene would fail the same way.
        if (err instanceof LoginError) throw err;
        console.log(`${now()} ✗ ${creator}/${stem}: ${err.message}`);
        failed++;
      }
    }
  }
  return { ok, failed, generated, short, capped };
}

/**
 * Unpack `archive` into `out`: unar (RAR 5 included), else bsdtar. Throws
 * when neither can. No stdin and a time limit: bsdtar asks for the password
 * of an encrypted ZIP on the terminal and would wait for it forever.
 */
function unpack(archive, out) {
  const tries = [['unar', ['-q', '-f', '-D', '-o', out, archive]], ['bsdtar', ['-xf', archive, '-C', out]]];
  const errors = [];
  for (const [cmd, args] of tries) {
    const r = spawnSync(cmd, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 30 * 60 * 1000 });
    if (r.status === 0) return;
    const why = r.error?.code === 'ENOENT' ? 'not installed' : r.error?.code === 'ETIMEDOUT' ? 'timed out' : (r.stderr || r.stdout || '').trim().split('\n').pop();
    if (/password/i.test(why)) throw new Error(`${path.basename(archive)} is password-protected (the password is usually in the post); unpack it by hand`);
    errors.push(`${cmd}: ${why}`);
  }
  throw new Error(`can't unpack ${path.basename(archive)} (${errors.join('; ')}); brew install unar`);
}

/** Every file under `dir` (AppleDouble sidecars and __MACOSX left out), as `{name, path, size}`. */
function filesUnder(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name.startsWith('._') || e.name === '__MACOSX') continue;
    const full = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...filesUnder(full));
    else if (e.isFile()) out.push({ name: e.name, path: full, size: fs.statSync(full).size });
  }
  return out;
}

/**
 * A packed scene (lib.archiveScenes): download the archive into staging (or
 * take the copy saved by hand), unpack it, and file every scene inside
 * (lib.unpackedScenes) like a downloaded one: named, scripts under the
 * library's axis names, sidecar (`video` = the archive, so it counts as
 * fetched), the length rule. An archive that files no scene (none inside,
 * all under the minimum length, or all in the library already) is recorded as
 * rejected so it isn't fetched again. The archive and its unpacked files are
 * removed once filed; a failed unpack keeps the archive in staging for the next pass.
 */
async function fetchPacked(c, sc, { api, userData, incoming, staging, local, now }) {
  const archive = path.join(staging, sc.video.name);
  const got = await download(api, sc.video, archive, staging, local);
  const out = path.join(staging, `${sc.base}.unpacked`);
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });
  const r = { ok: 0, short: 0, generated: 0 };
  try {
    unpack(archive, out);
    const scenes = lib.unpackedScenes(filesUnder(out), { vr: c.vr });
    const creator = lib.creatorFolder(c.title, c.author);
    const reject = (reason, extra = {}) => rejectedVideos.recordRejection(userData, {
      topicId: c.id, video: sc.video.name, title: c.title, url: c.url, reason, ...extra, at: new Date().toISOString(),
    });
    if (!scenes.length) {
      reject('no-video');
      console.log(`${now()} – ${creator}/${sc.video.name}: no scene inside; skipped`);
      fs.rmSync(archive, { force: true });
      return r;
    }
    const shared = c.scenes.length > 1 || scenes.length > 1;
    for (const s of scenes) {
      const stem = lib.sceneTitle({ title: c.title, author: c.author, video: s.video.name, shared });
      const dir = path.join(incoming, creator, stem);
      const video = path.join(dir, stem + path.extname(s.video.name).toLowerCase());
      if (fs.existsSync(video)) { console.log(`${now()} = ${creator}/${stem}: already in the library; left as is`); continue; }
      const main = s.scripts.find((f) => f.axis === '');
      let lengthMs = null;
      try { lengthMs = lib.scriptDurationMs(JSON.parse(fs.readFileSync(main.path, 'utf8'))); } catch { /* unreadable script: file it anyway */ }
      if (lengthMs !== null && lengthMs < prefs.MIN_VIDEO_MS) {
        console.log(`${now()} – ${creator}/${stem}: ${Math.round(lengthMs / 1000)} s, under ${lib.MIN_VIDEO_LABEL}; skipped`);
        r.short++;
        continue;
      }
      fs.mkdirSync(dir, { recursive: true });
      for (const f of s.scripts) fs.renameSync(f.path, path.join(dir, lib.scriptFileName(stem, f.axis)));
      fs.renameSync(s.video.path, video);
      fs.writeFileSync(path.join(dir, 'eroscripts.json'), JSON.stringify({
        topicId: c.id, url: c.url, title: c.title, author: c.author, tags: c.tags,
        video: sc.video.name, archiveEntry: s.video.name, file: path.basename(video), scenes: c.scenes.length, at: new Date().toISOString(),
      }, null, 1));
      let note = '';
      if (!s.axes.length) {
        const up = spawnSync(process.execPath, [path.join(REPO, 'scripts/upgrade-multi-axis.mjs'), dir], { cwd: REPO, encoding: 'utf8' });
        if (up.status === 0) { r.generated++; note = ' + generated axes'; } else note = ` (axis generation failed: ${(up.stderr || up.stdout).trim().split('\n').pop()})`;
      }
      console.log(`${now()} ✓ ${creator}/${stem}${note} (unpacked from ${sc.video.name}${got === 'local' ? ', saved by hand' : ''})`);
      r.ok++;
    }
    // Nothing filed: no sidecar names this archive, so record it, or every pass and run fetches it again.
    if (!r.ok) reject(r.short ? 'short' : 'duplicate');
    fs.rmSync(archive, { force: true });
    return r;
  } finally {
    fs.rmSync(out, { recursive: true, force: true });
  }
}

/**
 * The saved list as a download queue (content-rule breaks dropped, `--ids` applied; `--vrbangers`: posts with a
 * vrbangers.com video only), checked again against the library as it is now (lib.notYetHeld).
 */
function savedQueue(opts, { userData, config, incoming }) {
  const file = path.join(userData, 'eroscripts-scout.json');
  if (!fs.existsSync(file)) throw new Error(`No scout list at ${file}; run npm run eroscripts:scout first.`);
  const { favourites, scoring } = live.scoutPreferences(config);
  const candidates = JSON.parse(fs.readFileSync(file, 'utf8')).candidates.map((c) => lib.withCurrentFavourite(c, favourites));
  const breaks = {};
  for (const c of candidates) {
    const rule = lib.contentRuleBreak(c);
    if (rule) breaks[rule] = (breaks[rule] || 0) + 1;
  }
  const dropped = Object.values(breaks).reduce((n, k) => n + k, 0);
  if (dropped) console.log(`Dropped ${dropped} waiting posts that break the content rules ${JSON.stringify(breaks)}`);
  const picked = lib.downloadQueue(candidates, scoring, favourites)
    .filter((c) => (!opts.ids || opts.ids.has(c.id)) && (!opts.vrbangers || lib.isVrbangersCandidate(c)) && (!opts.auto || lib.autoFetchPick(c, scoring)));
  const { library, knownTopics, turnedDown } = scoutContext(config, userData, incoming);
  const ownTopics = new Set();
  fetchedScenes(incoming, userData, ownTopics);
  const held = lib.notYetHeld(picked, { knownTopics, ownTopics, turnedDown, libraryVideos: library.videos });
  if (held.posts || held.scenes) console.log(`Skipped what the library holds now: ${held.posts} posts, ${held.scenes} more scenes`);
  return held.queue;
}

/** Write ~/Downloads/download-for-fs.txt: the waiting posts' files to download by hand. */
function manualList(opts, { userData, config, incoming }) {
  const done = fetchedScenes(incoming, userData);
  const posts = lib.manualDownloads(savedQueue(opts, { userData, config, incoming }), (id, video) => done.has(lib.fetchedKey(id, video)));
  fs.mkdirSync(opts.manualDir, { recursive: true });
  const out = path.join(DOWNLOADS, 'download-for-fs.txt');
  fs.writeFileSync(out, lib.manualListText(posts, { folder: opts.manualDir }));
  console.log(`${out}: ${posts.length} posts, ${posts.reduce((n, p) => n + p.files.length, 0)} files to download by hand into ${opts.manualDir}`);
  return 0;
}

async function fetchAll(opts, { api, userData, config, incoming }) {
  const queue = savedQueue(opts, { userData, config, incoming });
  const gb = queue.reduce((n, c) => n + c.scenes.reduce((m, sc) => m + (sc.video.size || 0), 0), 0) / 1e9;
  console.log(`Queue: ${queue.length} posts, ${queue.reduce((n, c) => n + c.scenes.length, 0)} videos, ${gb.toFixed(1)} GB → ${incoming}`);
  if (opts.dryRun) {
    for (const c of queue) console.log(`  ${c.axisType.padEnd(6)} ${c.id} ${c.title}`);
    return probeVrbangers(queue, { userData, incoming });
  }
  ensureLibrarySource(userData, { name: FOLDER, path: incoming });
  const staging = path.join(path.dirname(incoming), '_staging');
  for (let pass = 1; pass <= Math.max(1, opts.passes); pass++) {
    const held = libraryVideoCount(readConfig(userData));
    if (held >= prefs.LIBRARY_MAX_VIDEOS) {
      console.log(`Library holds ${held} videos, the cap is ${prefs.LIBRARY_MAX_VIDEOS} (LIBRARY_MAX_VIDEOS): nothing more downloaded. Delete scenes or raise the cap.`);
      break;
    }
    console.log(`Pass ${pass}/${Math.max(1, opts.passes)} (library ${held}/${prefs.LIBRARY_MAX_VIDEOS} videos)`);
    // Chrome (vrbangers.com) starts with the pass's first VRBangers scene and is closed when the pass ends, failed or not.
    const r = await vrbangers.run(() => fetchPass(queue, { api, userData, incoming, staging, room: prefs.LIBRARY_MAX_VIDEOS - held, manualDir: opts.manualDir }));
    console.log(`Pass ${pass}: ${r.ok} downloaded (${r.generated} with generated axes), ${r.short} under ${lib.MIN_VIDEO_LABEL}, ${r.failed} failed${r.capped ? `; stopped at the library cap (${prefs.LIBRARY_MAX_VIDEOS} videos)` : ''}`);
    if (r.ok) dedupeAfterFetch(userData);
    if (r.capped) break;
    if (!r.failed) return 0;
    if (pass < opts.passes) {
      console.log(`Waiting ${opts.wait / 60} min for host quotas`);
      await new Promise((resolve) => setTimeout(resolve, opts.wait * 1000));
    }
  }
  return 2;
}

/**
 * `--fetch --dry-run`: each waiting vrbangers.com scene's signed link is
 * read (logged in) and asked for its first byte, nothing downloaded; the
 * file taken, its size and when the link expires are printed (the token
 * redacted). Chrome is closed at the end.
 */
async function probeVrbangers(queue, { userData, incoming }) {
  const done = fetchedScenes(incoming, userData);
  const scenes = queue.flatMap((c) => c.scenes.filter((sc) => sc.video.host === vrbHost.VRBANGERS_HOST && !done.has(lib.fetchedKey(c.id, sc.video.name))).map((sc) => ({ c, sc })));
  if (!scenes.length) return 0;
  console.log(`vrbangers.com: resolving ${scenes.length} signed link${scenes.length > 1 ? 's' : ''} (1-byte probe, nothing downloaded)`);
  let failed = 0;
  await vrbangers.run(async () => {
    for (const { c, sc } of scenes) {
      const { creator, stem } = lib.sceneNameOf(c, sc);
      try {
        const { link, status, total } = await vrbangers.probe(sc.video);
        const hours = link.expires ? Math.round((link.expires * 1000 - Date.now()) / 3_600_000) : null;
        const planned = link.file === sc.video.name ? 'as planned' : `planned ${sc.video.name}`;
        console.log(`  ${c.id} ${creator}/${stem}\n    ${link.file} (${link.quality}, ${planned}) → HTTP ${status}, ${total ? `${(total / 1e9).toFixed(2)} GB` : 'size unknown'}${link.expires ? `, link expires ${new Date(link.expires * 1000).toISOString()} (in ${hours} h)` : ''}\n    ${redactUrl(link.url)}`);
        if (status !== 206 && status !== 200) failed++;
      } catch (err) {
        if (err instanceof LoginError) throw err;
        console.log(`  ${c.id} ${creator}/${stem}: ✗ ${err.message}`);
        failed++;
      }
    }
  });
  return failed ? 2 : 0;
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
      const rec = JSON.parse(fs.readFileSync(full, 'utf8'));
      if (opts.ids && !opts.ids.has(Number(rec.topicId))) return;
      scenes.push({ dir, rec, files: fs.readdirSync(dir).filter((n) => !n.startsWith('._')) });
    } catch { process.stderr.write(`! unreadable ${full}\n`); }
  });
  const moves = lib.retitlePlan(scenes, incoming, (p) => fs.existsSync(p));
  const rel = (p) => path.relative(incoming, p);
  for (const m of moves) console.log(`${rel(m.from)}\n  → ${rel(m.to)}`);
  console.log(`${moves.length} of ${scenes.length} scenes ${opts.dryRun ? 'would be renamed' : 'to rename'}`);
  if (opts.dryRun || !moves.length) return 0;
  return applyMoves(moves, { userData, incoming, sidecar: 'eroscripts.json' });
}

// ── Status, daily run, run lock ─────────────────────────────────────────

const DAILY_FILE = 'eroscripts-scout-daily.json';
const LOCK_FILE = 'eroscripts-scout.lock';

/** `--status`: the scan state, backfill progress and waiting list, offline. */
function status({ userData, config, incoming }) {
  const state = readScoutState(userData);
  const { favourites, scoring } = live.scoutPreferences(config);
  let saved = [];
  try { saved = JSON.parse(fs.readFileSync(path.join(userData, 'eroscripts-scout.json'), 'utf8')).candidates || []; } catch { /* no list yet */ }
  const done = fetchedScenes(incoming, userData);
  const candidates = lib.downloadQueue(saved, scoring, favourites).filter((c) => c.scenes.some((sc) => !done.has(lib.fetchedKey(c.id, sc.video.name))));
  let daily = null;
  try { daily = JSON.parse(fs.readFileSync(path.join(userData, DAILY_FILE), 'utf8')); } catch { /* no daily run yet */ }
  for (const line of lib.scoutStatus(state, { candidates, scoring, daily })) console.log(line);
  return 0;
}

/** The exit code of a run step: rate limit 2, expired session or VRBangers login 3; other errors are thrown. */
async function exitCodeOf(step) {
  try {
    return await step();
  } catch (err) {
    if (err instanceof RateLimitedError) { console.error(err.message); return 2; }
    if (err instanceof SessionExpiredError) { console.error(err.message); return 3; }
    if (err instanceof LoginError) { console.error(`vrbangers.com: ${err.message}`); return 3; }
    throw err;
  }
}

/**
 * `--titles`: title, author, dates and image for judged posts that lack them, newest first,
 * paced like a scan. A post the forum no longer has (404) is marked `gone`; a run that hits
 * the rate limit or a run of refusals (expired session) stops, saving what it has.
 */
async function fillTitles({ api, userData }) {
  const pacer = createPacer();
  const state = readScoutState(userData);
  const ids = lib.untitledTopicIds(state);
  console.log(`${ids.length} judged posts without a title`);
  let done = 0; let gone = 0; let refused = 0; let skipped = 0;
  try {
    for (const id of ids) {
      await pacer.wait();
      const data = await api.getTopicData(id);
      if (data?.error_type === 'rate_limit') throw new RateLimitedError();
      if (data?.error_type === 'not_found') { lib.addPostInfo(state, id, null); gone++; refused = 0; continue; }
      if (!data?.title) {
        skipped++;
        // 20 refusals or empty answers in a row: the session or the forum is down; go on next run.
        if (++refused >= 20) throw (data?.error_type === 'invalid_access' ? new SessionExpiredError() : new RateLimitedError());
        continue;
      }
      refused = 0;
      lib.addPostInfo(state, id, data);
      if (++done % 50 === 0) {
        writeScoutState(userData, state);
        console.log(`  ${done} of ${ids.length}`);
      }
    }
  } finally {
    writeScoutState(userData, state);
    console.log(`Titles: ${done} filled in, ${gone} posts gone, ${skipped} unreadable (tried again next run)`);
  }
  return skipped ? 2 : 0;
}

/**
 * `--daily`: a scan (new posts plus the usual backfill), then `--fetch --auto`.
 * `<userData>/eroscripts-scout-daily.json` records when each step ran and its
 * exit code; it's written right after the scan, so the watchdog sees the day's
 * run as done while the downloads go on.
 */
async function daily(opts, ctx) {
  const file = path.join(ctx.userData, DAILY_FILE);
  const scanAt = new Date().toISOString();
  const scanExit = await exitCodeOf(() => scan(opts, ctx));
  writeAtomic(file, JSON.stringify({ scanAt, scanExit }, null, 1));
  if (scanExit === 3) return 3;
  console.log(`\nDaily: downloading the waiting hand-made multi-axis posts scoring ${prefs.AUTO_FETCH_MIN_SCORE}+`);
  const fetchExit = await exitCodeOf(() => fetchAll({ ...opts, fetch: true, auto: true }, ctx));
  writeAtomic(file, JSON.stringify({ scanAt, scanExit, fetchAt: new Date().toISOString(), fetchExit }, null, 1));
  return scanExit || fetchExit;
}

/**
 * One forum run at a time (scan, --add, --fetch, --daily): they all rewrite the
 * scan state and the list. Null when this run holds `<userData>/eroscripts-scout.lock`
 * (`<pid>@<host>`, freed at exit), else the holder of the live run that does. A
 * lock left by an ended run (this host: its pid is gone; another host, e.g. a
 * finished pod: older than 12 h) is taken over (scripts/host-env.mjs).
 */
function takeRunLock(userData) {
  return takePidLock(path.join(userData, LOCK_FILE));
}

// ── Main ────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserDataDir();
  const config = readConfig(userData);
  const incoming = downloadFolder(config, FOLDER, opts.incoming).path;
  if (opts.status) return status({ userData, config, incoming });
  if (opts.retitle) return retitle(opts, { userData, incoming });
  if (opts.manualList) return manualList(opts, { userData, config, incoming });
  const session = config.settings?.eroscripts?.session || {};
  if (!session.cookie) { console.error('Not logged in to EroScripts. Log in from the app (EroScripts panel), then re-run.'); return 3; }
  const holder = takeRunLock(userData);
  if (holder) { console.error(`Another eroscripts:scout run (${describeHolder(holder)}) is going; try again when it ends.`); return 2; }

  // The API module logs through electron-log; keep the CLI's output clean.
  const log = require('../electron/logger.js');
  log.transports.file.level = false;
  log.transports.console.level = false;
  const { EroScriptsAPI } = require('../electron/eroscripts-api.js');
  const api = new EroScriptsAPI();
  api.restoreSession(session.cookie, session.username);
  vrbangers = vrbHost.createVrbangers({ userData, credentialsFile: opts.vrbCredentials, log: (m) => process.stderr.write(`  vrbangers.com: ${m}\n`) });

  const ctx = { api, userData, config, incoming };
  if (opts.daily) return daily(opts, ctx);
  if (opts.add) return exitCodeOf(() => addPicked(opts, ctx));
  if (opts.titles) return exitCodeOf(() => fillTitles(ctx));
  return exitCodeOf(() => (opts.fetch ? fetchAll(opts, ctx) : scan(opts, ctx)));
}

main().then((code) => process.exit(code), (err) => {
  console.error(`eroscripts:scout: ${err.message}`);
  process.exit(1);
});
