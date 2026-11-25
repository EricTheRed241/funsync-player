// Pornhub videos a post links (`pornhub.com/view_video.php?viewkey=…`, any
// subdomain or locale), read and downloaded with yt-dlp (on PATH; brew install
// yt-dlp). Used by npm run eroscripts:scout (scripts/eroscripts-scout.mjs):
// the scan reads each link's metadata (`yt-dlp -J --no-playlist`) and turns it
// into one pseudo video file in the best quality within the caps
// (FLAT_MAX_HEIGHT / VR_MAX_HEIGHT, shared/content-preferences.js), which
// the post's scripts pair with by length like a rule34video page; --fetch reads
// the metadata again, checks the length, and has yt-dlp download and merge
// that quality into an mp4.
//
// The pure parts (links, format string, metadata, files) are exported for the
// tests; createPornhub takes the process runner as a fake there.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { downloadFits, streamTitle } from './stream-hosts.mjs';

export const PORNHUB_HOST = 'pornhub';
export const YT_DLP = 'yt-dlp';

// ── Links ───────────────────────────────────────────────────────────────

const PORNHUB_PAGE = /^https?:\/\/(?:[a-z0-9-]+\.)*pornhub\.(?:com|org|net)\/view_video\.php\?(?:[^#]*&)?viewkey=([A-Za-z0-9_-]+)/i;

/** The viewkey of a Pornhub video link, else null. */
export function pornhubViewkey(href) {
  return (PORNHUB_PAGE.exec(String(href || '')) || [])[1] || null;
}

/**
 * The Pornhub video pages among a post's links, as
 * `https://www.pornhub.com/view_video.php?viewkey=<key>` (locale subdomain,
 * other query parameters and fragment dropped), one per viewkey, in link order.
 * Profile, model and search pages aren't videos.
 */
export function pornhubLinks(hrefs) {
  const keys = new Set();
  for (const href of hrefs || []) {
    const key = pornhubViewkey(href);
    if (key) keys.add(key);
  }
  return [...keys].map((k) => `https://www.pornhub.com/view_video.php?viewkey=${k}`);
}

// ── yt-dlp ──────────────────────────────────────────────────────────────

/** yt-dlp's format selection: the best video ≤ `cap` px high with the best audio, else the best single file ≤ `cap`. */
export function pornhubFormat(cap) {
  const h = Math.floor(Number(cap));
  if (!(h > 0)) throw new Error(`pornhub: bad height cap ${cap}`);
  return `bv*[height<=${h}]+ba/b[height<=${h}]`;
}

/** Arguments for the metadata read. */
export const infoArgs = (url) => ['-J', '--no-playlist', '--no-warnings', url];

/** Arguments for the download of `url` in `cap` into `<dir>/video.mp4`. */
export function downloadArgs(url, cap, dir) {
  return [
    '--no-playlist', '--no-warnings', '--no-progress', '--no-part',
    '-f', pornhubFormat(cap), '--merge-output-format', 'mp4',
    '-o', path.join(dir, 'video.%(ext)s'), url,
  ];
}

/** yt-dlp's errors for a video that is gone for good (disabled, removed, private): like a dead link. */
const GONE = /disabled|removed|not available|unavailable|private|does not exist|deleted|HTTP Error 404|HTTP Error 410|terms of service|copyright/i;

/** An Error for a failed yt-dlp run; `status` 404 when the video is gone, so the post isn't read again later. */
export function ytDlpError(stderr, url) {
  const line = String(stderr || '').split('\n').map((l) => l.trim()).filter((l) => /^ERROR:/.test(l)).pop()
    || String(stderr || '').trim().split('\n').pop() || 'yt-dlp failed';
  const err = new Error(`pornhub: ${line.replace(/^ERROR:\s*/, '')} (${url})`);
  if (GONE.test(line)) err.status = 404;
  return err;
}

/** yt-dlp isn't installed: the Pornhub host is skipped. */
export class YtDlpMissingError extends Error {
  constructor() {
    super('pornhub: yt-dlp not found on PATH (brew install yt-dlp); Pornhub links skipped');
    this.name = 'YtDlpMissingError';
  }
}

/**
 * What `yt-dlp -J` says of a video: title, length (s), and the height the
 * download gets within `cap` (the highest video format's, else the top-level
 * height when it fits); a live stream, an unplayable or formatless entry
 * throws as gone (`status` 404).
 *
 * @returns {{title: string, durationSec: number|null, height: number}}
 */
export function parsePornhubInfo(json, cap) {
  const info = typeof json === 'string' ? JSON.parse(json) : json;
  if (!info || typeof info !== 'object' || info._type === 'playlist' || info.is_live) {
    throw Object.assign(new Error('pornhub: not a single video'), { status: 404 });
  }
  const heights = (info.formats || [])
    .filter((f) => f && f.vcodec !== 'none' && Number(f.height) > 0 && Number(f.height) <= cap)
    .map((f) => Number(f.height));
  const top = Number(info.height) > 0 && Number(info.height) <= cap ? Number(info.height) : 0;
  const height = heights.length ? Math.max(...heights) : top;
  if (!height) throw Object.assign(new Error(`pornhub: no video format up to ${cap}p`), { status: 404 });
  const durationSec = Number(info.duration) > 0 ? Number(info.duration) : null;
  return { title: String(info.title || '').trim(), durationSec, height };
}

/**
 * The pseudo video file of a parsed Pornhub video (parsePornhubInfo):
 * `{name: '<title> <height>p.mp4', size: 0, host: 'pornhub', url, viewkey, title, height, duration}`.
 */
export function pornhubFile(url, info) {
  const viewkey = pornhubViewkey(url) || '';
  const base = streamTitle(info.title) || `pornhub ${viewkey}`.trim();
  return {
    name: `${base} ${info.height}p.mp4`, size: 0, host: PORNHUB_HOST, url, viewkey, title: info.title,
    height: info.height, duration: info.durationSec ?? null,
  };
}

/** True when the main script (`scriptMs`) fits the video (`durationSec`) for a download (downloadFits); unknown lengths pass. */
export function pornhubFits(durationSec, scriptMs) {
  return !(scriptMs > 0 && durationSec > 0) || downloadFits(durationSec, scriptMs);
}

/** Runs yt-dlp: `{code, stdout, stderr}`; YtDlpMissingError when it isn't installed. */
export function runYtDlp(args, { bin = YT_DLP } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(bin, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', (d) => { stdout += d; });
    child.stderr.on('data', (d) => { stderr += d; });
    child.on('error', (err) => reject(err.code === 'ENOENT' ? new YtDlpMissingError() : err));
    child.on('close', (code) => resolve({ code, stdout, stderr }));
  });
}

/**
 * Pornhub through yt-dlp (`run(args)` injectable for tests):
 * - `files(url, {cap})`: the video's pseudo file (pornhubFile) in a one-item list.
 * - `info(url, cap)`: the metadata read now (parsePornhubInfo).
 * - `download(file, tmp, {cap, scriptMs, videoSeconds})`: the metadata read
 *   again (gone → error; length not the script's → error), then yt-dlp
 *   downloads `pornhubFormat(cap)` merged into an mp4 in `<tmp>.d/`, moved
 *   to `tmp`; the downloaded length is checked again (`videoSeconds`, ffprobe).
 */
export function createPornhub({ run = runYtDlp } = {}) {
  async function info(url, cap) {
    const r = await run(infoArgs(url));
    if (r.code !== 0) throw ytDlpError(r.stderr, url);
    return parsePornhubInfo(r.stdout, cap);
  }
  return {
    info,
    async files(url, { cap }) {
      return [pornhubFile(url, await info(url, cap))];
    },
    async download(file, tmp, { cap, scriptMs = null, videoSeconds = null }) {
      const now = await info(file.url, cap);
      const want = scriptMs || (file.duration ? file.duration * 1000 : null);
      if (!pornhubFits(now.durationSec, want)) {
        throw new Error(`pornhub: video ${Math.round(now.durationSec)} s, script ${Math.round(want / 1000)} s; not this scene's video`);
      }
      const dir = `${tmp}.d`;
      fs.rmSync(dir, { recursive: true, force: true });
      fs.mkdirSync(dir, { recursive: true });
      try {
        const r = await run(downloadArgs(file.url, cap, dir));
        if (r.code !== 0) throw ytDlpError(r.stderr, file.url);
        const out = path.join(dir, 'video.mp4');
        if (!fs.existsSync(out)) throw new Error(`pornhub: yt-dlp left no mp4 (${file.url})`);
        fs.renameSync(out, tmp);
      } finally {
        fs.rmSync(dir, { recursive: true, force: true });
      }
      if (videoSeconds) {
        const got = videoSeconds(tmp);
        if (!got) throw new Error('pornhub: the download isn\'t a readable video (ffprobe)');
        if (want && !downloadFits(got, want)) throw new Error(`pornhub: downloaded video ${Math.round(got)} s, script ${Math.round(want / 1000)} s; not this scene's video`);
      }
    },
  };
}
