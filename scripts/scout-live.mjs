// Live action in Discovery, for every scout (scripts/eroscripts-scout.mjs,
// scripts/faptap-scout.mjs).
//
// The scouts take animated scenes. Live action gets in two ways:
//   • by a favourite: a creator or actress the user likes and already has in
//     the library, listed in `<userData>/scout-favourites.json`
//     (`{ "favourites": ["Noodledude", "Candy Love", …] }`, created with
//     DEFAULT_FAVOURITES on first use and edited by hand);
//   • on the quota: anyone else, one in every LIVE_QUOTA_EVERY downloads,
//     to discover new live action. Every scene either scout downloads counts;
//     `<userData>/scout-live-quota.json` holds `{ downloads, live }`.

import fs from 'node:fs';
import path from 'node:path';

export const LIVE_QUOTA_EVERY = 20;
export const DEFAULT_FAVOURITES = ['ScyllaHMV', 'Agapilord', 'Eternal Enigma', 'Cuddle Mocap', 'Noodledude', 'NoFaceGirl', 'Candy Love'];

// Letters and digits, single-spaced, like eroscripts-scout-lib's normName
// (that module imports this one).
const normName = (s) => String(s || '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
const squash = (s) => normName(s).replace(/ /g, '');

/**
 * The favourite a text names (a title, uploader, performer or tag), or null.
 * Whole words (`Candy Love`, not `Candy Lovers`), and run together
 * (`ScyllaHMV` matches `Scylla HMV`, `NoFaceGirl` matches `no face girl`). A
 * one-word CamelCase name also matches its first part when that part is
 * long enough to be distinctive (`ScyllaHMV` matches `Scylla Compilation`).
 *
 * @param {string[]} texts
 * @param {string[]} favourites
 */
export function favouriteIn(texts, favourites) {
  const names = (texts || []).filter(Boolean).map((t) => normName(t).split(' '));
  // Consecutive words run together: `no face girl` → `nofacegirl`.
  const runs = (words, joined) => words.some((_, i) => {
    let s = '';
    for (let j = i; j < words.length && s.length < joined.length; j++) s += words[j];
    return s === joined;
  });
  for (const fav of favourites || []) {
    const spaced = normName(fav);
    if (!spaced) continue;
    const head = /\s/.test(fav.trim()) ? '' : (fav.match(/^\p{Lu}?\p{Ll}+(?=\p{Lu})/u)?.[0] || '').toLowerCase();
    if (names.some((w) => ` ${w.join(' ')} `.includes(` ${spaced} `) || runs(w, squash(fav)) || (head.length >= 6 && w.includes(head)))) return fav;
  }
  return null;
}

/** True when the next download may be a non-favourite live scene: one in every LIVE_QUOTA_EVERY. */
export function liveAllowed(quota) {
  return quota.live < Math.floor((quota.downloads + 1) / LIVE_QUOTA_EVERY);
}

/** Count one finished download (mutates `quota`). */
export function countDownload(quota, quotaLive) {
  quota.downloads++;
  if (quotaLive) quota.live++;
}

// ── Files ───────────────────────────────────────────────────────────────

const favouritesPath = (userData) => path.join(userData, 'scout-favourites.json');
const quotaPath = (userData) => path.join(userData, 'scout-live-quota.json');

function writeJson(file, value) {
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(value, null, 1)}\n`);
  fs.renameSync(tmp, file);
}

/** The favourites list; written with the defaults when missing. */
export function readFavourites(userData) {
  try {
    const list = JSON.parse(fs.readFileSync(favouritesPath(userData), 'utf8'))?.favourites;
    if (Array.isArray(list)) return list.filter((s) => typeof s === 'string' && s.trim());
  } catch (err) {
    if (err.code !== 'ENOENT') throw new Error(`${favouritesPath(userData)}: ${err.message}`);
  }
  writeJson(favouritesPath(userData), { favourites: DEFAULT_FAVOURITES });
  return [...DEFAULT_FAVOURITES];
}

export function readQuota(userData) {
  try {
    const q = JSON.parse(fs.readFileSync(quotaPath(userData), 'utf8'));
    if (Number.isInteger(q?.downloads) && Number.isInteger(q?.live)) return q;
  } catch { /* first download */ }
  return { downloads: 0, live: 0 };
}

/** Read, count one download, write: both scouts may run at once. */
export function recordDownload(userData, quotaLive) {
  const q = readQuota(userData);
  countDownload(q, quotaLive);
  writeJson(quotaPath(userData), q);
  return q;
}
