'use strict';

// The EroScripts links file: video → EroScripts post links found by
// `npm run eroscripts:sync`, kept in userData beside config.json.
//
// The desktop app owns config.json (electron-conf) and may be running while
// the CLI works, so the CLI never touches it. It writes this file instead;
// the app reads it on every library scan and merges the links into
// `library.scriptTags` (renderer/js/eroscripts-links.js).

const fs = require('fs');
const path = require('path');

const LINKS_FILE = 'eroscripts-links.json';
const LINKS_VERSION = 1;

function linksFilePath(userDataDir) {
  return path.join(userDataDir, LINKS_FILE);
}

/** An empty links document. */
function emptyLinks() {
  return { version: LINKS_VERSION, updatedAt: null, links: {}, review: {} };
}

/**
 * Read the links file. A missing file reads as empty; an unreadable or
 * foreign one returns null so nobody mistakes it for "no links".
 *
 * @param {string} userDataDir
 * @returns {{version: number, updatedAt: string|null, links: object, review: object}|null}
 */
function readLinksFile(userDataDir) {
  let raw;
  try {
    raw = fs.readFileSync(linksFilePath(userDataDir), 'utf8');
  } catch (err) {
    return err.code === 'ENOENT' ? emptyLinks() : null;
  }
  try {
    const data = JSON.parse(raw);
    if (!data || typeof data !== 'object' || data.version !== LINKS_VERSION) return null;
    return {
      ...emptyLinks(),
      ...data,
      links: data.links && typeof data.links === 'object' ? data.links : {},
      review: data.review && typeof data.review === 'object' ? data.review : {},
    };
  } catch {
    return null;
  }
}

/**
 * Write the links file atomically (temp file + rename), so the app never
 * reads half a file.
 */
function writeLinksFile(userDataDir, data) {
  const target = linksFilePath(userDataDir);
  const tmp = `${target}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify({ ...data, version: LINKS_VERSION }, null, 2)}\n`);
  fs.renameSync(tmp, target);
  return target;
}

module.exports = { LINKS_FILE, LINKS_VERSION, linksFilePath, emptyLinks, readLinksFile, writeLinksFile };
