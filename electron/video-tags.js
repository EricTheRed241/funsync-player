'use strict';

// The video tags file: video → tags a tagging job decided (vision-classified
// `vr`, `pov`, `creampie`, `riding`), kept in userData beside config.json.
//
// The desktop app owns config.json and rebuilds `library.scriptTags` from the
// scene sidecars and the EroScripts links on every scan, so tags written
// straight into scriptTags would be lost. The `video-tags` CLI
// (crates/video-tags, Rust) writes this file instead; the app reads it on
// every library scan and merges it into `library.scriptTags`
// (renderer/js/video-tags.js).

const fs = require('fs');
const path = require('path');

const VIDEO_TAGS_FILE = 'video-tags.json';
const VIDEO_TAGS_VERSION = 1;

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

/**
 * Read the video tags file. A missing, unreadable or foreign file returns
 * null, which the merge treats as "change nothing": only a valid file can
 * withdraw tags it added before.
 *
 * @param {string} userDataDir
 * @returns {{version: number, updatedAt: string|null, videos: Object<string, {tags: string[]}>}|null}
 */
function readVideoTagsFile(userDataDir) {
  const file = videoTagsFilePath(userDataDir);
  let raw;
  try {
    raw = fs.readFileSync(file, 'utf8');
  } catch (err) {
    logOnce(err.code === 'ENOENT' ? `${file} not found; no video tags applied` : `${file} unreadable: ${err.message}`);
    return null;
  }
  try {
    const data = JSON.parse(raw);
    if (!data || typeof data !== 'object' || data.version !== VIDEO_TAGS_VERSION
      || !data.videos || typeof data.videos !== 'object' || Array.isArray(data.videos)) {
      logOnce(`${file} is not a version ${VIDEO_TAGS_VERSION} video tags file; ignored`);
      return null;
    }
    return { version: data.version, updatedAt: data.updatedAt ?? null, videos: data.videos };
  } catch (err) {
    logOnce(`${file} is not valid JSON (${err.message}); ignored`);
    return null;
  }
}

module.exports = { VIDEO_TAGS_FILE, VIDEO_TAGS_VERSION, videoTagsFilePath, readVideoTagsFile };
