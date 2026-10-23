// Reverse search: link library videos to their EroScripts posts.
//
// Logic for `npm run eroscripts:sync` (scripts/eroscripts-sync.mjs), kept
// apart from the CLI so it can be tested without a forum or a library.
//
// Precision first. A link is applied only when
//   confirmed — the post carries the local main script itself (same
//               normalised actions), or one of the scene's hand-made axis
//               files, byte for byte in its actions; or
//   probable  — a post script has the local script's (normalised) base name
//               AND the same length within 2 s.
// Anything weaker goes on the review list with its candidates.
//
// Forum access is read-only and paced (≥ 700 ms between requests). A 429
// stops the run with progress saved; a 401/403 stops it as an expired session.

import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { getAxisBySuffix, parseAxisSuffix, getBaseName } from '../renderer/js/multi-axis.js';
import { handmadeAxisSuffixes, generatedCompanionFiles } from '../renderer/js/handmade-axes.js';
import { archiveFiles, planFolder, applyPrune } from './prune-generated-lib.mjs';
import { actionsHash, normalisedActions } from './handmade-scenes.mjs';

const require = createRequire(import.meta.url);
const { AXIS_SUFFIXES } = require('../electron/library-scan.js');
const JSZip = require('jszip');

export const FORUM_INTERVAL_MS = 700;
// Discourse's per-user search limit is per minute; a full-library run hit
// it at ~25 searches a minute. 3 s keeps searches to 20 a minute.
export const SEARCH_INTERVAL_MS = 3000;
export const LENGTH_TOLERANCE_MS = 2000;
export const DEFAULT_STALE_DAYS = 7;
const DAY_MS = 24 * 60 * 60 * 1000;
const CANDIDATES_PER_QUERY = 5;
// A further result must share this much of the query to be checked as a
// repost once the video is already confirmed.
const RELATED_MIN_OVERLAP = 0.6;
// A search result is inspected only when its title has at least half of
// the query's significant words.
const CANDIDATE_MIN_OVERLAP = 0.5;
// A review candidate without a name or length near miss must carry this
// much of a query in its title.
const REVIEW_MIN_OVERLAP = 0.75;
const MAX_TOPICS_PER_VIDEO = 8;
const MAX_MAIN_DOWNLOADS_PER_TOPIC = 6;
const MAX_AXIS_DOWNLOADS_PER_TOPIC = 3;
const MAX_REVIEW_CANDIDATES = 5;

// ── Errors ──────────────────────────────────────────────────────────────

export class RateLimitedError extends Error {
  constructor() {
    super('EroScripts is rate limiting requests (HTTP 429). Progress is saved; run the command again later to resume.');
    this.name = 'RateLimitedError';
  }
}

export class SessionExpiredError extends Error {
  constructor() {
    super('The EroScripts session has expired (HTTP 401/403). Log in again from the app (EroScripts panel), then re-run.');
    this.name = 'SessionExpiredError';
  }
}

// ── Names ───────────────────────────────────────────────────────────────

// Release tags that say nothing about which video this is: resolution,
// frame rate, projection, encode. Stripped wherever they appear.
const FORMAT_TAG = String.raw`(?:\d{3,4}p|[1-9]\d?k|\d{2,3}\s?fps|vr|2d|3d|sbs|tb|ou|lr|180|360|hevc|h26[45]|x26[45]|hq|uhd|fhd|hd|upscaled?|passthrough|remaster(?:ed)?)`;
// Version / difficulty words, stripped only as a trailing `_tag` (or ` tag`):
// they choose between a scene's scripts, so they stay in the base name used
// for script matching but not in a search query.
const TRAILING_TAG = String.raw`(?:hard(?:core)?|soft|easy|normal|medium|alt(?:ernate)?|extreme|v\d+(?:\.\d+)*|ver\d+|final)`;
const FORMAT_TOKEN_RE = new RegExp(String.raw`(^|[\s_.\-])${FORMAT_TAG}(?=$|[\s_.\-])`, 'gi');
const TRAILING_TAG_RE = new RegExp(String.raw`[\s_\-]+${TRAILING_TAG}$`, 'i');
const BRACKETS_RE = /\[[^\]]*\]|\([^)]*\)|\{[^}]*\}/g;

const collapse = (s) => s.replace(/\s+/g, ' ').trim();
const stripExt = (name) => name.replace(/\.(funscript|zip|mp4|m4v|mkv|webm|avi|mov|wmv)$/i, '');

function stripFormatTags(s) {
  let prev;
  let out = s;
  do { prev = out; out = out.replace(FORMAT_TOKEN_RE, '$1'); } while (out !== prev);
  return out;
}

function creatorForms(creator) {
  if (!creator || /^unknown$/i.test(creator)) return [];
  const words = creator.split(/\s+/).filter(Boolean);
  const forms = new Set([creator.toLowerCase(), creator.replace(/\s+/g, '').toLowerCase()]);
  if (words.length > 1) forms.add(words.map((w) => w[0]).join('').toLowerCase());
  return [...forms].filter((f) => f.length >= 2);
}

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** Remove a leading `Creator - ` / `FH - ` or a trailing ` - Creator` / ` by Creator`. */
export function stripCreator(title, creator) {
  let out = title;
  for (const form of creatorForms(creator)) {
    const re = new RegExp(String.raw`^${escapeRe(form)}(?:\s*[-:–|]\s*|\s+)`, 'i');
    if (re.test(out) && out.replace(re, '').trim().length >= 3) { out = out.replace(re, ''); break; }
  }
  for (const form of creatorForms(creator)) {
    const re = new RegExp(String.raw`(?:\s+[-–|]\s*|\s+by\s+)${escapeRe(form)}$`, 'i');
    if (re.test(out) && out.replace(re, '').trim().length >= 3) { out = out.replace(re, ''); break; }
  }
  return collapse(out.replace(/^[\s\-–:|]+|[\s\-–:|]+$/g, ''));
}

/**
 * Search title for a video or script file name: no extension, bracketed
 * release info, format tags, trailing version words or creator prefix.
 *
 *   `Fap Hero - Fantasy Play_hard.mp4`, creator `Fap Hero` → `Fantasy Play`
 *   `[Drills3D] Judy Alvarez - Braindance Tryout VR.mp4`  → `Judy Alvarez - Braindance Tryout`
 */
export function cleanVideoName(name, creator = null) {
  let s = stripExt(path.basename(String(name || '')));
  s = s.replace(BRACKETS_RE, ' ');
  s = stripFormatTags(s);
  let prev;
  do { prev = s; s = s.replace(TRAILING_TAG_RE, '').replace(/[\s_\-]+$/, ''); s = stripFormatTags(s); } while (s !== prev);
  s = collapse(s.replace(/_/g, ' '));
  return stripCreator(s, creator);
}

/** Search-safe form: Discourse search treats punctuation as noise or syntax. */
function queryText(s) {
  return collapse(s.replace(/[^\p{L}\p{N}' ]+/gu, ' '));
}

/**
 * Up to three search queries for a video, most specific first: the cleaned
 * title, the creator plus title, and the title without a trailing
 * `_character` label (`Cyber Sex_eve` → `Cyber Sex`).
 */
export function queryVariants(videoName, creator = null) {
  const title = cleanVideoName(videoName, creator);
  const out = [];
  const add = (q) => {
    const text = queryText(q || '');
    if (text.length >= 3 && !out.some((x) => x.toLowerCase() === text.toLowerCase())) out.push(text);
  };
  // A one-word title ("Bad", "365") finds little on its own: lead with the creator.
  const withCreator = creatorForms(creator).length ? `${creator} ${title}` : null;
  if (withCreator && tokens(title).size < 2) add(withCreator);
  add(title);
  if (withCreator) add(withCreator);
  const stem = stripExt(path.basename(String(videoName || '')));
  const underscore = stem.lastIndexOf('_');
  if (underscore > 0) add(cleanVideoName(stem.slice(0, underscore), creator));
  return out.slice(0, 3);
}

/**
 * Base name used to pair a post's script with the local one: format tags,
 * brackets and the creator go; difficulty words stay (`_hard` ≠ `_soft`).
 */
export function normalisedBase(fileName, creator = null) {
  let s = getBaseName(path.basename(String(fileName || '')));
  s = stripExt(s).replace(/\.generated$/i, '');
  s = stripFormatTags(s.replace(BRACKETS_RE, ' '));
  s = stripCreator(collapse(s.replace(/_/g, ' ')), creator);
  s = collapse(s.replace(/[.\-–]+/g, ' '));
  return collapse(s.toLowerCase().replace(/[^\p{L}\p{N} ]+/gu, ' '));
}

const STOPWORDS = new Set(['the', 'and', 'with', 'for', 'from', 'pmv', 'hmv', 'part', 'multi', 'axis', 'script', 'scripts', 'funscript', 'version', 'versions']);
// Significant words: 3+ characters, or 2 with a digit ("2B", "P2").
function tokens(s) {
  return new Set(String(s || '').toLowerCase().split(/[^\p{L}\p{N}]+/u)
    .filter((t) => (t.length >= 3 || (t.length === 2 && /\d/.test(t))) && !STOPWORDS.has(t)));
}

/** Share of the query's significant words found in a topic title, 0..1. */
export function titleOverlap(query, title) {
  const q = tokens(query);
  if (q.size === 0) return 0;
  const t = tokens(title);
  let hit = 0;
  for (const w of q) if (t.has(w)) hit++;
  return hit / q.size;
}

/**
 * Is a search result worth inspecting? Half the query's words in its
 * title, or, for a query naming the creator, a title naming the creator
 * too: bundles ("[Agapilord] HMV Hexa-Bundle! (6 scripts)") carry the
 * creator but not every video's title.
 */
export function isRelevant(query, title, creator) {
  const overlap = titleOverlap(query, title);
  if (overlap >= CANDIDATE_MIN_OVERLAP) return true;
  if (overlap === 0 || creatorForms(creator).length === 0) return false;
  const c = tokens(creator);
  const q = tokens(query);
  const t = tokens(title);
  return c.size > 0 && [...c].every((w) => q.has(w) && t.has(w));
}

// ── Scripts ─────────────────────────────────────────────────────────────

/** Parse funscript JSON (BOM tolerated); null when it isn't one. */
export function parseScript(text) {
  try {
    const data = JSON.parse(String(text).replace(/^\uFEFF/, ''));
    return data && typeof data === 'object' ? data : null;
  } catch {
    return null;
  }
}

/** Time of the last action, ms (0 when empty). */
export function scriptLengthMs(script) {
  const acts = normalisedActions(script);
  return acts.length ? acts[acts.length - 1][0] : 0;
}

const isFunscript = (name) => /\.funscript$/i.test(name);
const isZip = (name) => /\.zip$/i.test(name);
const isGeneratedName = (name) => /\.generated\.[^.]+\.funscript$/i.test(name);
const axisOf = (name) => (isGeneratedName(name) ? null : parseAxisSuffix(path.basename(name)));

/** Read a file's script entries: a funscript itself, or the funscripts in a zip. */
export async function scriptEntries(name, bytes) {
  if (isZip(name)) {
    let zip;
    try { zip = await JSZip.loadAsync(bytes); } catch { return []; }
    const out = [];
    for (const entry of Object.values(zip.files)) {
      const base = entry.name.split('/').pop();
      if (entry.dir || !isFunscript(base) || base.startsWith('._') || entry.name.includes('__MACOSX/')) continue;
      out.push({ entry: entry.name, name: base, script: parseScript(await entry.async('string')) });
    }
    return out;
  }
  return [{ entry: null, name, script: parseScript(bytes.toString('utf8')) }];
}

// ── Pacing & forum access ───────────────────────────────────────────────

/**
 * At least `intervalMs` between successive `wait()` resolutions.
 * `now`/`sleep` injectable for fake timers.
 */
export function createPacer({
  intervalMs = FORUM_INTERVAL_MS,
  now = () => Date.now(),
  sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
} = {}) {
  let last = -Infinity;
  let count = 0;
  return {
    async wait() {
      const delay = last + intervalMs - now();
      if (delay > 0) await sleep(delay);
      last = now();
      count++;
    },
    get count() { return count; },
  };
}

function checkForumResult(r) {
  if (r?.rateLimited) throw new RateLimitedError();
  if (r?.authExpired) throw new SessionExpiredError();
}

/**
 * Paced, read-only forum access with per-run memo of topics and files and a
 * persistent search cache (`cache.search`, kept for `staleDays`).
 *
 * Every request waits on `pacer`; searches also wait on `searchPacer`, as
 * Discourse limits searches per user per minute well below other requests.
 *
 * @param {object} api EroScriptsAPI (search, getTopicAttachments, fetchScript)
 */
export function createForum(api, {
  pacer = createPacer(),
  searchPacer = createPacer({ intervalMs: SEARCH_INTERVAL_MS }),
  cache = {}, now = () => Date.now(), staleDays = DEFAULT_STALE_DAYS,
} = {}) {
  cache.search ||= {};
  // Expired searches would only be re-run: drop them so the file stays small.
  for (const [q, hit] of Object.entries(cache.search)) {
    if (!(now() - Date.parse(hit?.at) < staleDays * DAY_MS)) delete cache.search[q];
  }
  const topics = new Map();
  const files = new Map();
  return {
    pacer,
    async search(query) {
      const hit = cache.search[query];
      if (hit && now() - Date.parse(hit.at) < staleDays * DAY_MS) return hit.results;
      await searchPacer.wait();
      await pacer.wait();
      const r = await api.search(query);
      checkForumResult(r);
      if (r.error) throw new Error(`search "${query}": ${r.error}`);
      const results = (r.results || []).map((t) => ({
        id: t.id, title: t.title, url: t.url, creator: t.creator || null,
        tags: t.tags || [], category: t.category || null, paid: !!t.paid,
        preview: t.preview || null,
      }));
      cache.search[query] = { at: new Date(now()).toISOString(), results };
      return results;
    },
    async topic(topicId) {
      if (topics.has(topicId)) return topics.get(topicId);
      await pacer.wait();
      const r = await api.getTopicAttachments(topicId);
      checkForumResult(r);
      // "No posts found" and similar are an empty topic, not a failure.
      const out = { attachments: r.attachments || [], linked: r.linked || [], external: r.external || [], paid: !!r.paid, preview: r.preview || null, error: r.error || null };
      topics.set(topicId, out);
      return out;
    },
    /** Script entries of a forum upload (memoised); [] when unavailable. */
    async entries(url, name) {
      if (files.has(url)) return files.get(url);
      await pacer.wait();
      const r = await api.fetchScript(url);
      checkForumResult(r);
      const out = r.success ? await scriptEntries(name, r.bytes) : [];
      files.set(url, out);
      return out;
    },
  };
}

// ── Local scene ─────────────────────────────────────────────────────────

/**
 * What matching needs to know about a library video's scene.
 *
 * @param {{path: string, funscriptPath: string, creator?: string}} video
 */
export function loadLocalScene(video) {
  const dir = path.dirname(video.funscriptPath);
  const script = parseScript(fs.readFileSync(video.funscriptPath, 'utf8'));
  const files = fs.readdirSync(dir).filter((n) => !n.startsWith('._')).map((n) => path.join(dir, n));
  const handmade = handmadeAxisSuffixes({ primaryPath: video.funscriptPath, allFiles: files, parsed: script });
  // Hand-made companion files by canonical axis, for the axis-hash check.
  const stem = path.basename(video.funscriptPath, '.funscript');
  const axisFiles = new Map();
  for (const f of files) {
    const n = path.basename(f);
    const axis = axisOf(n);
    if (!axis || getBaseName(n).toLowerCase() !== stem.toLowerCase()) continue;
    if (!axisFiles.has(axis.suffix)) axisFiles.set(axis.suffix, f);
  }
  return {
    video,
    dir,
    stem,
    files,
    script,
    hash: actionsHash(script),
    lengthMs: scriptLengthMs(script),
    base: normalisedBase(video.funscriptPath, video.creator),
    handmade,
    axisFiles,
    _axisHashes: new Map(),
    axisHash(axis) {
      if (!this._axisHashes.has(axis)) {
        const f = this.axisFiles.get(axis);
        this._axisHashes.set(axis, f ? actionsHash(parseScript(fs.readFileSync(f, 'utf8'))) : null);
      }
      return this._axisHashes.get(axis);
    },
  };
}

// ── Matching ────────────────────────────────────────────────────────────

const fmtLen = (ms) => {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};

/**
 * Classify one downloaded post script against the local scene.
 *
 * @returns {{confidence: 'confirmed'|'probable'|null, basis?: string, reason?: string}}
 */
export function classifyScript(local, { name, script }, creator = local.video.creator) {
  const hash = actionsHash(script);
  if (hash && local.hash && hash === local.hash) return { confidence: 'confirmed', basis: 'main-hash' };
  const sameBase = normalisedBase(name, creator) === local.base && local.base.length > 0;
  const len = scriptLengthMs(script);
  const lengthOk = len > 0 && local.lengthMs > 0 && Math.abs(len - local.lengthMs) <= LENGTH_TOLERANCE_MS;
  if (sameBase && lengthOk) return { confidence: 'probable', basis: 'name+length' };
  if (sameBase) return { confidence: null, reason: `name matches, length differs (${fmtLen(len)} vs ${fmtLen(local.lengthMs)})` };
  if (lengthOk) return { confidence: null, reason: `length matches (${fmtLen(len)}), name differs: ${name}` };
  return { confidence: null };
}

/**
 * Inspect one topic: download its main scripts (and, failing a match, the
 * axis files the scene has hand-made copies of) and classify.
 */
export async function evaluateTopic(forum, local, candidate) {
  const topic = await forum.topic(candidate.id);
  const result = {
    candidate,
    paid: topic.paid || !!candidate.paid,
    confidence: null,
    basis: null,
    matchedFile: null,
    reasons: [],
    mains: [],
    axes: [],
    external: [...new Set((topic.external || []).map((e) => e.host))],
    preview: topic.preview || null,
  };

  const describe = (a, extra = {}) => ({
    name: a.name, url: a.url, host: a.host || 'eroscripts',
    postNumber: a.postNumber ?? null, username: a.username ?? null, byAuthor: !!a.byAuthor, ...extra,
  });

  // Axis files listed by the post: forum uploads and pixeldrain links.
  for (const a of [...topic.attachments, ...topic.linked]) {
    if (!isFunscript(a.name)) continue;
    const axis = axisOf(a.name);
    if (axis) result.axes.push(describe(a, { axis: axis.suffix, base: getBaseName(a.name) }));
  }

  // Main scripts: forum funscripts without an axis suffix, and zips. Names
  // closest to the local script first, author before replies otherwise.
  const mains = topic.attachments
    .filter((a) => (isFunscript(a.name) && !axisOf(a.name) && !isGeneratedName(a.name)) || isZip(a.name))
    .map((a, i) => ({ a, i, score: normalisedBase(a.name, local.video.creator) === local.base ? 2 : titleOverlap(local.base, a.name) }))
    .sort((x, y) => y.score - x.score || x.i - y.i)
    .slice(0, MAX_MAIN_DOWNLOADS_PER_TOPIC);

  let best = null;
  for (const { a } of mains) {
    const entries = await forum.entries(a.url, a.name);
    for (const e of entries) {
      const axis = axisOf(e.name);
      if (axis) {
        result.axes.push(describe(a, { name: e.name, entry: e.entry, zip: a.name, axis: axis.suffix, base: getBaseName(e.name), lengthMs: scriptLengthMs(e.script) }));
        continue;
      }
      if (!e.script) continue;
      result.mains.push(describe(a, { name: e.name, entry: e.entry, zip: e.entry ? a.name : undefined, base: getBaseName(e.name), lengthMs: scriptLengthMs(e.script) }));
      const verdict = classifyScript(local, e);
      if (verdict.confidence === 'confirmed') {
        best = { ...verdict, file: e.name };
        break;
      }
      if (verdict.confidence === 'probable' && !best) best = { ...verdict, file: e.name };
      if (verdict.reason) result.reasons.push(verdict.reason);
    }
    if (best?.confidence === 'confirmed') break;
  }

  // No main-script match: a hand-made axis file of the scene, identical in
  // the post, identifies the post just as well.
  if (best?.confidence !== 'confirmed' && local.axisFiles.size > 0) {
    const tries = result.axes
      .filter((x) => x.host === 'eroscripts' && !x.entry && local.axisFiles.has(x.axis))
      .slice(0, MAX_AXIS_DOWNLOADS_PER_TOPIC);
    for (const x of tries) {
      const [e] = await forum.entries(x.url, x.name);
      x.lengthMs = scriptLengthMs(e?.script);
      const hash = actionsHash(e?.script);
      if (hash && hash === local.axisHash(x.axis)) {
        best = { confidence: 'confirmed', basis: 'axis-hash', file: x.name };
        break;
      }
    }
  }

  if (best) {
    result.confidence = best.confidence;
    result.basis = best.basis;
    result.matchedFile = best.file;
  } else {
    if (result.paid) result.reasons.push('paid post (no attached scripts)');
    else if (mains.length === 0 && topic.linked.length === 0 && result.external.length) result.reasons.push(`scripts hosted off-forum (${result.external.join(', ')})`);
    else if (mains.length === 0) result.reasons.push('no forum-hosted main script');
    else if (result.reasons.length === 0) result.reasons.push(`no matching script (${result.mains.length} checked)`);
  }
  return result;
}

function linkRecord(evaluation, local, now) {
  const c = evaluation.candidate;
  const record = {
    topicId: c.id,
    topicUrl: c.url || `https://discuss.eroscripts.com/t/${c.id}`,
    title: c.title || null,
    creator: c.creator || null,
    tags: c.tags || [],
    category: c.category || null,
    paid: evaluation.paid,
    confidence: evaluation.confidence,
    basis: evaluation.basis,
    matchedFile: evaluation.matchedFile,
    matchedAt: new Date(now).toISOString(),
    checkedAt: new Date(now).toISOString(),
    localLengthMs: local.lengthMs,
    mains: evaluation.mains.map(({ name, base, lengthMs, byAuthor, username, postNumber }) => ({ name, base, lengthMs, byAuthor, username, postNumber })),
    axes: evaluation.axes,
  };
  // Search results carry a poster-size `preview` only when the topic's image
  // is big enough; the topic endpoint may know a better one.
  const preview = c.preview || evaluation.preview;
  if (preview) record.previewUrl = preview;
  return record;
}

/**
 * Find the post for one video.
 *
 * The first confirmed post becomes the link. Reposts are common (a
 * "[Multi-Axis Initiative]" thread re-uploading the original with axes), so
 * the remaining close-titled results of the same search are checked too;
 * those that also carry the scene's script are kept as `related`, and their
 * axis files count towards the hand-made axes report.
 *
 * @returns {Promise<{outcome: 'confirmed'|'probable'|'review'|'none',
 *   link?: object, review?: object}>}
 */
export async function matchVideo(forum, local, { knownTopicIds = [], now = Date.now() } = {}) {
  const video = local.video;
  const queries = queryVariants(path.basename(video.path), video.creator);
  const seen = new Set();
  const evaluated = [];
  let probable = null;
  let primary = null;
  const related = [];

  const consider = async (candidate, query) => {
    if (seen.has(candidate.id)) {
      // A saved link checked by id alone: take its title etc. from the search.
      const ev = evaluated.find((e) => e.candidate.id === candidate.id);
      if (ev && !ev.candidate.title && candidate.title) ev.candidate = { ...candidate, ...Object.fromEntries(Object.entries(ev.candidate).filter(([, v]) => v != null)) };
      return;
    }
    if (evaluated.length >= MAX_TOPICS_PER_VIDEO) return;
    seen.add(candidate.id);
    const ev = await evaluateTopic(forum, local, candidate);
    ev.query = query;
    evaluated.push(ev);
    if (ev.confidence === 'confirmed') {
      if (!primary) primary = ev;
      else related.push(ev);
    } else if (ev.confidence === 'probable' && !probable) {
      probable = ev;
    }
  };

  for (const id of knownTopicIds) await consider({ id, title: null, url: null }, '(saved link)');
  for (const query of queries) {
    const results = await forum.search(query);
    const relevant = results.filter((t) => isRelevant(query, t.title, video.creator)).slice(0, CANDIDATES_PER_QUERY);
    for (const t of relevant) {
      if (primary && titleOverlap(query, t.title) < RELATED_MIN_OVERLAP) continue;
      await consider(t, query);
    }
    if (primary) break;
  }
  if (primary) {
    const link = linkRecord(primary, local, now);
    if (related.length) {
      link.related = related.map((ev) => {
        const r = linkRecord(ev, local, now);
        return {
          topicId: r.topicId, topicUrl: r.topicUrl, title: r.title, creator: r.creator,
          confidence: r.confidence, basis: r.basis, matchedFile: r.matchedFile, mains: r.mains, axes: r.axes,
        };
      });
    }
    return { outcome: 'confirmed', link };
  }
  if (probable) return { outcome: 'probable', link: linkRecord(probable, local, now) };

  // The review list is for near misses, not every search hit: a candidate
  // stays when its title carries most of the query, or a script in it
  // matched by name or by length.
  const candidates = evaluated
    .map((ev) => ({
      topicId: ev.candidate.id,
      title: ev.candidate.title,
      url: ev.candidate.url,
      creator: ev.candidate.creator || null,
      paid: ev.paid,
      overlap: Math.round(Math.max(0, ...queries.map((q) => titleOverlap(q, ev.candidate.title || ''))) * 100) / 100,
      nearMiss: ev.reasons.some((r) => /^(name|length) matches/.test(r)),
      reasons: [...new Set(ev.reasons)].slice(0, 3),
    }))
    .filter((c) => c.nearMiss || c.overlap >= REVIEW_MIN_OVERLAP)
    .sort((a, b) => Number(b.nearMiss) - Number(a.nearMiss) || b.overlap - a.overlap)
    .slice(0, MAX_REVIEW_CANDIDATES)
    .map(({ nearMiss, ...c }) => c);
  if (candidates.length === 0) return { outcome: 'none', queries };
  return { outcome: 'review', review: { checkedAt: new Date(now).toISOString(), queries, candidates } };
}

// ── Staleness & the run ─────────────────────────────────────────────────

/** True when `iso` is missing or older than `days`. */
export function isStale(iso, now, days = DEFAULT_STALE_DAYS) {
  const t = Date.parse(iso || '');
  return !Number.isFinite(t) || now - t >= days * DAY_MS;
}

/**
 * What to do with a video this run.
 *
 * @returns {'skip'|'match'|'refresh-topic'}
 */
export function planVideo(state, videoPath, { now, refresh = false, retry = false, staleDays = DEFAULT_STALE_DAYS }) {
  if (refresh) return 'match';
  if (retry && state.links[videoPath]?.confidence !== 'confirmed') return 'match';
  const link = state.links[videoPath];
  if (link?.confidence === 'confirmed') return isStale(link.checkedAt, now, staleDays) ? 'refresh-topic' : 'skip';
  const last = state.checked?.[videoPath]?.checkedAt || link?.checkedAt;
  return isStale(last, now, staleDays) ? 'match' : 'skip';
}

/**
 * Run the reverse search over `videos`.
 *
 * @param {object} args
 * @param {Array<{path: string, funscriptPath: string|null, creator?: string}>} args.videos
 * @param {object} args.forum from createForum
 * @param {object} args.state links document (mutated)
 * @param {(state: object) => void} [args.persist] called after every video
 * @param {Object<string, number[]>} [args.knownTopics] videoPath → topic ids the app already links
 * @returns {Promise<{checked: number, skipped: number, noScript: number,
 *   outcomes: object, errors: Array<{video: string, error: string}>, stopped: string|null}>}
 */
export async function runSync({
  videos, forum, state, persist = () => {}, knownTopics = {},
  now = () => Date.now(), refresh = false, retry = false, limit = Infinity, staleDays = DEFAULT_STALE_DAYS,
  onVideo = () => {},
}) {
  state.links ||= {};
  state.review ||= {};
  state.checked ||= {};
  const summary = { checked: 0, skipped: 0, noScript: 0, outcomes: { confirmed: 0, probable: 0, review: 0, none: 0 }, errors: [], stopped: null };

  for (const video of videos) {
    if (!video.funscriptPath) { summary.noScript++; continue; }
    const plan = planVideo(state, video.path, { now: now(), refresh, retry, staleDays });
    if (plan === 'skip') { summary.skipped++; continue; }
    if (summary.checked >= limit) break;
    summary.checked++;
    try {
      const local = loadLocalScene(video);
      if (plan === 'refresh-topic') {
        const link = state.links[video.path];
        const ev = await evaluateTopic(forum, local, { id: link.topicId, title: link.title, url: link.topicUrl, creator: link.creator, tags: link.tags, category: link.category, preview: link.previewUrl });
        const t = new Date(now()).toISOString();
        if (ev.confidence === 'confirmed') {
          const fresh = linkRecord(ev, local, now());
          state.links[video.path] = { ...link, axes: fresh.axes, mains: fresh.mains, checkedAt: t, localLengthMs: local.lengthMs };
          if (!link.previewUrl && fresh.previewUrl) state.links[video.path].previewUrl = fresh.previewUrl;
          summary.outcomes.confirmed++;
          onVideo(video, 'confirmed', state.links[video.path]);
          persist(state);
          continue;
        }
        // The post no longer carries the local script: match from scratch.
      }
      const r = await matchVideo(forum, local, { knownTopicIds: knownTopics[video.path] || [], now: now() });
      const t = new Date(now()).toISOString();
      state.checked[video.path] = { checkedAt: t, outcome: r.outcome };
      if (r.link) {
        state.links[video.path] = r.link;
        delete state.review[video.path];
      } else {
        delete state.links[video.path];
        if (r.review) state.review[video.path] = r.review;
        else delete state.review[video.path];
      }
      summary.outcomes[r.outcome]++;
      onVideo(video, r.outcome, r.link || r.review || null);
    } catch (err) {
      if (err instanceof RateLimitedError) { summary.stopped = 'rate-limited'; persist(state); return summary; }
      if (err instanceof SessionExpiredError) { persist(state); throw err; }
      summary.errors.push({ video: video.path, error: err.message });
      onVideo(video, 'error', err.message);
    }
    persist(state);
  }
  return summary;
}

// ── Hand-made axes from linked posts ────────────────────────────────────

/** Suffix for an installed file: the spelling the library scan pairs. */
export function installSuffix(axis) {
  const def = getAxisBySuffix(axis);
  if (!def) return axis;
  return [def.suffix, ...(def.aliases || [])].find((n) => AXIS_SUFFIXES.has(n)) || def.suffix;
}

/**
 * The axis set of one post that belongs to the local scene.
 *
 * Posts can carry axes for several versions (soft / hard); the set used is
 * the one named like the post's matched script, else the only set, else
 * the one named like the local script. Otherwise the post is ambiguous.
 */
function postAxisSet(post, local) {
  const creator = local.video.creator;
  const groups = new Map();
  for (const x of post.axes || []) {
    const key = normalisedBase(x.base || x.name, creator);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(x);
  }
  if (groups.size === 0) return null;
  let key = null;
  let reason = null;
  const matchedKey = post.matchedFile ? normalisedBase(post.matchedFile, creator) : null;
  if (matchedKey && groups.has(matchedKey)) { key = matchedKey; reason = 'named like the matched script'; }
  else if (groups.size === 1) { key = [...groups.keys()][0]; reason = 'only axis set in the post'; }
  else if (groups.has(local.base)) { key = local.base; reason = 'named like the local script'; }
  if (key == null) return { ambiguous: true, groups: [...groups.values()].map((g) => g[0].base) };

  // Install guard: the post's main script is the local one (confirmed), or
  // the set's own main script has the local script's length.
  const setMain = (post.mains || []).find((m) => normalisedBase(m.base || m.name, creator) === key);
  const lengthOk = !!setMain && Math.abs((setMain.lengthMs || 0) - local.lengthMs) <= LENGTH_TOLERANCE_MS;
  const guard = post.confidence === 'confirmed' ? 'confirmed' : lengthOk ? 'length' : null;
  return { ambiguous: false, base: groups.get(key)[0].base, reason, guard, files: groups.get(key) };
}

/**
 * Axis files the linked posts (the link and its `related` reposts) offer
 * that the scene lacks as hand-made. Per axis the first file wins (link
 * before reposts, author before replies); the rest are alternatives.
 *
 * @param {object} link stored link
 * @param {object} local from loadLocalScene
 * @returns {{missing: Array<{axis: string, file: object, alternatives: object[],
 *   installable: boolean, guard: string|null}>, sets: object[], ambiguous: object[]}}
 */
export function availableAxes(link, local) {
  const have = new Set(local.handmade);
  const byAxis = new Map();
  const sets = [];
  const ambiguous = [];
  for (const post of [link, ...(link.related || [])]) {
    const set = postAxisSet(post, local);
    if (!set) continue;
    const where = { topicId: post.topicId, topicUrl: post.topicUrl, title: post.title };
    if (set.ambiguous) { ambiguous.push({ ...where, groups: set.groups }); continue; }
    sets.push({ ...where, base: set.base, reason: set.reason, guard: set.guard });
    for (const x of set.files) {
      if (have.has(x.axis)) continue;
      const file = { ...x, ...where, guard: set.guard };
      if (!byAxis.has(x.axis)) byAxis.set(x.axis, []);
      byAxis.get(x.axis).push(file);
    }
  }
  const installableFile = (f) => !!f.guard && f.host === 'eroscripts';
  const missing = [...byAxis.entries()].map(([axis, files]) => {
    const file = files.find(installableFile) || files[0];
    return {
      axis,
      file,
      alternatives: files.filter((f) => f !== file),
      guard: file.guard,
      installable: installableFile(file),
    };
  });
  return { missing, sets, ambiguous };
}

/**
 * Install the missing axes of one scene, then archive the stem's generated
 * companions. Never overwrites; refuses files that aren't scripts or run
 * past the local script.
 *
 * @returns {Promise<{installed: string[], skipped: Array<{axis: string, reason: string}>, archived: Array<{from: string, to: string}>}>}
 */
export async function installAxes({ forum, local, plan, archiveDir, relativeTo, dryRun = false }) {
  const installed = [];
  const skipped = [];
  for (const m of plan.missing) {
    if (!m.installable) { skipped.push({ axis: m.axis, reason: m.guard ? `hosted on ${m.file.host}` : 'post script not confirmed and length unverified' }); continue; }
    const target = path.join(local.dir, `${local.stem}.${installSuffix(m.axis)}.funscript`);
    if (fs.existsSync(target)) { skipped.push({ axis: m.axis, reason: 'file exists' }); continue; }
    const entries = await forum.entries(m.file.url, m.file.zip || m.file.name);
    const e = m.file.entry ? entries.find((x) => x.entry === m.file.entry) : entries[0];
    const acts = normalisedActions(e?.script);
    if (acts.length < 2) { skipped.push({ axis: m.axis, reason: 'not a funscript' }); continue; }
    if (acts[acts.length - 1][0] > local.lengthMs + LENGTH_TOLERANCE_MS) { skipped.push({ axis: m.axis, reason: 'longer than the local script' }); continue; }
    if (!dryRun) {
      try {
        fs.writeFileSync(target, JSON.stringify(e.script), { flag: 'wx' });
      } catch (err) {
        if (err.code === 'EEXIST') { skipped.push({ axis: m.axis, reason: 'file exists' }); continue; }
        throw err;
      }
    }
    installed.push(target);
  }
  let archived = [];
  if (installed.length > 0) {
    const files = dryRun ? local.files : fs.readdirSync(local.dir).filter((n) => !n.startsWith('._')).map((n) => path.join(local.dir, n));
    const generated = generatedCompanionFiles(local.video.funscriptPath, files);
    if (generated.length) archived = await archiveFiles(generated, { archiveDir, relativeTo, dryRun });
  }
  return { installed, skipped, archived };
}

/**
 * The folder rule after `installAxes` (see prune-generated-lib.mjs): in each
 * folder that got hand-made axes, copy them to sibling videos whose stroke
 * script's timing provably matches and archive every generated file left.
 *
 * @param {Array<{dir: string, archiveDir: string, relativeTo: string}>} folders
 * @returns {Promise<Array<{dir: string, copied: string[], refused: string[],
 *   archived: Array<{from: string, to: string}>, errors: string[]}>>}
 *   `refused`: sibling videos that got nothing (not timing-compatible)
 */
export async function pruneInstalledFolders(folders) {
  const out = [];
  for (const { dir, archiveDir, relativeTo } of folders) {
    const plan = await planFolder(dir, archiveDir, relativeTo);
    if (!plan) continue;
    const { copied, archived, errors } = await applyPrune([plan], archiveDir);
    const refused = plan.siblings.filter((s) => !s.source).map((s) => s.video);
    if (copied.length || archived.length || refused.length || errors.length) out.push({ dir, copied, refused, archived, errors });
  }
  return out;
}

// ── Report ──────────────────────────────────────────────────────────────

const rel = (p, roots) => {
  for (const r of roots) if (p.startsWith(`${r}/`)) return `${path.basename(r)}/${p.slice(r.length + 1)}`;
  return p;
};

/**
 * Markdown report of the links file plus this run's results.
 */
export function renderReport({ state, summary, axes = [], siblings = [], previews, roots = [], dryRun = false, now = Date.now() }) {
  const links = Object.entries(state.links || {});
  const confirmed = links.filter(([, l]) => l.confidence === 'confirmed');
  const probable = links.filter(([, l]) => l.confidence === 'probable');
  const review = Object.entries(state.review || {});
  const out = [];
  out.push(`# EroScripts sync report — ${new Date(now).toISOString().slice(0, 16).replace('T', ' ')}${dryRun ? ' (dry run)' : ''}`, '');
  out.push('## Summary', '');
  out.push(`- Linked: ${confirmed.length + probable.length} (confirmed ${confirmed.length}, probable ${probable.length})`);
  out.push(`- Review list: ${review.length}`);
  out.push(`- Checked this run: ${summary.checked} (skipped ${summary.skipped} up to date, ${summary.noScript} without a script)`);
  if (summary.errors.length) out.push(`- Errors: ${summary.errors.length}`);
  if (summary.stopped) out.push(`- Stopped early: ${summary.stopped} (progress saved; re-run to resume)`);
  if (previews) out.push(`- Previews available: ${previews.available} (${previews.withUrl} with a known image URL; ${previews.withoutThumb} videos have no custom thumbnail yet)`);
  const axisScenes = axes.filter((a) => a.plan.missing.length);
  out.push(`- Hand-made axes available: ${axisScenes.reduce((n, a) => n + a.plan.missing.length, 0)} files in ${axisScenes.length} scenes`);
  const installedCount = axes.reduce((n, a) => n + (a.result?.installed.length || 0), 0);
  if (installedCount) out.push(`- Hand-made axes installed: ${installedCount}`);
  out.push('');

  const linkLine = ([p, l]) => {
    const also = (l.related || []).map((r) => `[${r.topicId}](${r.topicUrl})`).join(', ');
    return `- \`${rel(p, roots)}\` → [${l.title || l.topicId}](${l.topicUrl}) by ${l.creator || '?'} — ${l.basis}${l.matchedFile ? ` (\`${l.matchedFile}\`)` : ''}${also ? `; also in ${also}` : ''}`;
  };
  if (confirmed.length) out.push('## Confirmed', '', ...confirmed.map(linkLine), '');
  if (probable.length) out.push('## Probable', '', ...probable.map(linkLine), '');
  if (review.length) {
    out.push('## Review (not applied)', '');
    for (const [p, r] of review) {
      out.push(`- \`${rel(p, roots)}\` — searched: ${r.queries.map((q) => `"${q}"`).join(', ')}`);
      for (const c of r.candidates) out.push(`  - [${c.title}](${c.url})${c.paid ? ' (paid)' : ''}: ${c.reasons.join('; ') || 'no match'}`);
    }
    out.push('');
  }
  if (axisScenes.length || axes.some((a) => a.plan.ambiguous.length)) {
    out.push('## Hand-made axes in linked posts', '');
    for (const a of axes) {
      if (!a.plan.missing.length && !a.plan.ambiguous.length) continue;
      out.push(`- \`${rel(a.video, roots)}\``);
      for (const m of a.plan.missing) {
        const from = m.file.byAuthor ? 'author' : `reply #${m.file.postNumber} by ${m.file.username}`;
        const alt = m.alternatives.length ? `; also ${m.alternatives.map((x) => `\`${x.name}\` (${x.topicId})`).join(', ')}` : '';
        const res = a.result?.installed.some((f) => f.endsWith(`.${installSuffix(m.axis)}.funscript`)) ? ' — **installed**' : '';
        const how = m.installable ? `installable: ${m.guard}` : m.guard ? `not installable: ${m.file.host}` : 'not installable: unverified';
        out.push(`  - ${m.axis}: \`${m.file.name}\`${m.file.zip ? ` in \`${m.file.zip}\`` : ''} — [${m.file.topicId}](${m.file.topicUrl}), ${from}, ${how}${alt}${res}`);
      }
      for (const amb of a.plan.ambiguous) out.push(`  - [${amb.topicId}](${amb.topicUrl}): several axis sets, none matches this scene (${amb.groups.map((g) => `\`${g}\``).join(' / ')})`);
      for (const s of a.result?.skipped || []) out.push(`  - ${s.axis}: not installed — ${s.reason}`);
      for (const x of a.result?.archived || []) out.push(`  - archived \`${path.basename(x.from)}\``);
    }
    out.push('');
  }
  if (siblings.length) {
    out.push('## Sibling videos in folders that got hand-made axes', '');
    for (const f of siblings) {
      out.push(`- \`${rel(f.dir, roots)}\``);
      for (const c of f.copied) out.push(`  - copied \`${path.basename(c)}\``);
      for (const v of f.refused) out.push(`  - \`${path.basename(v)}\`: sibling not timing-compatible, nothing copied`);
      for (const x of f.archived) out.push(`  - archived \`${path.basename(x.from)}\``);
      for (const e of f.errors) out.push(`  - error: ${e}`);
    }
    out.push('');
  }
  if (summary.errors.length) {
    out.push('## Errors', '', ...summary.errors.map((e) => `- \`${rel(e.video, roots)}\`: ${e.error}`), '');
  }
  return out.join('\n');
}

// ── Library helpers ─────────────────────────────────────────────────────

/** `<Source>/<Creator>/<Scene>/file` → Creator (null for files at the root). */
export function creatorOf(videoPath, sourceRoots) {
  const root = sourceRoots.filter((r) => videoPath.startsWith(`${r}/`)).sort((a, b) => b.length - a.length)[0];
  if (!root) return null;
  const parts = videoPath.slice(root.length + 1).split('/');
  return parts.length > 1 ? parts[0] : null;
}

/** Where the userData of the desktop app lives on this OS. */
export function defaultUserDataDir(platform = process.platform, env = process.env, home = env.HOME || env.USERPROFILE || '') {
  if (platform === 'darwin') return path.join(home, 'Library', 'Application Support', 'funsync-player');
  if (platform === 'win32') return path.join(env.APPDATA || path.join(home, 'AppData', 'Roaming'), 'funsync-player');
  return path.join(env.XDG_CONFIG_HOME || path.join(home, '.config'), 'funsync-player');
}

// ── launchd schedule (macOS) ────────────────────────────────────────────

export const LAUNCHD_LABEL = 'com.funsync.eroscripts-sync';

/** `3d` / `12h` / `90m` → seconds; null when unreadable. */
export function parseInterval(text) {
  const m = String(text || '').trim().match(/^(\d+)\s*([dhm])$/i);
  if (!m || Number(m[1]) <= 0) return null;
  return Number(m[1]) * { d: 86400, h: 3600, m: 60 }[m[2].toLowerCase()];
}

const xml = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** launchd agent running the sync every `intervalSeconds`, logging into userData. */
export function launchdPlist({ nodePath, scriptPath, workingDirectory, intervalSeconds, logPath, args = [], pathEnv = '/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin' }) {
  const argv = [nodePath, scriptPath, ...args].map((a) => `    <string>${xml(a)}</string>`).join('\n');
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
${argv}
  </array>
  <key>WorkingDirectory</key>
  <string>${xml(workingDirectory)}</string>
  <key>StartInterval</key>
  <integer>${intervalSeconds}</integer>
  <key>RunAtLoad</key>
  <false/>
  <key>ProcessType</key>
  <string>Background</string>
  <key>LowPriorityIO</key>
  <true/>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>${xml(pathEnv)}</string>
  </dict>
  <key>StandardOutPath</key>
  <string>${xml(logPath)}</string>
  <key>StandardErrorPath</key>
  <string>${xml(logPath)}</string>
</dict>
</plist>
`;
}
