'use strict';

// Triage deletions: a video the user puts in the No thank you playlist or
// rates 2 stars or lower (desktop, web remote or HereSphere; see
// renderer/js/reject-rule.js) is deleted from disk, together with its
// scripts, axis companions (hand-made and generated) and subtitles. A scene
// folder left with nothing but its scout sidecar (`eroscripts.json` or
// `faptap.json`) goes too, and so does a creator folder it leaves empty.
//
// Rejections are remembered in `<userData>/eroscripts-scout-rejected.json`
// so neither `npm run eroscripts:scout` nor `npm run faptap:scout` downloads
// them again: EroScripts scenes under `<topicId>|<video>`, FapTap scenes
// under `faptap|<faptapId>`. The EroScripts scout adds its own entries there
// (videos shorter than a minute). No Electron dependency: the scout CLIs
// read and write the same file through this module.

const fs = require('fs');
const path = require('path');
const { AXIS_SUFFIXES, VIDEO_EXTS, SUBTITLE_EXTS, isAppleDoubleSidecar } = require('./library-scan');

const REJECT_PLAYLIST = 'No thank you';
const REJECTED_FILE = 'eroscripts-scout-rejected.json';
const REJECTED_VERSION = 1;
// Why a video went: the user's triage (scout never takes the post again) or
// the scout's own length check.
const REASONS = new Set(['no-thank-you', 'low-rating', 'short']);
// What may remain in a scene folder that is then removed whole.
const LEFTOVERS = new Set(['eroscripts.json', 'faptap.json', '.ds_store']);

// exFAT and APFS compare names case- and normalization-insensitively.
const fold = (s) => String(s).normalize('NFC').toLowerCase();

/** True for the reject playlist's name, whatever its case or spacing. */
function isRejectPlaylist(name) {
  return fold(String(name || '').trim()) === fold(REJECT_PLAYLIST);
}

/** Key of one post video, as the scout records fetched scenes. */
function rejectionKey(topicId, video) {
  return `${topicId}|${video}`;
}

/** Key of one FapTap scene, as the FapTap scout records it. */
function faptapRejectionKey(faptapId) {
  return `faptap|${faptapId}`;
}

function rejectedFilePath(userDataDir) {
  return path.join(userDataDir, REJECTED_FILE);
}

/** The rejection list; missing or unreadable reads as empty. */
function readRejected(userDataDir) {
  try {
    const data = JSON.parse(fs.readFileSync(rejectedFilePath(userDataDir), 'utf8'));
    if (data?.version === REJECTED_VERSION && Array.isArray(data.entries)) return data;
  } catch { /* none yet */ }
  return { version: REJECTED_VERSION, entries: [] };
}

/**
 * Add one entry (replacing an older one with the same key), atomically.
 * FapTap entries (`faptapId`) are keyed by `faptapRejectionKey`, the others
 * by `rejectionKey(topicId, video)`.
 */
function recordRejection(userDataDir, entry) {
  const data = readRejected(userDataDir);
  const key = entry.faptapId != null && entry.faptapId !== ''
    ? faptapRejectionKey(entry.faptapId)
    : rejectionKey(entry.topicId, entry.video);
  data.entries = [...data.entries.filter((e) => e.key !== key), { key, ...entry }];
  const target = rejectedFilePath(userDataDir);
  const tmp = `${target}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(data, null, 1)}\n`);
  fs.renameSync(tmp, target);
}

/** File names that belong to `videoName`: itself, its scripts, axes and subtitles. */
function companionNames(videoName) {
  const stem = videoName.slice(0, -path.extname(videoName).length);
  const names = [videoName, `${stem}.funscript`, ...SUBTITLE_EXTS.map((ext) => stem + ext)];
  for (const axis of AXIS_SUFFIXES) names.push(`${stem}.${axis}.funscript`, `${stem}.generated.${axis}.funscript`);
  return new Set(names.map(fold));
}

/**
 * Remove a file and its AppleDouble `._` sidecar. exFAT lists names
 * decomposed (NFD) but finds some `._` files only by their composed (NFC)
 * spelling, so both are tried.
 */
function removeFile(dir, name) {
  for (const n of [name, `._${name}`]) {
    for (const form of new Set([n, n.normalize('NFC')])) fs.rmSync(path.join(dir, form), { force: true });
  }
}

/** Remove `dir` when only `keep`-able leftovers remain; true when it went. */
function removeDirIfOnly(dir, keep) {
  let names;
  try { names = fs.readdirSync(dir); } catch { return false; }
  if (names.some((n) => !isAppleDoubleSidecar(n) && !keep(n))) return false;
  for (const n of names) if (!isAppleDoubleSidecar(n)) removeFile(dir, n);
  for (const n of fs.readdirSync(dir)) removeFile(dir, n);
  try { fs.rmdirSync(dir); } catch { return false; }
  removeFile(path.dirname(dir), `._${path.basename(dir)}`);
  return true;
}

/**
 * Delete a rejected video with its companions (see the file comment).
 * Only paths inside a library source are touched; a video already gone is
 * not an error.
 *
 * @param {string} videoPath
 * @param {object} o
 * @param {string[]} o.sources library source folders
 * @param {string} o.userDataDir where the rejection list lives
 * @param {number} [o.now]
 * @param {'no-thank-you'|'low-rating'} [o.reason]
 * @returns {{deleted: string[], removedDirs: string[]}}
 */
function deleteRejectedVideo(videoPath, { sources, userDataDir, now = Date.now(), reason = 'no-thank-you' }) {
  if (!REASONS.has(reason) || reason === 'short') throw new Error(`unknown reason: ${reason}`);
  const resolved = path.resolve(String(videoPath || ''));
  const roots = (sources || []).filter(Boolean).map((s) => path.resolve(s));
  if (!roots.some((r) => fold(resolved).startsWith(fold(r) + path.sep))) {
    throw new Error(`not inside a library source: ${videoPath}`);
  }
  if (!VIDEO_EXTS.includes(path.extname(resolved).toLowerCase())) throw new Error(`not a video: ${videoPath}`);
  const dir = path.dirname(resolved);
  let names;
  try { names = fs.readdirSync(dir); } catch { return { deleted: [], removedDirs: [] }; }
  const wanted = companionNames(path.basename(resolved));
  const hits = names.filter((n) => !isAppleDoubleSidecar(n) && wanted.has(fold(n)));
  if (!hits.some((n) => fold(n) === fold(path.basename(resolved)))) return { deleted: [], removedDirs: [] };

  const readSidecar = (name) => {
    try { return JSON.parse(fs.readFileSync(path.join(dir, name), 'utf8')); } catch { return null; }
  };
  const at = new Date(now).toISOString();
  const sidecar = readSidecar('eroscripts.json');
  if (sidecar?.topicId != null && sidecar.video) {
    recordRejection(userDataDir, {
      topicId: sidecar.topicId, video: sidecar.video, title: sidecar.title || null, url: sidecar.url || null,
      reason, path: resolved, at,
    });
  } else if (sidecar?.topicId == null) {
    const scene = readSidecar('faptap.json');
    if (scene?.faptapId != null && scene.faptapId !== '') {
      recordRejection(userDataDir, {
        source: 'faptap', faptapId: String(scene.faptapId), title: scene.title || null, url: scene.url || null,
        reason, path: resolved, at,
      });
    }
  }

  const deleted = [];
  for (const n of hits) {
    removeFile(dir, n);
    deleted.push(path.join(dir, n));
  }
  const removedDirs = [];
  const isRoot = (d) => roots.some((r) => fold(r) === fold(d));
  if (!isRoot(dir) && removeDirIfOnly(dir, (n) => LEFTOVERS.has(fold(n)))) {
    removedDirs.push(dir);
    const parent = path.dirname(dir);
    if (!isRoot(parent) && removeDirIfOnly(parent, (n) => fold(n) === '.ds_store')) removedDirs.push(parent);
  }
  return { deleted, removedDirs };
}

module.exports = {
  REJECT_PLAYLIST, REJECTED_FILE, isRejectPlaylist, rejectionKey, faptapRejectionKey, readRejected, recordRejection,
  deleteRejectedVideo,
};
