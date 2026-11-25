'use strict';

// The video tags file: video → tags a tagging job decided (vision-classified
// `vr`, `pov`, `creampie`, `riding`), kept in userData beside config.json.
//
// The desktop app owns config.json and rebuilds `library.scriptTags` from the
// scene sidecars and the EroScripts links on every scan, so tags written
// straight into scriptTags would be lost. The `video-tags` CLI
// (crates/video-tags, Rust) writes this file instead; the app reads it on
// every library scan and merges it into `library.scriptTags`
// (renderer/js/video-tags.js). The CLIs that rename scenes move its keys
// with them (`rewriteVideoTagsFile`; `library:reorganize` rewrites every
// JSON file in userData).
//
// The pornstar tags file (`pornstar-tags.json`, crates/pornstar-tags) is
// merged the same way, together with this one: video → the performers'
// tags, plus each performer's scene count and whether her tag is applied.
// Its CLI recomputes it from the library on every run, so renames heal
// there.

const fs = require('fs');
const path = require('path');

const VIDEO_TAGS_FILE = 'video-tags.json';
const VIDEO_TAGS_VERSION = 1;
const PORNSTAR_TAGS_FILE = 'pornstar-tags.json';
const PORNSTAR_TAGS_VERSION = 1;

/** Problems already logged, so a missing file isn't reported on every scan. */
const logged = new Set();

function logOnce(message) {
  if (logged.has(message)) return;
  logged.add(message);
  console.warn(`[video-tags] ${message}`);
}

function videoTagsFilePath(userDataDir) {
  return path.join(userDataDir, VIDEO_TAGS_FILE);
}

function pornstarTagsFilePath(userDataDir) {
  return path.join(userDataDir, PORNSTAR_TAGS_FILE);
}

const isMap = (v) => !!v && typeof v === 'object' && !Array.isArray(v);

/**
 * Read a userData side file: its parsed content when it's a JSON object of
 * `version` whose `keys` are all objects, else null (logged once).
 */
function readSideFile(file, version, keys, what) {
  let raw;
  try {
    raw = fs.readFileSync(file, 'utf8');
  } catch (err) {
    logOnce(err.code === 'ENOENT' ? `${file} not found; no ${what} applied` : `${file} unreadable: ${err.message}`);
    return null;
  }
  try {
    const data = JSON.parse(raw);
    if (!isMap(data) || data.version !== version || !keys.every((k) => isMap(data[k]))) {
      logOnce(`${file} is not a version ${version} ${what} file; ignored`);
      return null;
    }
    return data;
  } catch (err) {
    logOnce(`${file} is not valid JSON (${err.message}); ignored`);
    return null;
  }
}

/**
 * Read the video tags file. A missing, unreadable or foreign file returns
 * null, which the merge treats as "change nothing": only a valid file can
 * withdraw tags it added before.
 *
 * @param {string} userDataDir
 * @returns {{version: number, updatedAt: string|null, videos: Object<string, {tags: string[]}>}|null}
 */
function readVideoTagsFile(userDataDir) {
  const data = readSideFile(videoTagsFilePath(userDataDir), VIDEO_TAGS_VERSION, ['videos'], 'video tags');
  return data && { version: data.version, updatedAt: data.updatedAt ?? null, videos: data.videos };
}

/**
 * Read the pornstar tags file. A missing, unreadable or foreign file returns
 * null: the merge then applies no pornstar tags (and withdraws those it
 * added), video tags unaffected.
 *
 * @param {string} userDataDir
 * @returns {{version: number, updatedAt: string|null,
 *   pornstars: Object<string, {name: string, scenes: number, applied: boolean}>,
 *   videos: Object<string, string[]>}|null}
 */
function readPornstarTagsFile(userDataDir) {
  const data = readSideFile(pornstarTagsFilePath(userDataDir), PORNSTAR_TAGS_VERSION, ['pornstars', 'videos'], 'pornstar tags');
  return data && {
    version: data.version, updatedAt: data.updatedAt ?? null, pornstars: data.pornstars, videos: data.videos,
  };
}

/**
 * Rewrite the file's content in place after scenes were renamed, e.g.
 * `(data) => remapPaths(data, moves)`. Missing or unparsable: left alone.
 * On a change the previous file is kept as `video-tags.json.bak` and the
 * new one written atomically (temp file + rename), as the CLI does.
 *
 * @param {string} userDataDir
 * @param {(data: object) => {value: object, changed: number}} update
 * @returns {number} how many paths changed
 */
function rewriteVideoTagsFile(userDataDir, update) {
  const file = videoTagsFilePath(userDataDir);
  let data;
  try {
    data = JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return 0;
  }
  const { value, changed } = update(data);
  if (!changed) return 0;
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`);
  fs.copyFileSync(file, `${file}.bak`);
  fs.renameSync(tmp, file);
  return changed;
}

module.exports = {
  VIDEO_TAGS_FILE, VIDEO_TAGS_VERSION, videoTagsFilePath, readVideoTagsFile, rewriteVideoTagsFile,
  pornstarTagsFilePath, readPornstarTagsFile,
};
