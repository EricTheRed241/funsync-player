// Frames for the swipe deck (npm run scout:report): a waiting post whose card
// shows fewer than FRAMES_WANTED usable images gets up to three JPEG frames
// of its video, taken at 25, 50 and 75 % of its length, in
// `<userData>/scout-frames/eroscripts/<topicId>/{25,50,75}.jpg` (the deck
// serves them as `/frames/eroscripts/<id>/<n>.jpg`). Run by
// `npm run eroscripts:scout -- --frames [N]` and `--daily`
// (scripts/eroscripts-scout.mjs).
//
// Only free reads: forum attachments, rule34video (a fresh player link, the
// lowest quality that still shows the scene), pixeldrain's file API (a 403 is
// its hotlink protection: the post is marked failed, nothing else), MEGA
// (ranged megajs downloads; a bandwidth error stops MEGA for the run), the
// SexLikeReal scene's public trailer encodings (never the account's full
// download, which uses up the weekly quota) and VRBangers' public scene
// gallery (stills, no video: its videos need the paid account).
//
// ffmpeg reads each video through a short-lived HTTP range proxy on
// 127.0.0.1 (startRangeProxy), so every byte it takes is counted and capped
// (MAX_POST_BYTES), whatever the host. It seeks before opening the input and
// takes the keyframe there (`-noaccurate_seek -ss <t> -i …`), so each frame
// costs the index plus one keyframe: a few MB. Stereo VR keeps one eye
// (stereoLayout), and frames are at most MAX_FRAME_WIDTH wide. Every file is
// written through a temporary one and renamed.

import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { Readable } from 'node:stream';
import { isVideoName } from './eroscripts-scout-lib.mjs';
import { RULE34VIDEO_HEADERS, RULE34VIDEO_HOST } from './stream-hosts.mjs';
import { SLR_HOST, SLR_UA } from './slr-host.mjs';
import { VRBANGERS_HOST } from './vrbangers-host.mjs';
import { findScene, nuxtDataOf, reviveNuxt } from './vrbangers-lib.mjs';

/** Where the frames go, under userData: `scout-frames/eroscripts/<topicId>/` (the SLR scout writes `scout-frames/slr/<sceneId>/`). */
export const FRAMES_DIR = 'scout-frames';
/** Points of the video's length (%) a frame is taken at; each is saved as `<point>.jpg`. */
export const FRAME_POINTS = [25, 50, 75];
/** Widest frame saved (px); narrower ones keep their size. */
export const MAX_FRAME_WIDTH = 1280;
/** Longest one ffmpeg (or ffprobe) run may take. */
export const FRAME_TIMEOUT_MS = 60_000;
/** Most bytes one post's frames may read from its host (index, three keyframes, a probe); the read stops there. */
export const MAX_POST_BYTES = 96 * 1024 * 1024;
/** A failed post is tried again after this long. */
export const FAILED_RETRY_MS = 7 * 24 * 3600 * 1000;
/** Posts whose card shows fewer usable images than this get frames. */
export const FRAMES_WANTED = 3;
/** An upload this many judged posts show is branding, not a preview (crates/scout-report deck.rs SHARED_IMAGE_POSTS). */
export const SHARED_IMAGE_POSTS = 3;
/** `--frames` without a number: posts per run. */
export const DEFAULT_FRAME_POSTS = 100;
/** Posts `--daily` makes frames for, after its scan. */
export const DAILY_FRAME_POSTS = 50;
/** Lowest stream quality taken for frames: flat videos, and stereo VR (each eye gets half the width or height). */
export const MIN_FLAT_HEIGHT = 480;
export const MIN_STEREO_HEIGHT = 720;
const UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36';
const VRBANGERS_CONTENT = 'https://content.vrbangers.com';

/** A post's frames failed for `reason` (recorded as `framesFailed`); `stopHost`: the host's allowance is used up, skip it for the rest of the run. */
export class FrameError extends Error {
  constructor(reason, { stopHost = false } = {}) {
    super(reason);
    this.reason = reason;
    this.stopHost = stopHost;
  }
}

const SEP = String.raw`[_\-. /\\\[\]()]`;
const token = (words) => new RegExp(`(?:^|${SEP})(?:${words})(?=${SEP}|$)`, 'i');
const MONO = token('mono|mono180|mono360');
const TOP_BOTTOM = token(String.raw`3dv|overunder|over[-_ ]?under|top[-_ ]?bottom|(?:180|360)_tb`);
const SIDE_BY_SIDE = token(String.raw`sbs|sbsf|3dh|side[-_ ]?by[-_ ]?side|mkx\d*|fisheye\d*|vrca\d+|rf52|180x180|(?:180|360)_lr`);
// Short tokens that are also words or initials in titles: only VR videos' names count them.
const TOP_BOTTOM_VR = token('tb|tbf|ou|ouf');
const SIDE_BY_SIDE_VR = token('lr|lrf');

/**
 * How a video packs its picture, from its name (HereSphere / DeoVR tokens): `'sbs'` (two eyes side by
 * side: `_LR`, `3dh`, `sbs`, `MKX…`, `FISHEYE…`, `180x180`), `'tb'` (top and bottom: `_TB`, `3dv`,
 * `OverUnder`), `'mono'` (one picture: `mono`, `MONO180`, `MONO360`), `'flat'` for a non-VR video
 * without a token, null for a VR video without one (the frame size decides: layoutFromSize). The short
 * tokens (`LR`, `TB`, `OU`) count only for a VR video.
 */
export function stereoLayout(name, { vr = false } = {}) {
  const stem = String(name || '').replace(/\.[a-z0-9]{2,4}$/i, '');
  if (MONO.test(stem)) return 'mono';
  if (TOP_BOTTOM.test(stem) || (vr && TOP_BOTTOM_VR.test(stem))) return 'tb';
  if (SIDE_BY_SIDE.test(stem) || (vr && SIDE_BY_SIDE_VR.test(stem))) return 'sbs';
  return vr ? null : 'flat';
}

/** A VR video's layout from its frame size when the name doesn't say: 2:1 or wider side by side, 3:4 or taller top and bottom, else one picture. */
export function layoutFromSize(width, height) {
  const ratio = Number(width) / Number(height);
  if (!Number.isFinite(ratio) || ratio <= 0) return 'mono';
  if (ratio >= 1.8) return 'sbs';
  if (ratio <= 0.75) return 'tb';
  return 'mono';
}

/** The ffmpeg filter for a layout: the left half (side by side) or top half (top and bottom), at most MAX_FRAME_WIDTH wide. */
export function frameFilter(layout) {
  const crop = layout === 'sbs' ? 'crop=iw/2:ih:0:0,' : layout === 'tb' ? 'crop=iw:ih/2:0:0,' : '';
  return `${crop}scale='min(${MAX_FRAME_WIDTH},iw)':-2`;
}

/** Seconds into a video of `lengthMs` for each FRAME_POINTS frame (ms precision); [] without a length. */
export function frameTimes(lengthMs) {
  const ms = Number(lengthMs);
  if (!Number.isFinite(ms) || ms <= 0) return [];
  return FRAME_POINTS.map((p) => Math.round(ms * p / 100) / 1000);
}

/** `-headers` value: one `Name: value` line per header, CRLF-terminated. */
const headerLines = (headers) => Object.entries(headers).map(([k, v]) => `${k}: ${v}\r\n`).join('');

/**
 * ffmpeg's arguments for one frame: input seeking (the keyframe at or before `seekSec`, so only that
 * part of the file is read), one video frame, the layout's crop and scale, a JPEG written to `out`.
 * `http`: the input is an HTTP URL that answers range requests (`-seekable 1`); `headers` go with it.
 */
export function frameArgs({ input, seekSec, out, layout, http = false, headers = null }) {
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
    ...(http ? ['-seekable', '1'] : []),
    ...(headers && Object.keys(headers).length ? ['-headers', headerLines(headers)] : []),
    '-noaccurate_seek', '-ss', String(seekSec), '-i', input,
    '-map', '0:v:0', '-frames:v', '1', '-an', '-sn', '-dn',
    '-vf', frameFilter(layout), '-q:v', '3', '-c:v', 'mjpeg', '-f', 'mjpeg', out,
  ];
}

/** ffmpeg's arguments to save a still image (VRBangers' gallery) as a JPEG at most MAX_FRAME_WIDTH wide. */
export function imageArgs({ input, out, headers = null }) {
  return [
    '-hide_banner', '-loglevel', 'error', '-nostdin', '-y',
    ...(headers && Object.keys(headers).length ? ['-headers', headerLines(headers)] : []),
    '-i', input, '-frames:v', '1', '-vf', frameFilter('flat'), '-q:v', '3', '-c:v', 'mjpeg', '-f', 'mjpeg', out,
  ];
}

/** ffprobe's arguments for a video's length and first video stream's size (JSON). */
export function probeArgs({ input, http = false }) {
  return [
    '-v', 'error', ...(http ? ['-seekable', '1'] : []),
    '-select_streams', 'v:0', '-show_entries', 'format=duration:stream=width,height', '-of', 'json', input,
  ];
}

/** ffprobe's JSON (probeArgs) as `{lengthMs, width, height}`, each null when missing. */
export function parseProbe(stdout) {
  let j = null;
  try { j = JSON.parse(String(stdout || '')); } catch { /* not JSON */ }
  const sec = Number(j?.format?.duration);
  const s = j?.streams?.[0] || {};
  return {
    lengthMs: Number.isFinite(sec) && sec > 0 ? Math.round(sec * 1000) : null,
    width: Number(s.width) || null,
    height: Number(s.height) || null,
  };
}

// ── Which posts ─────────────────────────────────────────────────────────

/** An upload's identity: the SHA-1 Discourse names it by (originals and optimized copies share it), else the whole URL (deck.rs image_key). */
export function imageKey(url) {
  const name = String(url).split('/').pop();
  const hex = /^[0-9a-f]*/i.exec(name)[0].length;
  return hex === 40 ? name.slice(0, 40) : String(url);
}

/** A state record's image URLs: `images`, else its single `image` (records from before images were kept). */
export function recordImages(rec) {
  if (Array.isArray(rec?.images)) return rec.images.filter((u) => typeof u === 'string');
  return typeof rec?.image === 'string' ? [rec.image] : [];
}

/** How many judged posts show each upload (imageKey). */
export function imageCounts(state) {
  const counts = new Map();
  for (const rec of Object.values(state?.topics || {})) {
    for (const key of new Set(recordImages(rec).map(imageKey))) counts.set(key, (counts.get(key) || 0) + 1);
  }
  return counts;
}

/** The images a card shows of a record (deck.rs images): https only, one per upload, none SHARED_IMAGE_POSTS posts share. */
export function usableImages(rec, counts) {
  const out = [];
  for (const url of recordImages(rec)) {
    const key = imageKey(url);
    if (!url.startsWith('https://') || (counts.get(key) || 0) >= SHARED_IMAGE_POSTS || out.some((u) => imageKey(u) === key)) continue;
    out.push(url);
  }
  return out;
}

/** True when a failed try (`framesFailed`) is recent enough not to try again. */
export function recentlyFailed(rec, now = Date.now()) {
  const at = Date.parse(rec?.framesFailed?.at);
  return Number.isFinite(at) && now - at < FAILED_RETRY_MS;
}

/**
 * The waiting posts that need frames, in the order `--frames` takes them: on the list, a state record,
 * not in the library (`inLibrary(c)`), not swiped left (`hidden`), fewer than FRAMES_WANTED usable
 * images, no `frames` yet and no failed try in the last FAILED_RETRY_MS. Swiped right first, then posts
 * without any image, then by score (best first), then newest.
 *
 * @param {object[]} candidates the saved list's candidates
 * @param {{state: object, wanted?: Set<number>, hidden?: Set<number>, inLibrary?: (c: object) => boolean, now?: number}} o
 * @returns {{c: object, images: number}[]}
 */
export function framesQueue(candidates, { state, wanted = new Set(), hidden = new Set(), inLibrary = () => false, now = Date.now() }) {
  const counts = imageCounts(state);
  const out = [];
  for (const c of candidates) {
    const rec = state?.topics?.[c.id];
    if (!rec || hidden.has(c.id) || rec.frames || recentlyFailed(rec, now) || inLibrary(c)) continue;
    const images = usableImages(rec, counts).length;
    if (images < FRAMES_WANTED) out.push({ c, images });
  }
  const score = (x) => (Number.isFinite(x.c.score) ? x.c.score : -Infinity);
  return out.sort((a, b) => (wanted.has(b.c.id) - wanted.has(a.c.id))
    || ((a.images === 0 ? 0 : 1) - (b.images === 0 ? 0 : 1))
    || (score(b) - score(a))
    || (b.c.id - a.c.id));
}

// ── Sources ─────────────────────────────────────────────────────────────

/** The scene frames come from: the post's first one with a video (an archive's scenes have none to read). */
export function frameScene(c) {
  return (c?.scenes || []).find((sc) => !sc.packed && sc.video && (isVideoName(sc.video.name) || [RULE34VIDEO_HOST, SLR_HOST, VRBANGERS_HOST].includes(sc.video.host))) || null;
}

/**
 * Where a post's frames come from, free reads only (FrameSource kinds):
 * - `http`: a forum attachment (`forum: true`, the session goes to the forum host only) or a pixeldrain file (`/api/file/<id>`);
 * - `rule34video`: the page is read again for a fresh player link;
 * - `mega`: the file, read in ranges;
 * - `slr-trailer`: the scene's public trailer (never the account's download: it uses up the weekly quota);
 * - `vrbangers-gallery`: the public scene page's stills (its videos need the paid account).
 * `{skip: <host>}` for anything else. `layout` is stereoLayout of the video name (null: VR, size decides);
 * `lengthMs` the full video's length when known (an SLR trailer is shorter: probed instead).
 */
export function frameSource(c) {
  const sc = frameScene(c);
  if (!sc) return { skip: (c?.scenes || []).some((s) => s.packed) ? 'archive' : 'no video' };
  const v = sc.video;
  const base = {
    host: v.host, name: v.name, layout: stereoLayout(v.name, { vr: c.vr === true || v.vr === true }),
    lengthMs: (Number(v.duration) > 0 ? Math.round(v.duration * 1000) : null) || (Number(sc.durationMs) > 0 ? sc.durationMs : null),
  };
  switch (v.host) {
    case 'forum': return { ...base, kind: 'http', url: v.url, forum: true };
    case 'pixeldrain': {
      const id = /\/api\/file\/([A-Za-z0-9]+)/.exec(v.url || '')?.[1];
      return id ? { ...base, kind: 'http', url: `https://pixeldrain.com/api/file/${id}` } : { skip: 'pixeldrain' };
    }
    case RULE34VIDEO_HOST: return { ...base, kind: 'rule34video', url: v.url };
    case 'mega': return { ...base, kind: 'mega', url: v.url, remote: v.remote || v.name };
    case SLR_HOST: return Number(v.slrSceneId) > 0 ? { ...base, kind: 'slr-trailer', sceneId: Number(v.slrSceneId), lengthMs: null } : { skip: SLR_HOST };
    case VRBANGERS_HOST: return v.slug || v.url ? { ...base, kind: 'vrbangers-gallery', url: v.url || `https://vrbangers.com/video/${v.slug}/`, slug: v.slug || null } : { skip: VRBANGERS_HOST };
    default: return { skip: v.host || 'unknown' };
  }
}

/** The quality to read: the lowest at least `minHeight` high, else the highest there is; null when none. */
export function pickVariant(variants, { minHeight = MIN_FLAT_HEIGHT } = {}) {
  const list = (variants || []).filter((v) => Number(v?.height) > 0).sort((a, b) => a.height - b.height);
  return list.find((v) => v.height >= minHeight) || list[list.length - 1] || null;
}

/** Lowest quality worth reading for a layout: stereo VR keeps half the picture, so it takes MIN_STEREO_HEIGHT. */
export const minHeightFor = (layout) => (layout === 'flat' || layout === 'mono' ? MIN_FLAT_HEIGHT : MIN_STEREO_HEIGHT);

/**
 * An SLR scene's (`/v3/scenes/<id>` data) free trailer to read: an h264 trailer encoding (else any
 * encoding) of pickVariant's height, else the short `videoPreview`. `{url, height, kind}`; null when
 * the scene offers neither. The full video is never taken (its download uses up the account's quota).
 */
export function slrTrailer(scene, { minHeight = MIN_STEREO_HEIGHT } = {}) {
  const encodings = Array.isArray(scene?.trailerEncodings) ? scene.trailerEncodings : [];
  const sources = (e) => (Array.isArray(e?.videoSources) ? e.videoSources : [])
    .filter((s) => typeof s?.url === 'string' && /^https:\/\//.test(s.url))
    .map((s) => ({ url: s.url, height: Number(s.height || s.resolution) || 0 }));
  for (const list of [encodings.filter((e) => /h\.?264|avc/i.test(e?.name || '')), encodings]) {
    const pick = pickVariant(list.flatMap(sources), { minHeight });
    if (pick) return { ...pick, kind: 'trailer' };
  }
  const preview = scene?.videoPreview;
  return typeof preview === 'string' && /^https:\/\//.test(preview) ? { url: preview, height: Number(/_(\d+)p\./.exec(preview)?.[1]) || null, kind: 'preview' } : null;
}

/**
 * Up to three stills of a VRBangers scene (`findScene` of its public page): gallery images spread over
 * the gallery (at 25, 50 and 75 % of it), else the poster and slider image; each the largest preview at
 * most MAX_FRAME_WIDTH wide, else the original. Absolute URLs.
 */
export function vrbangersGallery(scene) {
  const abs = (p) => (typeof p !== 'string' || !p ? null : /^https?:\/\//.test(p) ? p : `${VRBANGERS_CONTENT}${p.startsWith('/') ? '' : '/'}${p}`);
  const best = (img) => {
    const fits = (Array.isArray(img?.previews) ? img.previews : [])
      .filter((p) => Number(p?.width) > 0 && Number(p.width) <= MAX_FRAME_WIDTH && p.permalink)
      .sort((a, b) => b.width - a.width);
    return abs(fits[0]?.permalink || img?.permalink);
  };
  const gallery = (Array.isArray(scene?.galleryImages) ? scene.galleryImages : []).map(best).filter(Boolean);
  let picks = gallery.length > FRAME_POINTS.length
    ? [...new Set(FRAME_POINTS.map((p) => gallery[Math.floor(gallery.length * p / 100)]))]
    : gallery;
  if (!picks.length) picks = [scene?.poster, scene?.sliderImage].map(best).filter(Boolean);
  return [...new Set(picks)].slice(0, FRAME_POINTS.length);
}

/** VRBangers' public scene page (HTML) → its stills (vrbangersGallery); [] when the page has no scene data. */
export function vrbangersPageImages(html, slug) {
  const flat = nuxtDataOf(html);
  if (!flat) return [];
  const scene = findScene(reviveNuxt(flat), slug);
  return scene ? vrbangersGallery(scene) : [];
}

// ── State ───────────────────────────────────────────────────────────────

/** Frames made for a post: `rec.frames = {at, source: '<host>: <video>', files}`; an earlier failure is forgotten. */
export function recordFrames(rec, { source, files, now = Date.now() }) {
  rec.frames = { at: new Date(now).toISOString(), source, files: [...files] };
  delete rec.framesFailed;
}

/** A failed try: `rec.framesFailed = {at, reason}`; not tried again for FAILED_RETRY_MS. */
export function recordFramesFailed(rec, reason, now = Date.now()) {
  rec.framesFailed = { at: new Date(now).toISOString(), reason: String(reason) };
}

/** A post's frames folder: `<userData>/scout-frames/eroscripts/<topicId>`. */
export const postFramesDir = (userData, topicId) => path.join(userData, FRAMES_DIR, 'eroscripts', String(Number(topicId)));

// ── Running ffmpeg ──────────────────────────────────────────────────────

/**
 * The default runner: `cmd args` without a shell, killed after `timeoutMs`. Async on purpose: the range
 * proxy ffmpeg reads runs in this process. Resolves `{code, stdout, stderr, timedOut}`.
 */
export function spawnRunner(cmd, args, { timeoutMs = FRAME_TIMEOUT_MS } = {}) {
  return new Promise((resolve) => {
    let child;
    try { child = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'] }); } catch (err) {
      resolve({ code: -1, stdout: '', stderr: err.message, timedOut: false });
      return;
    }
    let stdout = '';
    let stderr = '';
    let timedOut = false;
    child.stdout.on('data', (d) => { stdout += d; });
    child.stderr.on('data', (d) => { stderr = (stderr + d).slice(-4000); });
    const timer = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); }, timeoutMs);
    child.on('error', (err) => { clearTimeout(timer); resolve({ code: -1, stdout, stderr: `${cmd}: ${err.message}`, timedOut }); });
    child.on('close', (code) => { clearTimeout(timer); resolve({ code: code ?? -1, stdout, stderr, timedOut }); });
  });
}

const lastLine = (s) => String(s || '').trim().split('\n').pop() || '';

/** Remove the frames a run makes (and leftovers of a stopped one) from `dir`, so an earlier try's files can't be mistaken for this one's. */
function clearFrames(dir) {
  let names = [];
  try { names = fs.readdirSync(dir); } catch { return; }
  for (const n of names) if (/^\d+\.jpg(?:\.\d+\.tmp)?$/.test(n)) fs.rmSync(path.join(dir, n), { force: true });
}

/** A JPEG's `{width, height}` from its frame header (SOF0–SOF3); null when there is none. */
export function jpegSize(buf) {
  let i = 2;
  while (i + 9 < buf.length) {
    if (buf[i] !== 0xff) { i++; continue; }
    const marker = buf[i + 1];
    if (marker === 0xff) { i++; continue; }
    if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) { i += 2; continue; }
    if (marker >= 0xc0 && marker <= 0xc3) return { height: buf.readUInt16BE(i + 5), width: buf.readUInt16BE(i + 7) };
    i += 2 + buf.readUInt16BE(i + 2);
  }
  return null;
}

/** A JPEG this small for its size is one colour (a black fade, an empty title gap): nothing to see. A black 1280×720 frame is ~0.006 bytes a pixel, real frames 0.03 and up. */
export const BLANK_BYTES_PER_PIXEL = 0.012;
/** Where to look instead of a blank frame: this many % of the length later, then earlier. */
export const BLANK_RETRY_SHIFTS = [5, -5];

/** True when a JPEG is blank (BLANK_BYTES_PER_PIXEL). */
export function isBlankFrame(buf) {
  const s = jpegSize(buf);
  return !!s && s.width > 0 && s.height > 0 && buf.length < s.width * s.height * BLANK_BYTES_PER_PIXEL;
}

/** Run one ffmpeg that writes `out` through a temporary file; ok when a non-empty JPEG landed. A blank one (`skipBlank`) isn't kept. */
async function writeOne(run, args, tmp, out, timeoutMs, { skipBlank = false } = {}) {
  const r = await run('ffmpeg', args, { timeoutMs });
  let buf = null;
  try { if (r.code === 0) buf = fs.readFileSync(tmp); } catch { /* no output */ }
  if (!buf?.length) {
    fs.rmSync(tmp, { force: true });
    return { ok: false, why: r.timedOut ? 'timed out' : lastLine(r.stderr) || `ffmpeg exit ${r.code}` };
  }
  if (skipBlank && isBlankFrame(buf)) {
    fs.rmSync(tmp, { force: true });
    return { ok: false, why: 'blank frame' };
  }
  fs.renameSync(tmp, out);
  return { ok: true };
}

/**
 * The FRAME_POINTS frames of a video into `dir` (`25.jpg`, …), each by its own ffmpeg (frameArgs) with
 * `timeoutMs`. `lengthMs` sets the times; a blank frame is taken again BLANK_RETRY_SHIFTS % later, then
 * earlier. Returns `{files, errors}`: the files made, in order, and why the others weren't.
 */
export async function grabFrames({ input, http = false, headers = null, lengthMs, layout, dir, run = spawnRunner, timeoutMs = FRAME_TIMEOUT_MS }) {
  fs.mkdirSync(dir, { recursive: true });
  clearFrames(dir);
  const files = [];
  const errors = [];
  if (!frameTimes(lengthMs).length) return { files, errors: ['video length unknown'] };
  for (const point of FRAME_POINTS) {
    const name = `${point}.jpg`;
    const out = path.join(dir, name);
    const tmp = `${out}.${process.pid}.tmp`;
    let r;
    for (const shift of [0, ...BLANK_RETRY_SHIFTS]) {
      const seekSec = Math.round(lengthMs * (point + shift) / 100) / 1000;
      r = await writeOne(run, frameArgs({ input, seekSec, out: tmp, layout, http, headers }), tmp, out, timeoutMs, { skipBlank: true });
      if (r.ok || r.why !== 'blank frame') break;
    }
    if (r.ok) files.push(name); else errors.push(r.why);
  }
  return { files, errors };
}

/** Still images (`urls`, at most three) saved as the frames of `dir` (imageArgs). Same result as grabFrames. */
export async function grabImages({ urls, headers = null, dir, run = spawnRunner, timeoutMs = FRAME_TIMEOUT_MS }) {
  fs.mkdirSync(dir, { recursive: true });
  clearFrames(dir);
  const files = [];
  const errors = [];
  for (const [i, url] of urls.slice(0, FRAME_POINTS.length).entries()) {
    const name = `${FRAME_POINTS[i]}.jpg`;
    const out = path.join(dir, name);
    const tmp = `${out}.${process.pid}.tmp`;
    const r = await writeOne(run, imageArgs({ input: url, out: tmp, headers }), tmp, out, timeoutMs);
    if (r.ok) files.push(name); else errors.push(r.why);
  }
  return { files, errors };
}

// ── Range proxy and readers ─────────────────────────────────────────────

/**
 * A local HTTP server ffmpeg reads a remote file through: `GET`/`HEAD` with `Range: bytes=a-[b]` answered
 * from `open(start, end)` (a Readable, or a promise of one, of those bytes; destroyed when ffmpeg hangs up).
 * Counts what it serves; past `maxBytes` it stops serving (`capped`). The first reader error is kept
 * (`error()`): a MEGA bandwidth limit, an HTTP 403. Resolves `{url, bytes(), capped(), error(), close()}`.
 */
export async function startRangeProxy({ size, open, maxBytes = MAX_POST_BYTES, contentType = 'video/mp4' }) {
  let bytes = 0;
  let capped = false;
  let error = null;
  const live = new Set();
  const server = http.createServer((req, res) => {
    if (req.method !== 'GET' && req.method !== 'HEAD') { res.writeHead(405).end(); return; }
    const m = /^bytes=(\d+)-(\d*)$/.exec(String(req.headers.range || '').trim());
    const start = m ? Number(m[1]) : 0;
    const end = m && m[2] ? Math.min(Number(m[2]), size - 1) : size - 1;
    if (start >= size || start > end) { res.writeHead(416, { 'Content-Range': `bytes */${size}` }).end(); return; }
    const head = { 'Accept-Ranges': 'bytes', 'Content-Type': contentType, 'Content-Length': String(end - start + 1), ...(m ? { 'Content-Range': `bytes ${start}-${end}/${size}` } : {}) };
    if (req.method === 'HEAD') { res.writeHead(m ? 206 : 200, head).end(); return; }
    if (capped) { res.writeHead(503).end(); return; }
    Promise.resolve().then(() => open(start, end)).then((stream) => {
      if (res.destroyed || res.writableEnded) { stream.destroy?.(); return; }
      live.add(stream);
      // ffmpeg hung up (it seeks elsewhere, or it's done): stop reading; errors from then on are the abort's.
      let stopped = false;
      const stop = () => { stopped = true; live.delete(stream); stream.destroy?.(); };
      res.on('close', stop);
      stream.on('error', (err) => { if (!stopped) error ||= err; res.destroy(); });
      stream.on('data', (chunk) => {
        bytes += chunk.length;
        if (bytes > maxBytes) { capped = true; res.destroy(); stop(); }
      });
      res.writeHead(m ? 206 : 200, head);
      stream.pipe(res);
    }, (err) => {
      error ||= err;
      if (!res.headersSent) res.writeHead(502).end(); else res.destroy();
    });
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  return {
    url: `http://127.0.0.1:${server.address().port}/video`,
    bytes: () => bytes,
    capped: () => capped,
    error: () => error,
    close: () => new Promise((resolve) => {
      for (const s of live) s.destroy?.();
      server.closeAllConnections?.();
      server.close(() => resolve());
    }),
  };
}

const httpStatusError = (host, status) => new FrameError(status === 403 && host === 'pixeldrain'
  ? 'pixeldrain: HTTP 403 (hotlink protection; download it by hand)'
  : `${host}: HTTP ${status}`);

/**
 * A file over HTTP read in ranges: the first request (`Range: bytes=0-0`, redirects followed by hand;
 * `sensitive` headers, e.g. the forum session, only to `url`'s own host) gives its size and final URL.
 * A host that ignores ranges is refused (frames would read the whole file). `{size, open(start, end)}`.
 */
export async function httpSource({ url, host, headers = {}, sensitive = {}, fetch = globalThis.fetch, timeoutMs = 30_000 }) {
  const origin = new URL(url).host;
  let u = url;
  let resp = null;
  for (let hop = 0; hop < 6; hop++) {
    const h = { ...headers, ...(new URL(u).host === origin ? sensitive : {}), Range: 'bytes=0-0' };
    resp = await fetch(u, { headers: h, redirect: 'manual', signal: AbortSignal.timeout(timeoutMs) });
    if (resp.status < 300 || resp.status >= 400) break;
    const next = resp.headers.get('location');
    await resp.body?.cancel?.().catch(() => {});
    if (!next) throw new FrameError(`${host}: redirect without a location`);
    u = new URL(next, u).href;
  }
  await resp.body?.cancel?.().catch(() => {});
  if (resp.status !== 206) {
    if (resp.status === 200) throw new FrameError(`${host}: no range requests`);
    throw httpStatusError(host, resp.status);
  }
  const size = Number(/\/(\d+)\s*$/.exec(resp.headers.get('content-range') || '')?.[1]);
  if (!(size > 0)) throw new FrameError(`${host}: size unknown`);
  const final = u;
  const keep = new URL(final).host === origin ? sensitive : {};
  return {
    size,
    async open(start, end) {
      const ac = new AbortController();
      const r = await fetch(final, { headers: { ...headers, ...keep, Range: `bytes=${start}-${end}` }, signal: ac.signal });
      if (!r.body || (r.status !== 206 && !(r.status === 200 && start === 0))) {
        await r.body?.cancel?.().catch(() => {});
        throw httpStatusError(host, r.status);
      }
      const s = Readable.fromWeb(r.body);
      s.on('close', () => ac.abort());
      return s;
    },
  };
}

/** megajs download options for a range: small chunks, two in flight, so little is read past what ffmpeg takes. (Its one-connection mode leaves an unhandled rejection when stopped.) */
export const MEGA_RANGE_OPTIONS = Object.freeze({ maxConnections: 2, initialChunkSize: 256 * 1024, chunkSizeIncrement: 256 * 1024, maxChunkSize: 1024 * 1024 });

/**
 * A MEGA file (`url`, a file link or a folder link and the file's `remote` name) read in ranges:
 * each range is one megajs download (`{start, end}` with MEGA_RANGE_OPTIONS, stopped when ffmpeg hangs up).
 * `File` is megajs's File (injectable). `{size, open}`.
 */
export async function megaSource({ url, remote, File }) {
  const root = File.fromURL(url);
  await root.loadAttributes();
  const find = (n) => {
    for (const c of n.children || []) {
      if (c.directory) { const hit = find(c); if (hit) return hit; } else if (c.name === remote) return c;
    }
    return null;
  };
  const node = root.directory ? find(root) : root;
  if (!node) throw new FrameError(`mega: ${remote} is gone`);
  return { size: node.size, open: (start, end) => node.download({ ...MEGA_RANGE_OPTIONS, start, end }) };
}

/** A reader's error as a failure reason: MEGA's bandwidth limit stops MEGA for the run. */
export function readerFailure(host, err) {
  if (err instanceof FrameError) return err;
  const msg = String(err?.message || err);
  if (/bandwidth limit/i.test(msg)) return new FrameError(`mega: bandwidth limit (${msg.replace(/^.*?:\s*/, '')})`, { stopHost: true });
  return new FrameError(`${host}: ${msg}`);
}

/**
 * Frames for one post (frameSource), into `dir`. `deps`: `fetch`, `File` (megajs), `rule34video`
 * (stream-hosts createRule34video: `page(url)`), `slr` (slr-host createSlr: `scene(id)`), `forumHeaders`
 * (the session, forum host only), `run` (spawnRunner), `timeoutMs`, `maxBytes`.
 * Resolves `{files, source, bytes}` (files may be fewer than three), `{skip: host}` when the post has no
 * free source; throws FrameError when none came out.
 */
export async function postFrames(c, dir, deps) {
  const { fetch = globalThis.fetch, run = spawnRunner, timeoutMs = FRAME_TIMEOUT_MS, maxBytes = MAX_POST_BYTES } = deps;
  const plan = frameSource(c);
  if (plan.skip) return { skip: plan.skip };
  let source = `${plan.host}: ${plan.name}`;
  if (plan.kind === 'vrbangers-gallery') {
    const resp = await fetch(plan.url, { headers: { 'User-Agent': UA, Accept: 'text/html' }, signal: AbortSignal.timeout(timeoutMs) });
    if (!resp.ok) { await resp.body?.cancel?.().catch(() => {}); throw new FrameError(`vrbangers: HTTP ${resp.status} for the scene page`); }
    const slug = plan.slug || /\/video\/([^/?#]+)/.exec(plan.url)?.[1] || '';
    const urls = vrbangersPageImages(await resp.text(), slug);
    if (!urls.length) throw new FrameError('vrbangers: no gallery images on the scene page');
    const r = await grabImages({ urls, headers: { 'User-Agent': UA }, dir, run, timeoutMs });
    if (!r.files.length) throw new FrameError(`vrbangers: ${r.errors[0] || 'no image saved'}`);
    return { files: r.files, source: `${source} (scene page stills)`, bytes: null };
  }
  let reader;
  let lengthMs = plan.lengthMs;
  try {
    if (plan.kind === 'http') {
      reader = await httpSource({ url: plan.url, host: plan.host, headers: { 'User-Agent': UA }, sensitive: plan.forum ? deps.forumHeaders || {} : {}, fetch });
    } else if (plan.kind === 'rule34video') {
      const page = await deps.rule34video.page(plan.url);
      const v = pickVariant(page.variants, { minHeight: minHeightFor(plan.layout) });
      if (!v) throw new FrameError('rule34video: no playable quality on the page');
      if (page.durationSec) lengthMs = Math.round(page.durationSec * 1000);
      source += ` (${v.height}p)`;
      reader = await httpSource({ url: v.direct, host: plan.host, headers: { ...RULE34VIDEO_HEADERS }, fetch });
    } else if (plan.kind === 'mega') {
      reader = await megaSource({ url: plan.url, remote: plan.remote, File: deps.File });
    } else if (plan.kind === 'slr-trailer') {
      const scene = await deps.slr.scene(plan.sceneId);
      if (!scene) throw new FrameError('slr: scene not found');
      const t = slrTrailer(scene, { minHeight: minHeightFor(plan.layout) });
      if (!t) throw new FrameError('slr: no free trailer');
      source += ` (${t.kind}${t.height ? ` ${t.height}p` : ''})`;
      reader = await httpSource({ url: t.url, host: plan.host, headers: { 'User-Agent': SLR_UA }, fetch });
    }
  } catch (err) {
    throw readerFailure(plan.host, err);
  }
  const proxy = await startRangeProxy({ size: reader.size, open: reader.open, maxBytes });
  try {
    let layout = plan.layout;
    if (!lengthMs || !layout) {
      const p = parseProbe((await run('ffprobe', probeArgs({ input: proxy.url, http: true }), { timeoutMs })).stdout);
      lengthMs ||= p.lengthMs;
      layout ||= p.width && p.height ? layoutFromSize(p.width, p.height) : 'mono';
    }
    if (!lengthMs) throw proxy.error() ? readerFailure(plan.host, proxy.error()) : new FrameError(`${plan.host}: video length unknown`);
    const r = await grabFrames({ input: proxy.url, http: true, lengthMs, layout, dir, run, timeoutMs });
    if (!r.files.length) {
      if (proxy.error()) throw readerFailure(plan.host, proxy.error());
      if (proxy.capped()) throw new FrameError(`${plan.host}: read cap (${Math.round(maxBytes / 1048576)} MB) reached`);
      throw new FrameError(`${plan.host}: ${r.errors[0] || 'no frame'}`);
    }
    // Some frames came out before the host stopped serving (MEGA's bandwidth limit): keep them, skip the host from here.
    const err = proxy.error();
    return { files: r.files, source, bytes: proxy.bytes(), stopHost: !!err && readerFailure(plan.host, err).stopHost };
  } finally {
    await proxy.close();
  }
}
