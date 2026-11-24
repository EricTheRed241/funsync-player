#!/usr/bin/env node
// npm run eroscripts:scout — find free EroScripts posts worth adding, then
// download them into the library's EroScripts folder.
//
//   npm run eroscripts:scout                 scan new posts (+ a little backfill) → <userData>/eroscripts-scout.json
//   npm run eroscripts:scout -- --fetch      download the saved list (--parallel N: N scenes at once; each keeps
//                                            the library drive's minimum free space, scripts/disk-space.mjs)
//   npm run eroscripts:scout -- --retitle    rename fetched scenes to `Artist - Title (metadata)`
//   npm run eroscripts:scout -- --manual-list  write ~/Downloads/download-for-fs.txt: files to download by hand
//   npm run eroscripts:scout -- --status     backfill progress and the waiting list, offline
//   npm run eroscripts:scout -- --titles     title, author, dates, images for posts judged before verdicts kept them, and
//                                            the waiting posts' script previews (length, heat strip)
//   npm run eroscripts:scout -- --daily      scan, then download the waiting
//                                            posts swiped right in npm run scout:report (or wanted on the Creators site)
//   npm run eroscripts:scout -- --frames     frames of unswiped waiting posts' videos for the swipe deck (scripts/scout-frames.mjs)
//   npm run vrbangers:scout                  the same, VRBangers posts only (--vrbangers): vrbangers.com
//                                            scenes come from the paid account (scripts/vrbangers-host.mjs)
//   SexLikeReal scenes a post links (or, for a post naming SLR without a video, SLR's scene search finds by
//   title and script length) come from the user's SLR account (scripts/slr-host.mjs; npm run slr:scout -- --login),
//   only with --slr: the weekly npm run slr:scout -- --fetch hands them over post by post (account quota)
//
// Rules and ranking: scripts/eroscripts-scout-lib.mjs (no sex rule, the
// ranking decides; monster, feet and 2D hentai posts
// rank lower). Scenes land in the library's EroScripts folder,
// `<library parent>/EroScripts/<creator>/<Artist - Title (metadata)>/`, with an
// `eroscripts.json` sidecar. Single-axis scenes get generated twist / roll / pitch companions
// (scripts/upgrade-multi-axis.mjs). The EroScripts folder is a library
// source, so the scenes show in the library, the web remote and HereSphere;
// config.json is written (to add the source) only while the app is closed.
//
// Exit codes: 0 done, 1 error, 2 rate limited / downloads left after the
// last pass / downloads left for lack of disk space / SLR download quota used up / app running (--retitle) / another
// scout run doing the same kind of work (one lock per kind: scan, downloads,
// frames; lib.SCOUT_LOCKS), 3 not logged in / session expired (EroScripts,
// VRBangers, or SexLikeReal when its scenes were all that was left to do).

import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
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
const { appRunning, applyMoves, dedupeAfterFetch, downloadFolder, ensureLibrarySource, librarySources, libraryState, libraryVideoCount, readConfig, removeIfEmpty, walk, writeAtomic } = await import('./discovery-io.mjs');
const { createPacer, parseInterval, RateLimitedError, SessionExpiredError } = await import('./eroscripts-sync-lib.mjs');
const { defaultUserDataDir, describeHolder, releasePidLock, takePidLock } = await import('./host-env.mjs');
const live = await import('./scout-live.mjs');
const streams = await import('./stream-hosts.mjs');
const vrbHost = await import('./vrbangers-host.mjs');
const { DEFAULT_CREDENTIALS: VRB_CREDENTIALS, LoginError } = await import('./vrbangers-browser.mjs');
const { redactUrl } = await import('./vrbangers-lib.mjs');
const slrHost = await import('./slr-host.mjs');
const frames = await import('./scout-frames.mjs');
const prefs = await import('../renderer/js/content-preferences.js');
const disk = await import('./disk-space.mjs');
const require = createRequire(import.meta.url);
const rejectedVideos = require('../electron/rejected-videos.js');
const REPO = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
// One per run: every rule34video request goes through its pacing (≥ 400 ms apart, 60 s after a 429).
const rule34video = streams.createRule34video();
// One per run: pmvhaven.com API reads and downloads, ≥ 1 s apart.
const pmvhaven = streams.createPmvhaven();
const phHost = await import('./pornhub-host.mjs');
// Pornhub links: metadata and download through yt-dlp (scripts/pornhub-host.mjs).
const pornhub = phHost.createPornhub();
/** The height cap a post's videos are downloaded within (content-preferences videoCaps). */
const heightCap = (vr) => (vr ? prefs.VR_MAX_HEIGHT : prefs.FLAT_MAX_HEIGHT);
// Set in main (it needs the data folder and the credentials file): vrbangers.com reads, and Chrome for --fetch.
let vrbangers = null;
// Set in main (it needs the data folder, which holds the SLR session): sexlikereal.com reads and downloads.
let slr = null;
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
  --add 1,2,3            judge these post ids and put them on the list: only the exclusions,
                         posts the library has or the user turned down, and library duplicates count (the
                         live-not-POV rule doesn't); single-axis scenes get generated axes
Download:
  --fetch                download the saved list (multi-axis first) into the EroScripts library folder;
                         any length (a swiped scene is wanted); skips scenes rejected in the app (${prefs.DELETE_AT_OR_BELOW} stars or lower)
                         and waiting posts that break the content rules
  --ids 1,2,3            only these post ids (also with --frames)
  --auto                 only the posts swiped right in npm run scout:report (<userData>/${lib.SWIPES_FILE}) or wanted
                         on the Creators site (<userData>/${lib.CREATOR_SWIPES_FILE}: "eroscripts:<id>" posts
                         marked want; missing or unreadable: none), whatever their score or axes
  --parallel N           download up to N scenes at once (default 1; 1–${lib.MAX_PARALLEL}). Each host has its own limit so
                         none takes every slot: ${Object.entries(lib.HOST_SLOTS).filter(([h]) => h !== 'local').map(([h, n]) => `${h} ${n}`).join(', ')};
                         one scene folder is never written by two at once. Each scene prints a start and an end line.
                         Disk space (every download, also without --parallel): a video or archive starts only when
                         the library drive keeps ${prefs.MIN_FREE_GB} GB free after it (content-preferences.json swipeDeck.minFreeGb),
                         counting the downloads still running; else it's left for later and the next one is tried
                         (exit 2). Scripts aren't checked
  --passes N             repeat the queue until no failures, at most N times (default 1)
  --wait 1h              pause between passes (default 1h; also 90m, 3d)
  --dry-run              list what would be downloaded (or, with --retitle, renamed)
                         Files saved by hand (any host) are taken from --manual-dir or ~/Downloads first.
                         Videos on rule34video.com are read from their pages (paced, ≥ 400 ms apart), those on
                         pmvhaven.com from its API (≥ 1 s apart); both must fit the main script's length (checked
                         again with ffprobe; needs ffprobe on PATH).
                         Pornhub videos (view_video.php?viewkey=…) come through yt-dlp (on PATH; brew install yt-dlp):
                         the best quality within the caps, merged into an mp4, length checked like rule34video's;
                         with --dry-run their metadata is read instead. No yt-dlp: Pornhub links are skipped
                         vrbangers.com scenes come from the paid account through headless Google Chrome (logged in
                         once per pass; ≤ 8K, the planned file); with --dry-run the signed link is resolved and
                         probed for 1 byte instead
                         SexLikeReal scenes (only with --slr) come from the SLR account (<userData>/slr-session.json,
                         saved by npm run slr:scout -- --login): one of its video downloads each (35 per rolling 7
                         days; none left: SLR scenes wait, exit 2), the encode within the caps, never the trailer;
                         not logged in: SLR scenes are skipped (exit 3 when nothing else was done). With
                         --dry-run: the quota, the download type, its size and the target folder, no download
  --slr                  take SexLikeReal scenes too. Without it --fetch (also --auto, --ids, --dry-run, --daily's
                         fetch) and --manual-list leave them out and print how many: each costs one of the account's
                         weekly downloads, so the weekly npm run slr:scout -- --fetch hands them over in the user's
                         order, one post at a time (eroscripts:scout --fetch --ids <id> --slr)
  --vrb-credentials <f>  VRBangers account file (default: ${VRB_CREDENTIALS}; lines "Username: …", "password: …")
By hand (pixeldrain's hotlink protection needs a CAPTCHA in a browser):
  --manual-list          write ~/Downloads/download-for-fs.txt: the waiting posts' pixeldrain files with
                         their pages and instructions (with --ids: only those posts); no login needed.
                         With --slr, SexLikeReal videos are listed as "${slrHost.SLR_ACCOUNT_DOWNLOAD}" (--fetch --slr takes them)
  --manual-dir <dir>     where files downloaded by hand are saved (default ~/Downloads/for-fs)
Rename:
  --retitle              rename fetched scenes to "Artist - Title (metadata)" from their posts,
                         and the paths config.json keeps for them (app must be closed);
                         with --ids, only those posts' scenes
Frames (for the swipe deck's cards; free reads only):
  --frames [N]           waiting posts not in the library and not swiped yet (either way) whose card shows fewer than
                         ${frames.FRAMES_WANTED} usable images get frames of their first video at 25, 50 and 75 % (one eye for
                         stereo VR, ≤ ${frames.MAX_FRAME_WIDTH} px wide) in <userData>/${frames.FRAMES_DIR}/eroscripts/<id>/; posts
                         without images first, then by score; at most N posts (default ${frames.DEFAULT_FRAME_POSTS}).
                         Hosts: forum, rule34video, pmvhaven, pixeldrain (a hotlink-protected file fails), MEGA (stops at
                         its bandwidth limit), SexLikeReal's free trailer (never an account download),
                         VRBangers' public scene stills. ffmpeg reads a few MB per frame (≤ ${Math.round(frames.MAX_POST_BYTES / 1048576)} MB a post).
                         Each post is recorded in <userData>/${frames.FRAMES_FILE} (only --frames writes it;
                         each run first takes in records left on the scan state and frames folders without one).
                         A failed post is tried again after 7 days. Exit 2 when posts are left
Daily (run once a day):
  --daily                scan, then --fetch --auto (takes --passes, --wait, --parallel), each step under its own lock
                         (below); a step another run is doing is skipped (exit 2) and the next goes on; writes
                         <userData>/eroscripts-scout-daily.json. Frames are a separate run (--frames)
Run locks (in <userData>, holding the pid; a run whose lock another live run holds exits 2 and names it;
different kinds go on side by side):
  ${lib.SCOUT_LOCKS.scan.file.padEnd(29)}scan, --add, --rejudge, --titles (they write the scan state and the list)
  ${lib.SCOUT_LOCKS.fetch.file.padEnd(29)}--fetch (also --auto, --dry-run), --manual-list, --retitle (the library)
  ${lib.SCOUT_LOCKS.frames.file.padEnd(29)}--frames (${frames.FRAMES_FILE}, ${frames.FRAMES_DIR}/)
                         --status takes none. A lock whose pid has ended is taken over
Status:
  --status               print the scan state without going online: backfill per feed (page and oldest last
                         activity reached), verdicts, posts judged under older rules, the waiting list
  --titles               fill in title, author, dates and images of judged posts that lack them (judged before
                         verdicts kept them), waiting posts first, then newest first, one read per post at the
                         usual pace; verdicts stay. Waiting posts also get their first video's script read for
                         its length and heat strip. For npm run scout:report. A stopped run goes on next time
Both:
  --user-data <dir>      app data folder (default: $FUNSYNC_USER_DATA, else the desktop app's)
  --incoming <dir>       download folder (default: the EroScripts library source, else <library parent>/EroScripts)`;

function parseArgs(argv) {
  const opts = { pages: 6, animPages: 3, backfill: 2, top: 40, passes: 1, wait: 3600, fetch: false, dryRun: false, full: false, manualDir: path.join(DOWNLOADS, 'for-fs'), vrbCredentials: VRB_CREDENTIALS };
  const value = (i, flag) => {
    if (i + 1 >= argv.length) throw new Error(`${flag} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--fetch': opts.fetch = true; break;
      case '--frames': {
        const n = argv[i + 1];
        if (n !== undefined && /^\d+$/.test(n)) { opts.frames = Number(n); i++; } else opts.frames = frames.DEFAULT_FRAME_POSTS;
        break;
      }
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
      case '--parallel': opts.parallel = Number(value(i, a)); i++; break;
      case '--wait': opts.wait = parseInterval(value(i, a)); i++; break;
      case '--ids': opts.ids = new Set(value(i, a).split(',').map(Number)); i++; break;
      case '--add': opts.add = [...new Set(value(i, a).split(',').map((s) => Number(s.trim())).filter((n) => n > 0))]; i++; break;
      case '--feeds': opts.feeds = value(i, a).split(',').map((s) => s.trim()).filter(Boolean); i++; break;
      case '--since': opts.since = value(i, a); i++; break;
      case '--user-data': opts.userData = path.resolve(value(i, a)); i++; break;
      case '--incoming': opts.incoming = path.resolve(value(i, a)); i++; break;
      case '--vrbangers': opts.vrbangers = true; break;
      case '--slr': opts.slr = true; break;
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
  if (opts.parallel !== undefined) {
    if (!(Number.isInteger(opts.parallel) && opts.parallel >= 1 && opts.parallel <= lib.MAX_PARALLEL)) throw new Error(`--parallel must be 1 to ${lib.MAX_PARALLEL}`);
    if (!(opts.fetch || opts.daily)) throw new Error('--parallel goes with --fetch or --daily');
  }
  if (opts.slr && !(opts.fetch || opts.daily || opts.manualList)) throw new Error('--slr goes with --fetch, --daily or --manual-list');
  if (opts.rejudge && !opts.rejudge.length) throw new Error('--rejudge needs at least one verdict');
  if (opts.frames !== undefined && (opts.fetch || opts.daily || opts.add || opts.titles)) throw new Error('--frames runs on its own');
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
 * (one pseudo file per quality), pmvhaven pages (one pseudo file each), VRBangers scenes (one pseudo file each, the best quality within the VR caps:
 * a vrbangers.com/video link, a `VRBANGERS_<folder>…` file name, else the parts of a title naming VRBangers)
 * and SexLikeReal scenes (one pseudo file each: a sexlikereal.com/scenes link, else, for a post without a video
 * that names SLR, SLR's scene search on its title, a hit counting only when its length fits a main script read
 * now into `cache`).
 * `hostDown`: a rule34video or pmvhaven page or SLR couldn't be read for a reason other than being gone (403, 429, 5xx,
 * timeout), so a post without a video now may have one once the site answers again.
 */
async function topicFiles(api, pacer, topicId, cooked, title, tags = [], cache = new Map(), vr = false) {
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
  let hostDown = false;
  for (const [host, site, pages] of [
    [streams.RULE34VIDEO_HOST, rule34video, streams.rule34videoLinks(hrefs)],
    [streams.PMVHAVEN_HOST, pmvhaven, streams.pmvhavenLinks(hrefs)],
  ]) {
    if (pages.length) hosts.add(host);
    for (const u of pages) {
      try { files.push(...await site.files(u)); } catch (err) {
        hosts.add(`${host} (unreadable)`);
        if (err.status !== 404 && err.status !== 410) hostDown = true;
        process.stderr.write(`! ${topicId} ${err.message}\n`);
      }
    }
  }
  for (const u of phHost.pornhubLinks(hrefs)) {
    try {
      files.push(...await pornhub.files(u, { cap: heightCap(vr) }));
      hosts.add(phHost.PORNHUB_HOST);
    } catch (err) {
      if (err instanceof phHost.YtDlpMissingError) {
        // Not a verdict on the post: it stays pending until yt-dlp is installed.
        hosts.add('pornhub (yt-dlp missing)');
        hostDown = true;
      } else {
        hosts.add('pornhub (unreadable)');
        if (err.status !== 404) hostDown = true;
      }
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
  const slrLeads = slrHost.slrLeads({ hrefs, title, tags, text: cooked.replace(/<[^>]+>/g, ' '), hasVideo: files.some((f) => lib.isVideoName(f.name)) });
  if (slrLeads.length) {
    const scriptLengths = slrLeads.some((l) => l.from === 'search') ? await mainScriptLengths(api, pacer, files, cache) : [];
    const { files: scenes, missed, down } = await slr.filesFor(slrLeads, { scriptLengths, title });
    if (scenes.length) hosts.add(slrHost.SLR_HOST);
    if (down) { hostDown = true; hosts.add('slr (unreadable)'); }
    files.push(...scenes);
    for (const { lead, why } of missed) process.stderr.write(`! ${topicId} slr ${lead.from === 'link' ? lead.url : 'search'}: ${why}\n`);
  }
  const byName = new Map();
  for (const f of files) if (!byName.has(f.name)) byName.set(f.name, f);
  return { files: [...byName.values()], hosts: [...hosts], hostDown };
}

/** Length (ms) of each main script among a post's files (at most 4 read, through `cache`), to check SLR search hits against. */
async function mainScriptLengths(api, pacer, files, cache) {
  const out = [];
  for (const f of files.filter((x) => lib.scriptParts(x.name)?.axis === '').slice(0, 4)) {
    const ms = lib.scriptDurationMs(await parsedScript(api, pacer, f, cache));
    if (ms) out.push(ms);
  }
  return out;
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
 * A script file read and parsed, cached in `cache` by url (null = unreadable). Forum rate limits
 * and an expired session are thrown; other failures are an unreadable script.
 */
async function parsedScript(api, pacer, f, cache) {
  const key = `${f.url}|${f.remote || ''}`;
  if (!cache.has(key)) {
    let script = null;
    try {
      const bytes = await scriptBytes(api, pacer, f);
      if (bytes) script = JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/, ''));
    } catch (err) {
      if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
    }
    cache.set(key, script);
  }
  return cache.get(key);
}

/**
 * Set `durationMs` and `heat` (lib.scriptPreview) on the first scene, reading its main script, and
 * on every other scene whose main script `cache` already holds. Packed scenes have none to read.
 */
async function addScenePreviews(api, pacer, scenes, cache = new Map()) {
  for (const [i, sc] of scenes.entries()) {
    const main = lib.mainScriptOf(sc);
    if (!main || sc.packed) continue;
    if (i > 0 && !cache.has(`${main.url}|${main.remote || ''}`)) continue;
    Object.assign(sc, lib.scriptPreview(await parsedScript(api, pacer, main, cache)));
  }
  return scenes;
}

/**
 * A post's scenes (lib.planWithStreams): its scripts paired with its video
 * files; a scene with a streaming page (rule34video, pmvhaven, vrbangers.com,
 * sexlikereal.com) is checked by the length of its main script (read now,
 * or already in `scripts`); a rule34video one gets the chosen quality's
 * size, a pmvhaven one too when its API didn't give it (a vrbangers.com page gives it; an SLR scene's comes at --fetch).
 * Scenes get their script preview for the swipe deck (addScenePreviews).
 */
async function planTopic(api, pacer, topicId, files, { vr, libraryVideos, scripts = new Map() }) {
  const scriptMs = async (f) => lib.scriptDurationMs(await parsedScript(api, pacer, f, scripts)) || null;
  const plan = await lib.planWithStreams({
    scripts: files.filter((f) => /\.funscript$/i.test(f.name)),
    videos: files.filter((f) => lib.isVideoName(f.name)),
    vr, libraryVideos, scriptMs,
  });
  for (const d of plan.dropped) {
    process.stderr.write(`! ${topicId} ${d.url}: video ${clock(d.durationSec * 1000)}, script ${clock(d.scriptMs)}; not this scene's video\n`);
  }
  const sized = { [streams.RULE34VIDEO_HOST]: rule34video, [streams.PMVHAVEN_HOST]: pmvhaven };
  for (const sc of plan.scenes) {
    const site = sized[sc.video.host];
    if (!site || sc.video.size > 0) continue;
    let size = 0;
    try { size = await site.probeSize(sc.video); } catch { /* size unknown */ }
    sc.video = { ...sc.video, size };
  }
  await addScenePreviews(api, pacer, plan.scenes, scripts);
  return plan;
}

// ── Scan ────────────────────────────────────────────────────────────────

const statePath = (userData) => path.join(userData, 'eroscripts-scout-state.json');
const listPath = (userData) => path.join(userData, 'eroscripts-scout.json');

function readScoutState(userData) {
  try { return lib.normalizeScoutState(JSON.parse(fs.readFileSync(statePath(userData), 'utf8'))); } catch { return lib.emptyScoutState(); }
}

/**
 * Save the scan state (atomically: `--frames` reads it while a scan runs). Once the frames file exists, the
 * `frames` / `framesFailed` records still on state records are dropped: a `--frames` run copied them there.
 */
function writeScoutState(userData, state) {
  if (fs.existsSync(path.join(userData, frames.FRAMES_FILE))) lib.dropFrameRecords(state);
  writeAtomic(statePath(userData), JSON.stringify(state));
}

/** Save the waiting list (atomically: `--fetch` and `--frames` read it while a scan, `--add` or `--titles` writes it). */
function writeScoutList(userData, saved) {
  writeAtomic(listPath(userData), JSON.stringify(saved, null, 1));
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

/** A judged post as a saved candidate, scored with the user's tag scores and favourite artist bonus (lib.scoredCandidate). */
function makeCandidate({ t, s, author, artistInLibrary, favourite, hosts, scenes, first, picked = false }, scoring) {
  const axisType = scenes.some((sc) => sc.axes.length || sc.packed) ? 'multi' : 'single';
  const c = {
    id: t.id, title: t.title, url: `https://discuss.eroscripts.com/t/${t.id}`, author,
    tags: t.tags, likes: t.likes,
    vr: s.vr, music: s.music, axisType, artistInLibrary, hosts, live: s.live, favourite,
    ...(first.monster ? { monster: true } : {}), ...(first.feet ? { feet: true } : {}), ...(first.ffm ? { ffm: true } : {}), ...(picked ? { picked } : {}),
    scenes,
  };
  return lib.scoredCandidate(c, scoring);
}

async function scan(opts, { api, userData, config, incoming }) {
  const pacer = createPacer();
  const { library, knownTopics, turnedDown, favourites, scoring } = scoutContext(config, userData, incoming);
  const state = readScoutState(userData);
  const rejected = { known: 0, excluded: 0, blocked: 0, 'live-not-pov': 0, 'in-library': 0, 'no-video': 0 };
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
      const scripts = new Map();
      try { ({ files, hosts, hostDown } = await topicFiles(api, pacer, t.id, cooked, t.title, t.tags, scripts, s.vr)); } catch (err) {
        if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
        process.stderr.write(`! ${t.id} ${err.message}\n`);
        continue;
      }
      const plan = await planTopic(api, pacer, t.id, files, { vr: s.vr, libraryVideos: library.videos, scripts });
      // Scripts and videos packed in archives: the post's archives are scenes (unpacked at --fetch).
      const scenes = plan.scenes.length ? plan.scenes : lib.archiveScenes(files);
      // A video host (rule34video, SexLikeReal) refused or failed (not 404): no verdict, so the post stays pending and the next run reads it again.
      if (!scenes.length && hostDown) { process.stderr.write(`! ${t.id} ${hosts.filter((h) => h.endsWith('(unreadable)')).join(', ') || 'video host unreachable'}: left for the next run\n`); continue; }
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

  const out = listPath(userData);
  let previous = [];
  try { previous = JSON.parse(fs.readFileSync(out, 'utf8')).candidates || []; } catch { /* first scan */ }
  const done = fetchedScenes(incoming, userData);
  const candidates = lib.mergeCandidates(previous, fresh, (id, video) => done.has(lib.fetchedKey(id, video)), turnedDown, scoring, favourites);
  writeScoutList(userData, { generatedAt: new Date().toISOString(), listed, judged, rejected, candidates });

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
      const scripts = new Map();
      try { ({ files, hosts, hostDown } = await topicFiles(api, pacer, id, cooked, t.title, t.tags, scripts, s.vr)); } catch (err) {
        if (err instanceof RateLimitedError || err instanceof SessionExpiredError) throw err;
        console.log(`✗ ${id} ${t.title}: ${err.message}`);
        continue;
      }
      const plan = await planTopic(api, pacer, id, files, { vr: s.vr, libraryVideos: library.videos, scripts });
      const scenes = plan.scenes.length ? plan.scenes : lib.archiveScenes(files);
      if (!scenes.length) { console.log(`✗ ${id} ${t.title}: ${hostDown ? `${hosts.filter((h) => h.endsWith('(unreadable)')).join(', ') || 'video host unreachable'}, try again later` : plan.inLibrary > 0 ? 'in-library' : 'no-video'}`); continue; }
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
  const out = listPath(userData);
  let saved = { candidates: [] };
  try { saved = JSON.parse(fs.readFileSync(out, 'utf8')); } catch { /* no list yet */ }
  const done = fetchedScenes(incoming, userData);
  const candidates = lib.mergeCandidates(saved.candidates || [], fresh, (id, video) => done.has(lib.fetchedKey(id, video)), turnedDown, scoring, favourites);
  writeScoutList(userData, { ...saved, generatedAt: new Date().toISOString(), candidates });
  console.log(`\nAdded ${fresh.length} of ${opts.add.length}.${fresh.length ? ` Download them with: npm run ${opts.vrbangers ? 'vrbangers' : 'eroscripts'}:scout -- --fetch --ids ${fresh.map((c) => c.id).join(',')}` : ''}`);
  return 0;
}

// ── Fetch ───────────────────────────────────────────────────────────────

/** The `.part` files being written; removed if the run is stopped (Ctrl-C, kill). */
const activeParts = new Set();
for (const [sig, code] of [['SIGINT', 130], ['SIGTERM', 143]]) {
  process.on(sig, () => {
    for (const part of activeParts) fs.rmSync(part, { force: true });
    process.exit(code);
  });
}

/**
 * `cmd args` without blocking the downloads running beside it: spawnSync's result (`{status, stdout,
 * stderr, error}`); after `timeout` ms the child is killed (`error.code` ETIMEDOUT).
 */
function spawnAsync(cmd, args, { cwd, timeout } = {}) {
  return new Promise((resolve) => {
    const child = spawn(cmd, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    let error = null;
    const timer = timeout ? setTimeout(() => { error = Object.assign(new Error('timed out'), { code: 'ETIMEDOUT' }); child.kill('SIGKILL'); }, timeout) : null;
    child.stdout.on('data', (d) => { stdout += d; });
    child.stderr.on('data', (d) => { stderr += d; });
    child.on('error', (err) => { error ||= err; });
    child.on('close', (status) => {
      clearTimeout(timer);
      resolve({ status: error ? null : status, stdout, stderr, error });
    });
  });
}

/** Generated twist / roll / pitch for a single-axis scene folder (scripts/upgrade-multi-axis.mjs): null when made, else why not. */
async function generateAxes(dir) {
  const up = await spawnAsync(process.execPath, [path.join(REPO, 'scripts/upgrade-multi-axis.mjs'), dir], { cwd: REPO });
  return up.status === 0 ? null : (up.stderr || up.stdout || up.error?.message || '').trim().split('\n').pop();
}

/**
 * `fn()` once the host's earlier calls through here have settled. Its pacer (createPacer) spaces calls made
 * one after another only, so parallel downloads take their fresh links and streams from it in turn.
 */
const hostTurns = new Map();
function inTurn(host, fn) {
  const run = (hostTurns.get(host) || Promise.resolve()).then(fn);
  hostTurns.set(host, run.catch(() => {}));
  return run;
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
 * Download `file` to `dest` through `<staging>/<name>.<dest hash>.part` (the hash keeps parallel downloads of
 * same-named files in different scene folders apart). Every attempt
 * starts from scratch, so a failed one deletes its `.part` rather than leave
 * it behind. A copy the user saved by hand (`local`, see localDownloads) is
 * used instead of the network and removed from Downloads once it's in place.
 * A rule34video page (or a pmvhaven video, through its API) is read again for a fresh link, and the video must fit
 * the main script (`scriptMs`, else the page's length) before and after
 * the download (streams.downloadFits; ffprobe afterwards) and be whole (streams.wholeDownload: the Content-Length,
 * else within 1 % of the size pmvhaven's API gives now or the size probed at the scan). Other hosts' files and
 * copies saved by hand must have the exact size listed. A vrbangers.com
 * scene's signed link is read right now (logged-in Chrome), and the host
 * checks size and length itself (vrbangers-host.mjs download); so does a
 * SexLikeReal scene's (slr-host.mjs download: quota, download type, a fresh link).
 */
async function download(api, file, dest, staging, local = [], { scriptMs = null, vr = false } = {}) {
  if (fs.existsSync(dest)) return 'have';
  fs.mkdirSync(staging, { recursive: true });
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = path.join(staging, `${path.basename(dest)}.${createHash('sha1').update(dest).digest('hex').slice(0, 8)}.part`);
  const byHand = lib.findLocalCopy(file, local);
  const streamed = { [streams.RULE34VIDEO_HOST]: rule34video, [streams.PMVHAVEN_HOST]: pmvhaven }[file.host];
  let wantSize = file.size;
  let contentLength = 0;
  activeParts.add(tmp);
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
    } else if (streamed) {
      const now = await inTurn(file.host, () => streamed.fresh(file));
      if (file.host === streams.PMVHAVEN_HOST) wantSize = now.size;
      if (scriptMs && now.durationSec && !streams.downloadFits(now.durationSec, scriptMs)) {
        throw new Error(`${file.host}: video ${clock(now.durationSec * 1000)}, script ${clock(scriptMs)}; not this scene's video`);
      }
      const resp = await inTurn(file.host, () => streamed.stream(now.direct));
      contentLength = resp.headers.get('content-length');
      await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp));
    } else if (file.host === vrbHost.VRBANGERS_HOST) {
      await vrbangers.download(file, tmp, { scriptMs, videoSeconds });
    } else if (file.host === slrHost.SLR_HOST) {
      await slr.download(file, tmp, { scriptMs, videoSeconds });
    } else if (file.host === phHost.PORNHUB_HOST) {
      await pornhub.download(file, tmp, { cap: heightCap(vr), scriptMs, videoSeconds });
    } else {
      throw new Error(`unknown host ${file.host}`);
    }
    const size = fs.statSync(tmp).size;
    const selfChecked = file.host === vrbHost.VRBANGERS_HOST || file.host === slrHost.SLR_HOST;
    if (streamed && !byHand) {
      const broken = streams.wholeDownload(size, { contentLength, listed: wantSize });
      if (broken) throw new Error(`${file.host}: ${broken}`);
      const want = scriptMs || (file.duration ? file.duration * 1000 : null);
      const got = videoSeconds(tmp);
      if (!got) throw new Error(`${file.host}: the download isn't a readable video (ffprobe)`);
      if (want && !streams.downloadFits(got, want)) throw new Error(`${file.host}: downloaded video ${clock(got * 1000)}, script ${clock(want)}; not this scene's video`);
    } else if (wantSize && size !== wantSize && (byHand || !selfChecked)) throw new Error(`size ${size} ≠ ${wantSize}`);
    fs.renameSync(tmp, dest);
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    throw err;
  } finally {
    activeParts.delete(tmp);
  }
  if (byHand) {
    fs.rmSync(byHand.path, { force: true });
    local.splice(local.indexOf(byHand), 1);
    return 'local';
  }
  return 'ok';
}

/**
 * Scenes not to fetch: those with a sidecar and video, and every rejected one but those once turned down as too
 * short (since 2026-11-23 a swiped scene downloads whatever its length); `topics` (optional Set) gets the posts
 * with a sidecar.
 */
function fetchedScenes(incoming, userData, topics = null) {
  const done = new Set(rejectedVideos.readRejected(userData).entries.filter((e) => e.reason !== 'short').map((e) => e.key));
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

/**
 * One pass over the queue; downloads at most `room` videos (the library cap), up to `parallel` scenes at
 * once. A unit is one scene of a post (a packed one: its archive); lib.nextFetchUnit picks the next one in
 * queue order whose host has a slot left (lib.HOST_SLOTS) and whose scene folder no running unit holds, so a
 * second post bringing the same folder waits and then finds it in the library. Units running count against
 * `room`. SexLikeReal scenes need the account: once it isn't logged in (`slrStop: 'login'`) or has no video
 * download left (`'quota'`), the pass's later SLR scenes are skipped (`slrSkipped`) and the rest goes on.
 * A video or archive that would leave the library drive under MIN_FREE_GB free, counting the downloads in
 * flight (scripts/disk-space.mjs), is skipped and left for later (`noRoom`); the next one may fit. Not
 * logged in to vrbangers.com ends the pass: no unit starts any more, the running ones finish, then the
 * LoginError is thrown. With `parallel` > 1 each unit prints a start line as well as its end line.
 */
async function fetchPass(queue, { api, userData, incoming, staging, room, manualDir, parallel = 1 }) {
  const done = fetchedScenes(incoming, userData);
  // Files saved by hand: posts with one go first, and the folders are read
  // again before every scene, so a file saved during a long pass is taken
  // when its scene comes up, not a pass later.
  const readLocal = () => localDownloads([manualDir, DOWNLOADS]);
  const saved = readLocal();
  const byHand = (c) => c.scenes.some((sc) => [sc.video, ...sc.scripts].some((f) => lib.findLocalCopy(f, saved)));
  const order = [...queue].sort((a, b) => byHand(b) - byHand(a));
  const r = { ok: 0, failed: 0, generated: 0, capped: false, slrStop: null, slrSkipped: 0, noRoom: 0 };
  const now = () => new Date().toISOString().slice(11, 19);
  const minFree = prefs.MIN_FREE_GB * 1e9;
  const pending = order.flatMap((c) => c.scenes.map((sc) => {
    if (sc.packed) return { c, sc, name: `${lib.creatorFolder(c.title, c.author)}/${sc.video.name}`, dir: path.join(staging, `${sc.base}.unpacked`) };
    const { creator, stem } = lib.sceneNameOf(c, sc);
    const dir = path.join(incoming, creator, stem);
    return { c, sc, stem, name: `${creator}/${stem}`, dir, video: path.join(dir, stem + path.extname(sc.video.name).toLowerCase()) };
  }));
  const running = new Set();
  // A folder another running unit is writing: fetchPacked leaves it alone like one already in the library.
  const busy = (u) => (dir) => [...running].some((o) => o !== u && o.dir === dir);
  let loginError = null;

  const runScene = async (u, local) => {
    const { c, sc, dir, stem, video } = u;
    try {
      // The account first (session, a download left), so a scene it can't fetch leaves no scripts behind.
      if (sc.video.host === slrHost.SLR_HOST && !lib.findLocalCopy(sc.video, local)) await slr.ready();
      for (const f of sc.scripts) await download(api, f, path.join(dir, lib.scriptFileName(stem, f.axis)), staging, local);
      let lengthMs = null;
      try { lengthMs = lib.scriptDurationMs(JSON.parse(fs.readFileSync(path.join(dir, lib.scriptFileName(stem, '')), 'utf8'))); } catch { /* unreadable script: download anyway */ }
      const got = await download(api, sc.video, video, staging, local, { scriptMs: lengthMs, vr: c.vr === true });
      writeAtomic(path.join(dir, 'eroscripts.json'), JSON.stringify(
        lib.sceneSidecar(c, sc.video, path.basename(video), new Date().toISOString()), null, 1));
      let note = '';
      if (!sc.axes.length) {
        const why = await generateAxes(dir);
        if (why === null) { r.generated++; note = ' + generated axes'; } else note = ` (axis generation failed: ${why})`;
      }
      const kind = c.favourite && c.live ? ` [favourite: ${c.favourite}]` : '';
      console.log(`${now()} ✓ ${u.name}${kind}${note}${got === 'local' ? ' (from Downloads)' : ''}`);
      r.ok++;
    } catch (err) {
      // Not logged in to vrbangers.com: every later VRBangers scene would fail the same way.
      if (err instanceof LoginError) throw err;
      // Not logged in to SexLikeReal, or no download left: the pass's other SLR scenes wait, the rest goes on.
      if (err instanceof slrHost.SlrLoginError || err instanceof slrHost.SlrQuotaError) {
        r.slrStop = err instanceof slrHost.SlrQuotaError ? 'quota' : 'login';
        r.slrSkipped++;
        console.log(`${now()} – ${u.name}: ${err.message}; SexLikeReal scenes skipped from here`);
        return;
      }
      console.log(`${now()} ✗ ${u.name}: ${err.message}`);
      r.failed++;
    }
  };
  const runPacked = async (u, local) => {
    try {
      const p = await fetchPacked(u.c, u.sc, { api, userData, incoming, staging, local, now, busy: busy(u) });
      r.ok += p.ok; r.generated += p.generated;
    } catch (err) {
      console.log(`${now()} ✗ ${u.name}: ${err.message}`);
      r.failed++;
    }
  };
  const oneFinished = () => Promise.race([...running].map((u) => u.promise));

  while (!loginError) {
    const local = readLocal();
    for (let i = 0; i < pending.length;) {
      const u = pending[i];
      if (done.has(lib.fetchedKey(u.c.id, u.sc.video.name))) pending.splice(i, 1);
      else if (r.slrStop && u.sc.video.host === slrHost.SLR_HOST) { r.slrSkipped++; pending.splice(i, 1); } else {
        u.host = lib.findLocalCopy(u.sc.video, local) ? 'local' : u.sc.video.host;
        i++;
      }
    }
    if (!pending.length) break;
    if (r.ok + running.size >= room) {
      if (!running.size) { r.capped = true; break; }
      await oneFinished();
      continue;
    }
    const i = lib.nextFetchUnit(pending, { running: [...running], parallel });
    if (i < 0) { await oneFinished(); continue; }
    const [u] = pending.splice(i, 1);
    // Another post already brought this scene (same folder and video file): leave the
    // folder alone. Its scripts, sidecar and length were judged when it came, so this
    // post's files and the current length rule don't apply to it.
    if (!u.sc.packed && fs.existsSync(u.video)) { console.log(`${now()} = ${u.name}: already in the library from another post; left as is`); continue; }
    const inFlight = [...running].reduce((n, o) => n + (o.sc.video.size || 0), 0);
    const short = disk.roomFor({ dir: incoming, bytes: u.sc.video.size, inFlight, minFree });
    if (short) { console.log(`${now()} – ${u.name}: ${short}; left for later`); r.noRoom++; continue; }
    const size = u.sc.video.size ? `, ${(u.sc.video.size / 1e9).toFixed(1)} GB` : '';
    if (parallel > 1) console.log(`${now()} → ${u.name}: ${u.host === 'local' ? 'saved by hand' : u.host}${size}`);
    running.add(u);
    u.promise = (u.sc.packed ? runPacked(u, local) : runScene(u, local))
      .catch((err) => { loginError ||= err; })
      .finally(() => running.delete(u));
  }
  await Promise.all([...running].map((u) => u.promise));
  if (loginError) throw loginError;
  return r;
}

/**
 * Unpack `archive` into `out`: unar (RAR 5 included), else bsdtar. Throws
 * when neither can. No stdin and a time limit: bsdtar asks for the password
 * of an encrypted ZIP on the terminal and would wait for it forever.
 */
async function unpack(archive, out) {
  const tries = [['unar', ['-q', '-f', '-D', '-o', out, archive]], ['bsdtar', ['-xf', archive, '-C', out]]];
  const errors = [];
  for (const [cmd, args] of tries) {
    const r = await spawnAsync(cmd, args, { timeout: 30 * 60 * 1000 });
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
 * removed once filed; a failed unpack keeps the archive in staging for the next pass. A scene folder
 * another running download is writing (`busy(dir)`) is left alone like one already in the library.
 */
async function fetchPacked(c, sc, { api, userData, incoming, staging, local, now, busy = () => false }) {
  const archive = path.join(staging, sc.video.name);
  const got = await download(api, sc.video, archive, staging, local);
  const out = path.join(staging, `${sc.base}.unpacked`);
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });
  const r = { ok: 0, generated: 0 };
  try {
    await unpack(archive, out);
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
      const held = fs.existsSync(video) ? 'already in the library' : busy(dir) ? 'being downloaded for another post' : null;
      if (held) { console.log(`${now()} = ${creator}/${stem}: ${held}; left as is`); continue; }
      fs.mkdirSync(dir, { recursive: true });
      for (const f of s.scripts) fs.renameSync(f.path, path.join(dir, lib.scriptFileName(stem, f.axis)));
      fs.renameSync(s.video.path, video);
      writeAtomic(path.join(dir, 'eroscripts.json'), JSON.stringify({
        topicId: c.id, url: c.url, title: c.title, author: c.author, tags: c.tags,
        video: sc.video.name, archiveEntry: s.video.name, file: path.basename(video), scenes: c.scenes.length, at: new Date().toISOString(),
      }, null, 1));
      let note = '';
      if (!s.axes.length) {
        const why = await generateAxes(dir);
        if (why === null) { r.generated++; note = ' + generated axes'; } else note = ` (axis generation failed: ${why})`;
      }
      console.log(`${now()} ✓ ${creator}/${stem}${note} (unpacked from ${sc.video.name}${got === 'local' ? ', saved by hand' : ''})`);
      r.ok++;
    }
    // Nothing filed: no sidecar names this archive, so record it, or every pass and run fetches it again.
    if (!r.ok) reject('duplicate');
    fs.rmSync(archive, { force: true });
    return r;
  } finally {
    fs.rmSync(out, { recursive: true, force: true });
  }
}

/**
 * The posts swiped in npm run scout:report's deck (lib.readSwipes): none when the file isn't there; an
 * unreadable one is reported and counts as none.
 */
function readSwipesFile(userData) {
  const file = path.join(userData, lib.SWIPES_FILE);
  let text;
  try { text = fs.readFileSync(file, 'utf8'); } catch { return lib.readSwipes(null); }
  try { return lib.readSwipes(JSON.parse(text)); } catch (err) {
    console.error(`Ignoring ${file}: ${err.message}`);
    return lib.readSwipes(null);
  }
}

/**
 * `--auto`'s posts: those swiped right in the swipe deck (readSwipesFile) and the EroScripts posts wanted on
 * the Creators site (<userData>/creator-swipes.json, lib.creatorWants; missing or unreadable: none).
 */
function wantedPosts(userData) {
  const { wanted } = readSwipesFile(userData);
  let creators = new Set();
  try { creators = lib.creatorWants(JSON.parse(fs.readFileSync(path.join(userData, lib.CREATOR_SWIPES_FILE), 'utf8'))); } catch { /* none */ }
  return new Set([...wanted, ...creators]);
}

/**
 * The saved list as a download queue (content-rule breaks dropped, `--ids` applied; `--vrbangers`: posts with a
 * vrbangers.com video only; `--auto`: only posts swiped right or wanted on the Creators site, wantedPosts and
 * lib.autoFetchPick), checked again against the
 * library as it is now (lib.notYetHeld). Without `--slr` its SexLikeReal scenes are left out (lib.leaveOutSlr):
 * they cost the account's weekly quota, and the weekly SLR run takes them post by post.
 */
function savedQueue(opts, { userData, config, incoming }) {
  const file = listPath(userData);
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
  const wanted = opts.auto ? wantedPosts(userData) : new Set();
  const picked = lib.downloadQueue(candidates, scoring, favourites)
    .filter((c) => (!opts.ids || opts.ids.has(c.id)) && (!opts.vrbangers || lib.isVrbangersCandidate(c)) && (!opts.auto || lib.autoFetchPick(c, wanted)));
  const { library, knownTopics, turnedDown } = scoutContext(config, userData, incoming);
  const ownTopics = new Set();
  const done = fetchedScenes(incoming, userData, ownTopics);
  const held = lib.notYetHeld(picked, { knownTopics, ownTopics, turnedDown, libraryVideos: library.videos });
  if (held.posts || held.scenes) console.log(`Skipped what the library holds now: ${held.posts} posts, ${held.scenes} more scenes`);
  if (opts.slr) return held.queue;
  const left = lib.leaveOutSlr(held.queue, (id, video) => done.has(lib.fetchedKey(id, video)));
  if (left.scenes) console.log(`Left for the weekly SLR run (--slr): ${left.scenes} SexLikeReal video(s) in ${left.posts} post(s)`);
  return left.queue;
}

/** Write ~/Downloads/download-for-fs.txt: the waiting posts' files to download by hand. */
function manualList(opts, { userData, config, incoming }) {
  const done = fetchedScenes(incoming, userData);
  const posts = lib.manualDownloads(savedQueue(opts, { userData, config, incoming }), (id, video) => done.has(lib.fetchedKey(id, video)));
  fs.mkdirSync(opts.manualDir, { recursive: true });
  const out = path.join(DOWNLOADS, 'download-for-fs.txt');
  fs.writeFileSync(out, lib.manualListText(posts, { folder: opts.manualDir }));
  const account = posts.reduce((n, p) => n + p.account.length, 0);
  console.log(`${out}: ${posts.length} posts, ${posts.reduce((n, p) => n + p.files.length, 0)} files to download by hand into ${opts.manualDir}${account ? `; ${account} SexLikeReal videos --fetch takes with the account` : ''}`);
  return 0;
}

async function fetchAll(opts, { api, userData, config, incoming }) {
  const queue = savedQueue(opts, { userData, config, incoming });
  const gb = queue.reduce((n, c) => n + c.scenes.reduce((m, sc) => m + (sc.video.size || 0), 0), 0) / 1e9;
  console.log(`Queue: ${queue.length} posts, ${queue.reduce((n, c) => n + c.scenes.length, 0)} videos, ${gb.toFixed(1)} GB → ${incoming}`);
  if (opts.dryRun) {
    for (const c of queue) console.log(`  ${c.axisType.padEnd(6)} ${c.id} ${c.title}`);
    const vrb = await probeVrbangers(queue, { userData, incoming });
    const slrExit = await planSlr(queue, { userData, incoming });
    return Math.max(vrb, slrExit, await planPornhub(queue, { userData, incoming }));
  }
  ensureLibrarySource(userData, { name: FOLDER, path: incoming });
  const staging = path.join(path.dirname(incoming), '_staging');
  let last = null;
  let did = false;
  for (let pass = 1; pass <= Math.max(1, opts.passes); pass++) {
    const held = libraryVideoCount(readConfig(userData));
    if (held >= prefs.LIBRARY_MAX_VIDEOS) {
      console.log(`Library holds ${held} videos, the cap is ${prefs.LIBRARY_MAX_VIDEOS} (LIBRARY_MAX_VIDEOS): nothing more downloaded. Delete scenes or raise the cap.`);
      return 2;
    }
    console.log(`Pass ${pass}/${Math.max(1, opts.passes)} (library ${held}/${prefs.LIBRARY_MAX_VIDEOS} videos)`);
    // Chrome (vrbangers.com) starts with the pass's first VRBangers scene and is closed when the pass ends, failed or not.
    const r = await vrbangers.run(() => fetchPass(queue, { api, userData, incoming, staging, room: prefs.LIBRARY_MAX_VIDEOS - held, manualDir: opts.manualDir, parallel: opts.parallel ?? 1 }));
    const slrWaiting = r.slrSkipped ? `; ${r.slrSkipped} SexLikeReal scene${r.slrSkipped > 1 ? 's' : ''} waiting (${r.slrStop === 'quota' ? 'no video download left on the account' : 'not logged in to SexLikeReal'})` : '';
    const noRoom = r.noRoom ? `; ${r.noRoom} left for later: not enough disk space (${prefs.MIN_FREE_GB} GB kept free)` : '';
    console.log(`Pass ${pass}: ${r.ok} downloaded (${r.generated} with generated axes), ${r.failed} failed${r.capped ? `; stopped at the library cap (${prefs.LIBRARY_MAX_VIDEOS} videos)` : ''}${slrWaiting}${noRoom}`);
    if (r.ok) dedupeAfterFetch(userData);
    last = r;
    did ||= r.ok + r.failed > 0;
    if (r.capped || !r.failed) break;
    if (pass < opts.passes) {
      console.log(`Waiting ${opts.wait / 60} min for host quotas`);
      await new Promise((resolve) => setTimeout(resolve, opts.wait * 1000));
    }
  }
  if (last.capped || last.failed || last.noRoom) return 2;
  // SexLikeReal scenes left: not logged in is 3 when they were all there was to do; no download left is work left.
  if (last.slrStop === 'login') return did ? 2 : 3;
  return last.slrStop ? 2 : 0;
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

/**
 * `--fetch --dry-run`: the SexLikeReal account's video downloads left and,
 * for each waiting SLR scene, the download it would take (type, size, read
 * from the scene's download options with the account) and where it goes. No
 * download link is asked for: that request uses up one of the downloads.
 * Not logged in: 3; scenes that don't fit the quota or the caps: 2.
 */
async function planSlr(queue, { userData, incoming }) {
  const done = fetchedScenes(incoming, userData);
  const scenes = queue.flatMap((c) => c.scenes.filter((sc) => sc.video.host === slrHost.SLR_HOST && !done.has(lib.fetchedKey(c.id, sc.video.name))).map((sc) => ({ c, sc })));
  if (!scenes.length) return 0;
  console.log(`sexlikereal.com: planning ${scenes.length} download${scenes.length > 1 ? 's' : ''} (download options only; no download link asked for, nothing used up)`);
  let quota;
  try { quota = await slr.quota(); } catch (err) {
    if (err instanceof slrHost.SlrLoginError) { console.log(`  ${err.message}`); return 3; }
    throw err;
  }
  const short = Math.max(0, scenes.length - Math.max(0, quota.remaining));
  console.log(`  ${quota.remaining} of ${quota.total ?? '?'} video downloads left (rolling 7 days)${short ? `; ${short} scene${short > 1 ? 's' : ''} would wait` : ''}`);
  let failed = 0;
  for (const { c, sc } of scenes) {
    const { creator, stem } = lib.sceneNameOf(c, sc);
    const target = path.join(incoming, creator, stem, stem + path.extname(sc.video.name).toLowerCase());
    try {
      const pick = await slr.plan(sc.video);
      console.log(`  ${c.id} ${sc.video.url}\n    ${pick.type}: ${pick.width ? `${pick.width}×` : ''}${pick.height} ${pick.codec || ''}, ${pick.size ? `${(pick.size / 1e9).toFixed(2)} GB` : 'size unknown'}\n    → ${target}`);
    } catch (err) {
      console.log(`  ${c.id} ${creator}/${stem}: ✗ ${err.message}`);
      failed++;
    }
  }
  return failed || short ? 2 : 0;
}

/**
 * `--fetch --dry-run`: each waiting Pornhub scene's metadata read again with
 * yt-dlp (nothing downloaded): the height the download gets within the caps,
 * the video's length against the main script's, and where it goes. yt-dlp
 * missing, a video gone or a length that doesn't fit: 2.
 */
async function planPornhub(queue, { userData, incoming }) {
  const done = fetchedScenes(incoming, userData);
  const scenes = queue.flatMap((c) => c.scenes.filter((sc) => sc.video.host === phHost.PORNHUB_HOST && !done.has(lib.fetchedKey(c.id, sc.video.name))).map((sc) => ({ c, sc })));
  if (!scenes.length) return 0;
  console.log(`pornhub.com: reading ${scenes.length} video${scenes.length > 1 ? 's' : ''} with yt-dlp (metadata only, nothing downloaded)`);
  let failed = 0;
  for (const { c, sc } of scenes) {
    const { creator, stem } = lib.sceneNameOf(c, sc);
    const target = path.join(incoming, creator, stem, stem + path.extname(sc.video.name).toLowerCase());
    const cap = heightCap(c.vr === true);
    try {
      const info = await pornhub.info(sc.video.url, cap);
      const scriptMs = sc.durationMs || null;
      const fits = phHost.pornhubFits(info.durationSec, scriptMs);
      console.log(`  ${c.id} ${sc.video.url}\n    ${info.height}p (cap ${cap}p, -f "${phHost.pornhubFormat(cap)}"), ${info.durationSec ? clock(info.durationSec * 1000) : 'length unknown'}${scriptMs ? `, script ${clock(scriptMs)}${fits ? '' : ': not this scene\'s video'}` : ''}\n    → ${target}`);
      if (!fits) failed++;
    } catch (err) {
      console.log(`  ${c.id} ${sc.video.url}: ✗ ${err.message}`);
      failed++;
      if (err instanceof phHost.YtDlpMissingError) break;
    }
  }
  return failed ? 2 : 0;
}

// ── Frames ──────────────────────────────────────────────────────────────

/**
 * `--frames N`: frames for the swipe deck (scripts/scout-frames.mjs). First the frames file
 * (`<userData>/eroscripts-scout-frames.json`) takes in what earlier runs left (frames.migrateFrames:
 * records still on the scan state, folders on disk without a record). Then the waiting posts
 * frames.framesQueue picks (not in the library: lib.notYetHeld and the scout's own sidecars; not swiped
 * either way; fewer than FRAMES_WANTED usable images; no frames or recent failure), at most `opts.frames`
 * of them read, each recorded in the frames file (`frames` / `framesFailed`) as it's done. The scan state
 * and the list are only read, so this runs beside a scan or a download (its own lock). A host whose
 * allowance runs out (MEGA's bandwidth) is skipped for the rest of the run. Exit 2 when posts are left.
 */
async function makeFrames(opts, { api, userData, config, incoming }) {
  const state = readScoutState(userData);
  const framesFile = frames.readFramesFile(userData);
  const moved = frames.migrateFrames(framesFile, { state, onDisk: frames.framesOnDisk(userData) });
  if (moved.fromState || moved.fromDisk) {
    frames.writeFramesFile(userData, framesFile);
    console.log(`Frames: ${frames.FRAMES_FILE} took in ${moved.fromState} records from the scan state and ${moved.fromDisk} frames folders without a record`);
  }
  const listFile = listPath(userData);
  let candidates;
  try { candidates = JSON.parse(fs.readFileSync(listFile, 'utf8')).candidates || []; } catch {
    console.log(`No scout list at ${listFile}; no frames to make.`);
    return 0;
  }
  const { wanted, hidden } = readSwipesFile(userData);
  const { library, knownTopics, turnedDown } = scoutContext(config, userData, incoming);
  const ownTopics = new Set();
  fetchedScenes(incoming, userData, ownTopics);
  const open = new Set(lib.notYetHeld(candidates, { knownTopics, turnedDown, libraryVideos: library.videos }).queue.map((c) => c.id));
  const queue = frames.framesQueue(candidates.filter((c) => !opts.ids || opts.ids.has(c.id)), {
    state, framesFile, swiped: new Set([...wanted, ...hidden]), inLibrary: (c) => ownTopics.has(c.id) || !open.has(c.id),
  });
  console.log(`Frames: ${queue.length} unswiped waiting posts show fewer than ${frames.FRAMES_WANTED} images; reading at most ${opts.frames}`);
  const { File } = await import('megajs');
  const cookie = api._headers().Cookie;
  const deps = { File, rule34video, pmvhaven, slr, forumHeaders: cookie ? { Cookie: cookie } : {} };
  const pacer = createPacer();
  const stopped = new Map();
  const failed = {};
  const skipped = {};
  let made = 0; let frameCount = 0; let tried = 0; let left = 0; let bytes = 0;
  const started = Date.now();
  const mb = (n) => `${(n / 1048576).toFixed(1)} MB`;
  for (const { c } of queue) {
    const plan = frames.frameSource(c);
    if (plan.skip) { skipped[plan.skip] = (skipped[plan.skip] || 0) + 1; continue; }
    if (tried >= opts.frames || stopped.has(plan.host)) { left++; continue; }
    tried++;
    if (plan.forum) await pacer.wait();
    const dir = frames.postFramesDir(userData, c.id);
    const t0 = Date.now();
    try {
      const r = await frames.postFrames(c, dir, deps);
      frames.recordFrames(framesFile, c.id, { source: r.source, files: r.files });
      made++;
      frameCount += r.files.length;
      bytes += r.bytes || 0;
      if (r.stopHost) stopped.set(plan.host, 'its allowance ran out');
      console.log(`  ✓ ${c.id} ${r.source}: ${r.files.length} frame${r.files.length > 1 ? 's' : ''}${r.bytes != null ? `, ${mb(r.bytes)}` : ''}, ${((Date.now() - t0) / 1000).toFixed(1)} s`);
    } catch (err) {
      const reason = err instanceof frames.FrameError ? err.reason : `${plan.host}: ${err.message}`;
      frames.recordFramesFailed(framesFile, c.id, reason);
      fs.rmSync(dir, { recursive: true, force: true });
      const key = reason.replace(/\s*\(.*$/, '');
      failed[key] = (failed[key] || 0) + 1;
      if (err.stopHost) stopped.set(plan.host, reason);
      console.log(`  ✗ ${c.id} ${plan.host}: ${reason} (${((Date.now() - t0) / 1000).toFixed(1)} s)`);
    }
    frames.writeFramesFile(userData, framesFile);
  }
  const list = (o) => Object.entries(o).sort((a, b) => b[1] - a[1]).map(([k, n]) => `${k} ${n}`).join(', ');
  const nFailed = Object.values(failed).reduce((n, k) => n + k, 0);
  console.log(`Frames: ${made} posts got ${frameCount} frames (${mb(bytes)} read, ${clock(Date.now() - started)}), ${nFailed} failed${nFailed ? ` (${list(failed)})` : ''}`);
  if (Object.keys(skipped).length) console.log(`  skipped, no free source: ${list(skipped)}`);
  for (const [host, why] of stopped) console.log(`  ${host} stopped for this run: ${why}`);
  if (left) console.log(`  ${left} posts left for the next run`);
  return left ? 2 : 0;
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

// ── Status, daily run, run locks ────────────────────────────────────────

const DAILY_FILE = 'eroscripts-scout-daily.json';

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
  const wanted = wantedPosts(userData);
  for (const line of lib.scoutStatus(state, { candidates, scoring, wanted, daily })) console.log(line);
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
 * `--titles`: title, author, dates and images for judged posts that lack them (lib.topicsMissingInfo:
 * the waiting posts first, best score first, then newest first), paced like a scan, and the script
 * preview of each waiting post's first scene (lib.candidatesMissingPreview; one more read). A post the
 * forum no longer has (404) is marked `gone`; a run that hits the rate limit or a run of refusals
 * (expired session) stops, saving what it has.
 */
async function fillTitles({ api, userData }) {
  const pacer = createPacer();
  const state = readScoutState(userData);
  let saved = null;
  try { saved = JSON.parse(fs.readFileSync(listPath(userData), 'utf8')); } catch { /* no list yet */ }
  const waiting = saved?.candidates || [];
  const ids = lib.topicsMissingInfo(state, waiting.map((c) => c.id));
  // Waiting posts without a script preview (length, heat strip): read with their post, in deck order.
  const previewless = new Map(lib.candidatesMissingPreview(waiting).map((c) => [c.id, c]));
  console.log(`${ids.length} judged posts without a title or images, ${previewless.size} waiting posts without a script preview`);
  let done = 0; let gone = 0; let refused = 0; let skipped = 0; let previews = 0;
  const scripts = new Map();
  const preview = async (c) => {
    if (!previewless.delete(c.id)) return;
    await addScenePreviews(api, pacer, c.scenes.slice(0, 1), scripts);
    scripts.clear();
    previews++;
  };
  const save = () => {
    writeScoutState(userData, state);
    if (saved && previews) writeScoutList(userData, saved);
  };
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
      if (previewless.has(id)) await preview(previewless.get(id));
      if (++done % 50 === 0) {
        save();
        console.log(`  ${done} of ${ids.length}`);
      }
    }
    for (const c of [...previewless.values()]) {
      await preview(c);
      if (previews % 50 === 0) {
        save();
        console.log(`  ${previews} script previews`);
      }
    }
  } finally {
    save();
    console.log(`Titles: ${done} filled in, ${gone} posts gone, ${skipped} unreadable (tried again next run); ${previews} script previews`);
  }
  return skipped ? 2 : 0;
}

/**
 * `--daily`: a scan (new posts plus the usual backfill), then `--fetch --auto`, each step holding its own
 * run lock (lib.SCOUT_LOCKS) only while it runs. A step whose lock another run holds is skipped, exit 2
 * (that run is doing the work), and the next step goes on. `<userData>/eroscripts-scout-daily.json`
 * records when each step started, its exit code and, for a skipped step, the busy lock and its holder
 * (`<step>Skipped`); it's written after each step, first right after the scan, so the day's run shows as
 * done while the downloads go on. Frames are a separate run (`--frames`).
 */
async function daily(opts, ctx) {
  const file = path.join(ctx.userData, DAILY_FILE);
  const record = {};
  const step = async (key, kind, run) => {
    record[`${key}At`] = new Date().toISOString();
    const holder = takeRunLock(ctx.userData, kind);
    if (holder) {
      console.log(`Daily: ${lib.lockBusyMessage(kind, describeHolder(holder))}; step skipped`);
      record[`${key}Exit`] = 2;
      record[`${key}Skipped`] = `${lib.SCOUT_LOCKS[kind].file} held by ${describeHolder(holder)}`;
    } else {
      try { record[`${key}Exit`] = await run(); } finally { freeRunLock(ctx.userData, kind); }
    }
    writeAtomic(file, JSON.stringify(record, null, 1));
    return record[`${key}Exit`];
  };
  const scanExit = await step('scan', 'scan', () => exitCodeOf(() => scan(opts, ctx)));
  if (scanExit === 3) return 3;
  console.log('\nDaily: downloading the waiting posts swiped right');
  const fetchExit = await step('fetch', 'fetch', () => exitCodeOf(() => fetchAll({ ...opts, fetch: true, auto: true }, ctx)));
  return scanExit || fetchExit;
}

/**
 * Take the run lock of `kind` (lib.SCOUT_LOCKS: scan, fetch or frames): null when this run holds
 * `<userData>/<lock file>` (`<pid>`, freed at exit or by freeRunLock), else the holder of the live run
 * that does. A lock whose pid has ended is taken over (scripts/host-env.mjs).
 */
function takeRunLock(userData, kind) {
  return takePidLock(path.join(userData, lib.SCOUT_LOCKS[kind].file));
}

function freeRunLock(userData, kind) {
  releasePidLock(path.join(userData, lib.SCOUT_LOCKS[kind].file));
}

/** Take `kind`'s run lock for the whole run; false, after naming the run that has it, when it's busy. */
function lockRun(userData, kind) {
  const holder = takeRunLock(userData, kind);
  if (holder) console.error(`${lib.lockBusyMessage(kind, describeHolder(holder))}; try again when it ends.`);
  return !holder;
}

// ── Main ────────────────────────────────────────────────────────────────

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserDataDir();
  const config = readConfig(userData);
  const incoming = downloadFolder(config, FOLDER, opts.incoming).path;
  if (opts.status) return status({ userData, config, incoming });
  const kind = lib.scoutLockFor(opts);
  if (opts.retitle || opts.manualList) {
    if (!lockRun(userData, kind)) return 2;
    return opts.retitle ? retitle(opts, { userData, incoming }) : manualList(opts, { userData, config, incoming });
  }
  const session = config.settings?.eroscripts?.session || {};
  if (!session.cookie) { console.error('Not logged in to EroScripts. Log in from the app (EroScripts panel), then re-run.'); return 3; }
  if (kind && !lockRun(userData, kind)) return 2;

  // The API module logs through electron-log; keep the CLI's output clean.
  const log = require('../electron/logger.js');
  log.transports.file.level = false;
  log.transports.console.level = false;
  const { EroScriptsAPI } = require('../electron/eroscripts-api.js');
  const api = new EroScriptsAPI();
  api.restoreSession(session.cookie, session.username);
  vrbangers = vrbHost.createVrbangers({ userData, credentialsFile: opts.vrbCredentials, log: (m) => process.stderr.write(`  vrbangers.com: ${m}\n`) });
  slr = slrHost.createSlr({ userData, log: (m) => process.stderr.write(`  sexlikereal.com: ${m}\n`) });

  const ctx = { api, userData, config, incoming };
  if (opts.daily) return daily(opts, ctx);
  if (opts.frames !== undefined) return exitCodeOf(() => makeFrames(opts, ctx));
  if (opts.add) return exitCodeOf(() => addPicked(opts, ctx));
  if (opts.titles) return exitCodeOf(() => fillTitles(ctx));
  return exitCodeOf(() => (opts.fetch ? fetchAll(opts, ctx) : scan(opts, ctx)));
}

main().then((code) => process.exit(code), (err) => {
  console.error(`eroscripts:scout: ${err.message}`);
  process.exit(1);
});
