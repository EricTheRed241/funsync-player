#!/usr/bin/env node
// npm run eroscripts:preview-check — dry run of EroScripts preview matching
// on the real library, outside the app.
//
// Reads the app's config (custom thumbnails, script links, EroScripts
// session) and never writes it, nor anything else in the app's data folder.
// For every EroScripts poster set (and, with --pending, every linked video
// without a custom thumbnail) it reports:
//   - whether the current poster shows its video (what "Fetch EroScripts
//     previews" revalidation would keep or remove), with its distance;
//   - which image of the post matches the video, if any (what the batch
//     would set once the failing poster is gone).
// Frames are sampled by the backend's sampler (backend/services/ffmpeg.py)
// in a Python child process, images decoded by ffmpeg; the matching itself
// is the app's (electron/preview-match.js, electron/preview-validator.js).
//
//   npm run eroscripts:preview-check -- [--out <dir>] [--pending] [--limit N]
//       [--only <text>] [--no-candidates] [--user-data <dir>] [--python <bin>]

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import readline from 'node:readline';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

const USAGE = `Usage: npm run eroscripts:preview-check -- [options]
  --out <dir>          where report.md, report.json and contact sheets go
                       (default <tmp>/funsync-preview-check)
  --pending            also check linked videos that have no custom thumbnail
  --no-candidates      only check the current posters, fetch nothing from the forum
  --limit N            check at most N videos
  --only <text>        only videos whose path contains <text> (case-insensitive)
  --user-data <dir>    app data folder (default: the desktop app's)
  --python <bin>       Python with the backend's dependencies (default: backend/.venv)`;

function parseArgs(argv) {
  const opts = { pending: false, candidates: true, limit: Infinity };
  const value = (i, flag) => {
    if (i + 1 >= argv.length) throw new Error(`${flag} needs a value`);
    return argv[i + 1];
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    switch (a) {
      case '--out': opts.out = path.resolve(value(i, a)); i++; break;
      case '--pending': opts.pending = true; break;
      case '--no-candidates': opts.candidates = false; break;
      case '--limit': opts.limit = Number(value(i, a)); i++; break;
      case '--only': opts.only = value(i, a).toLowerCase(); i++; break;
      case '--user-data': opts.userData = path.resolve(value(i, a)); i++; break;
      case '--python': opts.python = value(i, a); i++; break;
      case '-h': case '--help': opts.help = true; break;
      default: throw new Error(`Unknown option: ${a}`);
    }
  }
  if (!(opts.limit > 0)) throw new Error('--limit must be a positive number');
  return opts;
}

function defaultUserData() {
  if (process.platform === 'darwin') return path.join(os.homedir(), 'Library', 'Application Support', 'funsync-player');
  if (process.platform === 'win32') return path.join(process.env.APPDATA || os.homedir(), 'funsync-player');
  return path.join(process.env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config'), 'funsync-player');
}

function defaultPython() {
  const venv = path.join(ROOT, 'backend', '.venv', process.platform === 'win32' ? 'Scripts/python.exe' : 'bin/python');
  return fs.existsSync(venv) ? venv : (process.platform === 'win32' ? 'python' : 'python3');
}

/** The backend's frame sampler in a long-lived Python child: one JSON request/response per line. */
function startSampler(python) {
  const code = `
import sys, json, base64
sys.path.insert(0, ${JSON.stringify(path.join(ROOT, 'backend'))})
from services.ffmpeg import sample_gray_frames
for line in sys.stdin:
    req = json.loads(line)
    try:
        r = sample_gray_frames(req["path"], req["min_count"], req["width"])
        r["frames"] = [{"pct": f["pct"], "gray": base64.b64encode(f["gray"]).decode()} for f in r["frames"]]
        out = {"ok": True, **r}
    except Exception as e:
        out = {"ok": False, "error": str(e)}
    sys.stdout.write(json.dumps(out) + "\\n")
    sys.stdout.flush()
`;
  const child = spawn(python, ['-c', code], { stdio: ['pipe', 'pipe', 'inherit'] });
  const lines = readline.createInterface({ input: child.stdout });
  const waiting = [];
  lines.on('line', (line) => waiting.shift()?.(JSON.parse(line)));
  let chain = Promise.resolve();
  const sample = (videoPath, minCount, width) => {
    const job = chain.then(() => new Promise((resolve) => {
      waiting.push(resolve);
      child.stdin.write(`${JSON.stringify({ path: videoPath, min_count: minCount, width })}\n`);
    }));
    chain = job.catch(() => {});
    return job.then((r) => {
      if (!r.ok) throw new Error(r.error);
      return { ...r, frames: r.frames.map((f) => ({ pct: f.pct, gray: new Uint8Array(Buffer.from(f.gray, 'base64')) })) };
    });
  };
  return { sample, stop: () => child.stdin.end() };
}

/** Decode an image with ffmpeg: grayscale frames 128 px wide (up to 6 of an animation). */
function decodeWithFfmpeg(tmpDir) {
  let n = 0;
  return async (bytes) => {
    const file = path.join(tmpDir, `img-${process.pid}-${n++}`);
    fs.writeFileSync(file, bytes);
    try {
      const probe = spawnSync('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height', '-of', 'csv=p=0', file], { encoding: 'utf8' });
      const [srcWidth, srcHeight] = probe.stdout.trim().split('\n')[0].split(',').map(Number);
      if (!srcWidth || !srcHeight) return [];
      const width = Math.min(128, srcWidth);
      const height = Math.max(1, Math.round(width * srcHeight / srcWidth));
      const r = spawnSync('ffmpeg', ['-v', 'error', '-i', file, '-vf', `scale=${width}:${height}:flags=area,format=gray`, '-f', 'rawvideo', 'pipe:1'], { maxBuffer: 1 << 28 });
      const size = width * height;
      const count = Math.floor((r.stdout?.length || 0) / size);
      if (!count) return [];
      const picks = [...new Set(Array.from({ length: Math.min(6, count) }, (_, i) => Math.floor(i * count / Math.min(6, count))))];
      return picks.map((i) => ({ gray: new Uint8Array(r.stdout.subarray(i * size, (i + 1) * size)), width, height, srcWidth, srcHeight }));
    } finally {
      fs.rmSync(file, { force: true });
    }
  };
}

/** A grid of images (ffmpeg tile), each labelled by its index in `items`. */
function contactSheet(items, outFile, tmpDir) {
  if (items.length === 0) return null;
  const dir = fs.mkdtempSync(path.join(tmpDir, 'sheet-'));
  items.forEach((it, i) => fs.copyFileSync(it.file, path.join(dir, `${String(i).padStart(3, '0')}.jpg`)));
  const cols = 6;
  const rows = Math.ceil(items.length / cols);
  const r = spawnSync('ffmpeg', ['-v', 'error', '-y', '-i', path.join(dir, '%03d.jpg'),
    '-vf', `scale=316:178:force_original_aspect_ratio=decrease,pad=320:180:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuvj420p,tile=${cols}x${rows}:padding=4`,
    '-frames:v', '1', '-update', '1', outFile], { encoding: 'utf8' });
  fs.rmSync(dir, { recursive: true, force: true });
  return r.status === 0 ? outFile : null;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) { console.log(USAGE); return 0; }
  const userData = opts.userData || defaultUserData();
  const out = opts.out || path.join(os.tmpdir(), 'funsync-preview-check');
  fs.mkdirSync(out, { recursive: true });
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'funsync-preview-check-'));

  const config = JSON.parse(fs.readFileSync(path.join(userData, 'config.json'), 'utf8'));
  const lib = config.settings?.library || {};
  const custom = lib.customThumbnails || {};
  const links = lib.scriptTags || {};

  const log = require('../electron/logger.js');
  log.transports.file.level = false;
  log.transports.console.level = false;
  const pm = require('../electron/preview-match.js');
  const { createPreviewValidator } = require('../electron/preview-validator.js');
  const { EroScriptsAPI } = require('../electron/eroscripts-api.js');

  let videos = Object.keys(custom).filter((p) => custom[p]?.type === 'image' && custom[p].source === 'eroscripts' && !custom[p].forced)
    .map((p) => ({ path: p, kind: 'poster', entry: custom[p] }));
  if (opts.pending) {
    videos.push(...Object.keys(links).filter((p) => links[p]?.topicId != null && !custom[p]).map((p) => ({ path: p, kind: 'pending' })));
  }
  videos.sort((a, b) => a.path.localeCompare(b.path));
  if (opts.only) videos = videos.filter((v) => v.path.toLowerCase().includes(opts.only));
  videos = videos.slice(0, opts.limit);

  let api = null;
  if (opts.candidates) {
    const session = config.settings?.eroscripts?.session || {};
    if (!session.cookie) throw new Error('Not logged in to EroScripts (needed for the candidates; --no-candidates skips them).');
    api = new EroScriptsAPI();
    api.restoreSession(session.cookie, session.username);
  }

  const sampler = startSampler(opts.python || defaultPython());
  const validator = createPreviewValidator({
    cacheDir: tmpDir,
    sampleVideo: sampler.sample,
    decodeImage: decodeWithFfmpeg(tmpDir),
    log: { warn: () => {} },
  });
  const bytesCache = new Map();
  const download = async (url) => {
    if (!bytesCache.has(url)) bytesCache.set(url, await api.downloadPreviewImage(url));
    return bytesCache.get(url);
  };

  const rows = [];
  const topicsFetched = new Set();
  const started = Date.now();
  for (const [i, v] of videos.entries()) {
    const row = { video: v.path, kind: v.kind, topicId: v.entry?.topicId ?? links[v.path]?.topicId ?? null };
    if (!fs.existsSync(v.path)) {
      row.error = 'video not found';
      rows.push(row);
      continue;
    }
    try {
      if (v.kind === 'poster') {
        row.poster = v.entry.imagePath;
        const s = await validator.scoreImage(v.path, fs.readFileSync(v.entry.imagePath));
        row.current = { accepted: s.accepted, distance: s.distance, reason: s.reason, pct: s.pct, view: s.view };
      }
      if (api && row.topicId != null) {
        if (!topicsFetched.has(row.topicId)) {
          if (topicsFetched.size) await sleep(800);
          topicsFetched.add(row.topicId);
        }
        const extra = [links[v.path]?.previewUrl].filter(Boolean);
        const found = await api.getPreviewCandidates(row.topicId, extra);
        if (!found) {
          row.candidates = { error: 'topic unavailable' };
        } else {
          const { scored, best } = await validator.scoreCandidates(v.path, found.candidates, download);
          row.candidates = {
            excluded: found.excluded.map((e) => ({ url: e.url, reason: e.reason })),
            scored: scored.map((s) => ({ url: s.url, origin: s.origin, distance: s.distance, accepted: s.accepted, reason: s.reason })),
            best: best ? { url: best.url, distance: best.distance } : null,
          };
          if (best) {
            row.bestFile = path.join(tmpDir, `best-${i}`);
            fs.writeFileSync(row.bestFile, best.bytes);
          }
        }
      }
    } catch (err) {
      row.error = err.message;
    }
    rows.push(row);
    const cur = row.current ? `${row.current.accepted ? 'KEEP  ' : 'REMOVE'} d=${row.current.distance ?? '-'}` : 'pending    ';
    const nb = row.candidates?.best ? `new d=${row.candidates.best.distance}` : (row.candidates ? 'no match' : '');
    console.log(`${String(i + 1).padStart(3)}/${videos.length} ${cur} ${nb.padEnd(9)} ${path.basename(v.path)}${row.error ? `  [${row.error}]` : ''}`);
  }
  sampler.stop();

  // ---- Report ----
  const posters = rows.filter((r) => r.kind === 'poster');
  const kept = posters.filter((r) => r.current?.accepted);
  const removed = posters.filter((r) => r.current && !r.current.accepted);
  const unchecked = posters.filter((r) => !r.current);
  const replaced = removed.filter((r) => r.candidates?.best);
  const pending = rows.filter((r) => r.kind === 'pending');
  const summary = {
    threshold: pm.MATCH_THRESHOLD,
    posters: posters.length,
    kept: kept.length,
    removed: removed.length,
    unchecked: unchecked.length,
    removedWithMatchingReplacement: replaced.length,
    keptWhereAnotherCandidateIsCloser: kept.filter((r) => r.candidates?.best && r.candidates.best.distance < r.current.distance).length,
    pending: pending.length,
    pendingWithMatch: pending.filter((r) => r.candidates?.best).length,
    minutes: +((Date.now() - started) / 60000).toFixed(1),
  };
  fs.writeFileSync(path.join(out, 'report.json'), JSON.stringify({ summary, rows: rows.map(({ bestFile, ...r }) => r) }, null, 2));

  const sheets = [];
  const sheet = (list, name, file) => {
    for (let k = 0; k < list.length; k += 48) {
      const f = contactSheet(list.slice(k, k + 48).map((r) => ({ file: file(r) })), path.join(out, `${name}-${k / 48 + 1}.jpg`), tmpDir);
      if (f) sheets.push(path.basename(f));
    }
  };
  sheet(removed, 'removed', (r) => r.poster);
  sheet(kept, 'kept', (r) => r.poster);

  const name = (r) => path.basename(r.video);
  const d = (x) => (x == null ? '–' : x);
  const md = [
    '# EroScripts preview check (dry run)',
    '',
    `Threshold: pHash distance ≤ ${pm.MATCH_THRESHOLD}/64. Run took ${summary.minutes} min.`,
    '',
    '| | count |',
    '|---|---|',
    `| Current EroScripts posters | ${summary.posters} |`,
    `| Accepted (kept) | ${summary.kept} |`,
    `| Rejected (removed, generated frame comes back) | ${summary.removed} |`,
    `| …of which another post image matches the video | ${summary.removedWithMatchingReplacement} |`,
    `| Not checked (video missing / error) | ${summary.unchecked} |`,
    ...(opts.pending ? [`| Linked videos without a poster | ${summary.pending} |`, `| …with a matching post image | ${summary.pendingWithMatch} |`] : []),
    '',
    `Contact sheets: ${sheets.join(', ') || 'none'} (removed-*.jpg in the order of the "Rejected" table).`,
    '',
    '## Rejected',
    '',
    '| # | Video | Distance | Reason | Matching replacement |',
    '|---|---|---|---|---|',
    ...removed.map((r, k) => `| ${k + 1} | ${name(r)} | ${d(r.current.distance)} | ${r.current.reason || ''} | ${r.candidates?.best ? `${r.candidates.best.url} (d=${r.candidates.best.distance})` : '–'} |`),
    '',
    '## Accepted',
    '',
    '| # | Video | Distance | Frame | Best post image |',
    '|---|---|---|---|---|',
    ...kept.map((r, k) => `| ${k + 1} | ${name(r)} | ${r.current.distance} | ${r.current.view} @ ${Math.round((r.current.pct || 0) * 100)}% | ${r.candidates?.best ? `${r.candidates.best.url} (d=${r.candidates.best.distance})` : '–'} |`),
    ...(unchecked.length ? ['', '## Not checked', '', ...unchecked.map((r) => `- ${name(r)}: ${r.error}`)] : []),
    '',
  ];
  fs.writeFileSync(path.join(out, 'report.md'), md.join('\n'));
  fs.rmSync(tmpDir, { recursive: true, force: true });
  console.log(`\n${JSON.stringify(summary)}\nReport: ${path.join(out, 'report.md')}`);
  return 0;
}

main().then((code) => process.exit(code), (err) => {
  console.error(err.message);
  process.exit(1);
});
