// Discovery scout: find free EroScripts posts worth adding to the library,
// and plan which scripts and which video file to download.
//
// Logic for `npm run eroscripts:scout` (scripts/eroscripts-scout.mjs), kept
// apart from the CLI so it can be tested without a forum, MEGA or pixeldrain.
//
// Rules (the tags, words, lengths, caps and weights are in
// renderer/js/content-preferences.js; this module applies them)
//   • Free Scripts only (the CLI lists that category).
//   • Never excluded content (tags, title or first post). Scenes already in
//     the library stay.
//   • Monsters, feet and FFM (tags, title words or first post) aren't
//     preferred: they come in, ranked lower (lessPreferred, their
//     TAG_SCORE_GROUPS group).
//   • Animated and live action alike (user, 2026-11-07: the live quota is
//     gone). Flat live action that shows it isn't POV (a non-POV tag, or a
//     PMV / HMV / music compilation without POV) is turned down
//     (liveNotPov, contentRuleBreak `live-not-pov`); favourites (title, tags
//     or poster; scripts/scout-live.mjs) and picks skip that.
//   • No sex rule (user, 2026-11-13): posts without sex tags or words come
//     in too (a multi-axis script can be worth it untagged) and the ranking
//     decides (tag scores, AUTO_FETCH_MIN_SCORE for `--fetch --auto`).
//     Never a post tagged sfw / non-porn (`excluded`). `--fetch` and every
//     scan drop saved candidates that now break the content rules
//     (contentRuleBreak).
//   • Swipes (user, 2026-11-13): a post swiped right in `npm run
//     scout:report`'s deck is wanted, `--fetch --auto` takes it whatever its
//     score or axes; a left swipe only hides it from the deck (readSwipes).
//   • Videos shorter than MIN_VIDEO_MS (by the main script) aren't
//     downloaded; shorter scenes fetched under older rules stay.
//   • Single-axis and multi-axis scenes alike (user, 2026-11-07: hand-made
//     axes are a preference, not a requirement); multi-axis posts rank
//     higher (RANKING.multiAxis) and download first.
//   • Ranking: RANKING weights (VR, library creator, favourite artist,
//     multi-axis, likes; 2D hentai only ranks lower) plus the tag scores
//     (TAG_SCORE_GROUPS: creampie, riding, music, POV up; monster, feet, FFM
//     down; the user's changes from `library.tagScores`); the favourite
//     artist bonus and the favourites are the user's too
//     (`library.favouriteArtistScore`, `library.favouriteArtists`).
//     There's no hentai feed; hentai comes in through the other feeds.
//   • Skip posts the library already links and scenes whose video is
//     already in the library.
//   • Video variant: within the VR caps, low bitrate when offered; flat
//     within its cap. Previews and trailers never.
//   • Single-axis scenes get generated twist / roll / pitch companions after
//     download (scripts/upgrade-multi-axis.mjs); hand-made axes always win.

import path from 'node:path';
import { createRequire } from 'node:module';
import { currentFavourite, favouriteIn } from './scout-live.mjs';
import { fold as foldName, isDistinctive, isNoiseWord, titleWords, titlesMatch } from './scene-dedupe-lib.mjs';
import {
  ANIMATION_TAGS, AUTO_FETCH_MIN_SCORE, BLOCKED_CREATORS, BLOCKED_TAGS, BLOCKED_TITLE, COMPILATION_TITLE, EXCLUDED_TEXT, FEED_TAGS, FLAT_MAX_HEIGHT,
  MIN_VIDEO_MS, MUSIC_TAGS, NON_POV_TAGS, PMV_TAGS, PMV_TITLE, POV_TAGS, POV_TITLE, SEARCH_FEEDS, SFW_TAGS, VR_MAX_FPS, VR_MAX_HEIGHT,
} from '../renderer/js/content-preferences.js';
import { isHentaiOnly, lessPreferred, candidateScore, inTagGroup, postTagScores } from '../renderer/js/preference-score.js';
import { canonicalArtist } from '../renderer/js/artist-playlists.js';
import { durationMatches, RULE34VIDEO_HOST, rule34videoPosterUrl } from './stream-hosts.mjs';
import { VRBANGERS_HOST } from './vrbangers-host.mjs';
import { createPacer, RateLimitedError, SessionExpiredError, SEARCH_INTERVAL_MS } from './eroscripts-sync-lib.mjs';

const { postImages } = createRequire(import.meta.url)('../electron/eroscripts-api.js');

// The ranking (renderer/js/preference-score.js, shared with the app's library
// score) is part of this module's API for the scouts.
export { candidateScore, effectiveTagScores, hasFeet, hasMonster, isHentaiOnly, lessPreferred, scoreCandidate } from '../renderer/js/preference-score.js';
// So are artist aliases (renderer/js/artist-playlists.js, ARTIST_ALIASES).
export { canonicalArtist };

/** The minimum as words for messages: `1:50`. */
export const MIN_VIDEO_LABEL = `${Math.floor(MIN_VIDEO_MS / 60_000)}:${String(Math.round(MIN_VIDEO_MS / 1000) % 60).padStart(2, '0')}`;

const VIDEO_EXT_RE = /\.(mp4|m4v|mkv|webm|mov)$/i;
const SCRIPT_AXES = new Set(['surge', 'sway', 'twist', 'roll', 'pitch', 'vib', 'lube', 'pump', 'suction', 'valve']);
// TCode channel suffixes some posts use instead of names (`Scene.R0.funscript`).
const TCODE_AXES = { L0: '', L1: 'surge', L2: 'sway', R0: 'twist', R1: 'roll', R2: 'pitch' };

// ── Names ───────────────────────────────────────────────────────────────

export const isVideoName = (name) => VIDEO_EXT_RE.test(String(name || ''));

/** Letters and digits of any script (CJK included), folded (scene-dedupe-lib fold: NFKC, lower case, no accents), single-spaced. */
export function normName(name) {
  return foldName(name).replace(VIDEO_EXT_RE, '').replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
}

/** normName without encode, variant and projection words (`1080p`, `8K60FPS`, `VR`, `low bitrate`, `alt` …; NAME_NOISE_WORDS). */
export function coreName(name) {
  return normName(name).split(' ').filter((w) => w && !isNoiseWord(w)).join(' ');
}

/** True when the words of `small` appear in `large` in a row (`work that body` in `jjulez work that body`, not `art` in `artoria`). */
export function containsWords(large, small) {
  return !!small && ` ${large} `.includes(` ${small} `);
}

/**
 * `Scene.twist.funscript` → `{ base: 'Scene', axis: 'twist' }`; the main
 * script has axis ''. TCode suffixes map to names (R0 → twist, L0 → main).
 * A video extension left inside the base (`Scene.mp4.funscript`) is dropped.
 * null for anything that isn't a funscript.
 */
export function scriptParts(name) {
  const m = String(name || '').match(/^(.*?)(?:\.([A-Za-z0-9]+))?\.funscript$/i);
  if (!m) return null;
  const [, stem, suffix] = m;
  const clean = (s) => s.replace(VIDEO_EXT_RE, '');
  if (suffix && SCRIPT_AXES.has(suffix.toLowerCase())) return { base: clean(stem), axis: suffix.toLowerCase() };
  if (suffix && suffix.toUpperCase() in TCODE_AXES) return { base: clean(stem), axis: TCODE_AXES[suffix.toUpperCase()] };
  return { base: clean(suffix ? `${stem}.${suffix}` : stem), axis: '' };
}

const digitWords = (s) => s.split(' ').filter((w) => /^\d+$/.test(w));

/**
 * How well a video file name matches a script base, 0..1 (0 = not this
 * video). Same core name is 1; numbers must agree (Part 1 is not Part 2,
 * 7 is not 17); then containment of a long enough name, then ≥ 75 % of the
 * base's words.
 */
export function matchScore(base, videoName) {
  const b = coreName(base);
  const v = coreName(videoName);
  if (!b || !v) return 0;
  if (b === v) return 1;
  const vWords = new Set(v.split(' '));
  if (digitWords(b).some((n) => !vWords.has(n))) return 0;
  if (Math.min(b.length, v.length) >= 6 && (v.includes(b) || b.includes(v))) return 0.95;
  const words = b.split(' ');
  const hit = words.filter((w) => vWords.has(w)).length;
  return hit >= 2 && hit / words.length >= 0.75 ? 0.9 * (hit / words.length) : 0;
}

/** True when two core names are the same video (library duplicate check). */
export function sameVideo(a, b) {
  if (!a || !b) return false;
  if (a === b) return true;
  const bWords = new Set(b.split(' '));
  const aWords = a.split(' ');
  if (digitWords(a).some((n) => !bWords.has(n))) return false;
  if (Math.min(a.length, b.length) >= 8 && (containsWords(a, b) || containsWords(b, a))) return true;
  return aWords.length >= 3 && aWords.filter((w) => bWords.has(w)).length / aWords.length >= 0.75;
}

// ── Screening ───────────────────────────────────────────────────────────

/** True when text names excluded content (whole words, any script). */
export function hasExcludedText(text) {
  return EXCLUDED_TEXT.test(String(text || ''));
}

/**
 * True when a post or video is by a BLOCKED_CREATORS name: in its title, as
 * its poster / uploader (`author`), its artist, or one of its performers.
 * Names compare without case, spaces or punctuation (`[Aiden Het]`, `aidenhet`).
 *
 * @param {{title?: string, author?: string, artist?: string, performers?: string[]}} post
 */
export function isBlockedCreator({ title = '', author = '', artist = '', performers = [] }) {
  const compact = (s) => String(s || '').normalize('NFC').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '');
  const people = new Set([author, artist, ...performers].map(compact).filter(Boolean));
  const inTitle = compact(title);
  return BLOCKED_CREATORS.map(compact).some((n) => people.has(n) || inTitle.includes(n));
}

/** True when a post or video carries a BLOCKED_TAGS tag or BLOCKED_TITLE in its title (MMD). */
export function hasBlockedTag({ tags = [], title = '' }) {
  return tags.some((t) => BLOCKED_TAGS.has(t)) || BLOCKED_TITLE.test(String(title || ''));
}

/** True when a post or video is POV: a POV_TAGS tag or POV as a title word. */
export function isPov({ tags = [], title = '' }) {
  return tags.some((t) => POV_TAGS.has(t)) || POV_TITLE.test(String(title || ''));
}

/**
 * True when live action breaks the live rule: flat, not a favourite's, not
 * picked, not POV, and something shows it isn't POV: a NON_POV_TAGS tag, or a
 * music compilation (PMV_TAGS, `compilation`, PMV_TITLE, COMPILATION_TITLE).
 * Without any indication it passes.
 *
 * @param {{tags?: string[], title?: string, live?: boolean, vr?: boolean, picked?: boolean, favourite?: string|null}} post
 */
export function liveNotPov({ tags = [], title = '', live, vr, picked, favourite }) {
  if (!live || vr || picked || favourite || isPov({ tags, title })) return false;
  const t = String(title || '');
  return tags.some((x) => NON_POV_TAGS.has(x) || PMV_TAGS.has(x) || x === 'compilation') || PMV_TITLE.test(t) || COMPILATION_TITLE.test(t);
}

/**
 * Why a post or video breaks the content rules every scout list obeys
 * (favourites and picks included, except where noted), judged by its tags
 * and title, so a saved candidate can be checked again: `blocked`
 * (isBlockedCreator, hasBlockedTag), `live-not-pov` (liveNotPov; favourites
 * and picks skip this one), or null when it doesn't. Monsters and feet only
 * rank lower (lessPreferred).
 *
 * @param {{tags?: string[], title?: string, author?: string, artist?: string,
 *   live?: boolean, vr?: boolean, picked?: boolean, favourite?: string|null}} post
 */
export function contentRuleBreak({ tags = [], title = '', author = '', artist = '', live, vr, picked, favourite }) {
  if (isBlockedCreator({ title, author, artist }) || hasBlockedTag({ tags, title })) return 'blocked';
  if (liveNotPov({ tags, title, live, vr, picked, favourite })) return 'live-not-pov';
  return null;
}

/**
 * First pass on a listed topic, before any request for its files. Posts
 * tagged sfw / non-porn count as excluded.
 *
 * @param {{id: number, title: string, tags: string[]}} topic
 * @param {object} ctx
 * @param {Set<number>} ctx.knownTopics posts the library already links
 * @param {string[]} ctx.creators creator names in the library (lower case)
 * @param {string[]} ctx.sceneNames normName of every library scene folder
 * @param {string[]} [ctx.favourites] favourite creators and actresses (scripts/scout-live.mjs)
 * @param {boolean} [ctx.picked] the user asked for this post by id: the live-not-POV rule doesn't apply
 * @returns {{ok: false, reason: string} | {ok: true, music: boolean, vr: boolean, artist: string|null,
 *   live: boolean, favourite: string|null, hentaiOnly: boolean}}
 *   reasons: known, excluded, blocked, live-not-pov, in-library
 */
export function screenTopic(topic, { knownTopics = new Set(), creators = [], sceneNames = [], favourites = [], picked = false } = {}) {
  const tags = topic.tags || [];
  if (knownTopics.has(topic.id)) return { ok: false, reason: 'known' };
  if (tags.some((t) => hasExcludedText(t) || SFW_TAGS.has(t)) || hasExcludedText(topic.title)) return { ok: false, reason: 'excluded' };
  if (isBlockedCreator({ title: topic.title }) || hasBlockedTag({ tags, title: topic.title })) return { ok: false, reason: 'blocked' };
  const title = normName(topic.title);
  const artist = creators.find((c) => c.length > 3 && title.includes(normName(c))) || null;
  const live = !(tags.some((t) => ANIMATION_TAGS.has(t)) && !tags.includes('real'));
  const vr = tags.includes('vr');
  const favourite = favouriteIn([topic.title, ...tags], favourites);
  if (liveNotPov({ tags, title: topic.title, live, vr, picked, favourite })) return { ok: false, reason: 'live-not-pov' };
  const music = tags.some((t) => MUSIC_TAGS.has(t));
  // In the library: a scene folder's name in the title or the title in it, as whole words. Only a
  // title that says which video it is (two distinctive words, or 8+ letters of them; not `Pain`,
  // `Fap Hero` or an emoji-only title) counts.
  const distinct = [...titleWords(topic.title)].filter(isDistinctive);
  const telling = distinct.length >= 2 || distinct.join(' ').length >= 8;
  if (telling && sceneNames.some((s) => s.length > 8 && (containsWords(title, s) || containsWords(s, title)))) return { ok: false, reason: 'in-library' };
  return {
    ok: true, music, vr, artist, live, favourite,
    hentaiOnly: isHentaiOnly({ tags, title: topic.title }),
  };
}

/**
 * Second pass, on the first post's text (tags stripped): the exclusions in
 * the text, then what it names that ranks lower (`monster` / `feet` / `ffm`,
 * lessPreferred), saved on the candidate.
 *
 * @param {{tags?: string[], title?: string}} topic
 * @param {string} text
 * @returns {{ok: false, reason: 'excluded'} | {ok: true, monster: boolean, feet: boolean, ffm: boolean}}
 */
export function judgeFirstPost({ tags = [], title = '' }, text) {
  if (hasExcludedText(text)) return { ok: false, reason: 'excluded' };
  return { ok: true, ...lessPreferred({ tags, title, text }) };
}

/** Discourse category of Free Scripts (Paid Scripts posts link to a store instead of files). */
export const FREE_SCRIPTS_CATEGORY = 14;

/**
 * Screen a post the user picked by id (`--add`): the exclusions, the
 * content rules (also on the first post, judgeFirstPost), posts the library
 * already links or the user turned down, and scenes the library already has
 * count. The live-not-POV rule doesn't apply.
 */
export function screenPick(topic, ctx) {
  const s = screenTopic(topic, { ...ctx, picked: true });
  return s.ok ? { ...s, picked: true } : s;
}

/** Archives the scout unpacks after downloading (unar, else bsdtar). */
export const ARCHIVE_RE = /\.(rar|zip|7z)$/i;
export const isArchiveName = (name) => ARCHIVE_RE.test(String(name || ''));

/**
 * A post that packs its scripts and videos in archives (RAR / ZIP / 7z on
 * pixeldrain, MEGA or the forum) as scenes: one per archive, `packed`, with
 * the archive as its `video` (so a fetched archive counts as done) and no
 * scripts or axes until it is unpacked (unpackedScenes).
 *
 * @param {{name: string, size?: number, host: string}[]} files the post's files
 */
export function archiveScenes(files) {
  const seen = new Set();
  return files.filter((f) => isArchiveName(f.name) && !seen.has(f.name) && seen.add(f.name))
    .map((f) => ({ base: f.name.replace(ARCHIVE_RE, ''), packed: true, axes: [], scripts: [], video: f, match: 0 }));
}

/**
 * The scenes in an unpacked archive (`files`: every file in it, `{name,
 * path, size}`), planned like a post's files (planScenes): scripts paired
 * with videos, axes from their names.
 *
 * @param {{name: string, path: string, size?: number}[]} files
 * @param {{vr?: boolean}} [o]
 */
export function unpackedScenes(files, { vr = false } = {}) {
  const local = files.filter((f) => !f.name.startsWith('._')).map((f) => ({ ...f, host: 'local' }));
  return planScenes({
    scripts: local.filter((f) => /\.funscript$/i.test(f.name)),
    videos: local.filter((f) => isVideoName(f.name)),
    vr,
  }).scenes;
}

/** Last action time of a parsed funscript in ms; 0 when it has none. */
export function scriptDurationMs(script) {
  const actions = Array.isArray(script?.actions) ? script.actions : [];
  return actions.reduce((max, a) => (Number.isFinite(a?.at) && a.at > max ? a.at : max), 0);
}

// ── Video variant ───────────────────────────────────────────────────────

// "K" of a file name (8K = 4320 lines, 4K = 2160); a height cap in the same unit.
const kOf = (height) => height / 540;

function resolutionOf(n) {
  if (/16k/.test(n)) return 16;
  if (/12k/.test(n)) return 12;
  if (/8k|4320/.test(n)) return 8;
  if (/6k|5k|2880/.test(n)) return 6;
  if (/4k|2160/.test(n)) return 4;
  if (/1440|2k/.test(n)) return 2.5;
  if (/1080/.test(n)) return 2;
  if (/720/.test(n)) return 1;
  // Only with the `p`: a bare 360 is a VR projection (`_360`).
  if (/(^|[^0-9])(480|540|576)p/.test(n)) return 0.75;
  if (/(^|[^0-9])(240|360)p/.test(n)) return 0.5;
  return 3;
}

/**
 * Preference among a scene's video files; the highest wins. VR: within
 * VR_MAX_HEIGHT and VR_MAX_FPS, low bitrate (heavier files stutter in the
 * headset). Flat: within FLAT_MAX_HEIGHT, and otherwise the highest quality
 * (more fps is better).
 */
export function videoPreference(name, size, vr) {
  const n = String(name).toLowerCase();
  const res = resolutionOf(n);
  const fps = Number((n.match(/(\d{2,3})\s*fps/) || [])[1] || 60);
  const isVr = /(^|[^a-z])(vr|sbs|180|fisheye|180x180|3dh|3dv)([^a-z]|$)/.test(n);
  let score = 0;
  if (vr) {
    score += (isVr ? 100 : 0) + (res <= kOf(VR_MAX_HEIGHT) ? res * 5 : -50);
    if (fps > VR_MAX_FPS) score -= 40;
    if (/low[- _]?bitrate|lowbit/.test(n)) score += 8;
    if (/high[- _]?bitrate/.test(n)) score -= 8;
  } else {
    score += (isVr ? -30 : 0) + (res <= kOf(FLAT_MAX_HEIGHT) ? res * 5 : -20) + Math.min(fps, 240) / 60;
  }
  if (/preview|trailer|sample/.test(n)) score -= 200;
  return score + Math.min((size || 0) / 1e10, 1);
}

// ── Scene plan ──────────────────────────────────────────────────────────

/**
 * Pair a post's script sets with its video files.
 *
 * Scripts are grouped by base (bracket / punctuation variants of one name
 * merge; per axis the first file wins, so list forum uploads first). Sets
 * that are the same files under names that differ only in quality words
 * (`… 1080p60fps.funscript` / `… 4k120fps.funscript`, same size per axis)
 * become one scene, which takes the best of both sets' videos. Each group
 * with a main script takes its best-matching video, best matches first, and
 * each video (all its encodes) pairs with one group only. A post with a
 * single script set whose videos don't name it takes its best video.
 * Handy-only scripts are ignored.
 *
 * @param {object} o
 * @param {{name: string}[]} o.scripts
 * @param {{name: string, size?: number}[]} o.videos
 * @param {boolean} [o.vr]
 * @param {string[]} [o.libraryVideos] coreName of every library video
 * @returns {{scenes: {base, axes: string[], scripts: object[], video: object, match: number}[], inLibrary: number}}
 */
export function planScenes({ scripts, videos, vr = false, libraryVideos = [] }) {
  const groups = new Map();
  for (const f of scripts) {
    const p = scriptParts(f.name);
    if (!p || /(^|[^a-z])handy([^a-z]|$)/i.test(p.base)) continue;
    const key = coreName(p.base) || p.base;
    const g = groups.get(key) || { base: p.base, files: [] };
    if (!g.files.some((x) => x.axis === p.axis)) g.files.push({ ...f, axis: p.axis });
    groups.set(key, g);
  }
  const merged = [];
  const sizes = (g) => (g.files.every((f) => f.size > 0) ? g.files.map((f) => `${f.axis}:${f.size}`).sort().join('|') : null);
  for (const g of groups.values()) {
    if (!g.files.some((f) => f.axis === '')) continue;
    const same = merged.find((m) => sizes(m) !== null && sizes(m) === sizes(g) && titlesMatch(titleWords(m.base), titleWords(g.base)));
    if (same) same.bases.push(g.base);
    else merged.push({ ...g, bases: [g.base] });
  }
  const withMain = merged;

  const options = new Map();
  for (const g of withMain) {
    for (const v of videos) {
      const m = Math.max(...g.bases.map((b) => matchScore(b, v.name)));
      if (m > 0) options.set(g, [...(options.get(g) || []), { v, m }]);
    }
  }
  if (withMain.length === 1 && options.size === 0 && videos.length) {
    options.set(withMain[0], videos.map((v) => ({ v, m: 0.5 })));
  }

  const best = (g) => Math.max(...options.get(g).map((o) => o.m));
  const used = new Set();
  const scenes = [];
  let inLibrary = 0;
  for (const g of [...options.keys()].sort((a, b) => best(b) - best(a))) {
    const open = options.get(g).filter((o) => !used.has(coreName(o.v.name)));
    if (!open.length) continue;
    const top = Math.max(...open.map((o) => o.m));
    const video = open.filter((o) => o.m === top).map((o) => o.v)
      .sort((a, b) => videoPreference(b.name, b.size, vr) - videoPreference(a.name, a.size, vr))[0];
    used.add(coreName(video.name));
    if (libraryVideos.some((l) => sameVideo(coreName(g.base), l) || sameVideo(coreName(video.name), l))) { inLibrary++; continue; }
    const axes = [...new Set(g.files.map((f) => f.axis).filter(Boolean))].sort();
    scenes.push({ base: g.base, axes, scripts: g.files, video, match: top });
  }
  return { scenes, inLibrary };
}

/** Hosts whose "files" are a streaming site's video page in one quality (scripts/stream-hosts.mjs, scripts/vrbangers-host.mjs). */
export const STREAM_HOSTS = new Set([RULE34VIDEO_HOST, VRBANGERS_HOST]);

/**
 * planScenes, with each scene paired to a streaming page's video checked by
 * length: the main script (`scriptMs(file)`, its last action in ms; null
 * when it can't be read) must fit the page's `duration` (durationMatches:
 * ≤ video + 2 s and ≥ 80 % of it). A page that fails, a teaser of a full
 * scene say, is dropped with all its qualities, and the post is planned
 * again without it, so the script can pair with another video. Unknown
 * lengths pass (the download is checked again with ffprobe).
 *
 * Then pages no scene took pair by length with script sets no video took
 * (page titles rarely name the script: `Yuffie's Interrogation Techniques`
 * for `FF - Yuffie Kisaragi (Hardcore)`): each page, in link order, takes the
 * set whose main script fits it, the one with the most axes when several do
 * (soft and hardcore versions of one video).
 *
 * @param {Parameters<typeof planScenes>[0] & {scriptMs: (file: object) => Promise<number|null>}} o
 * @returns {Promise<ReturnType<typeof planScenes> & {dropped: {url: string, durationSec: number, scriptMs: number}[]}>}
 */
export async function planWithStreams({ scripts, videos, vr = false, libraryVideos = [], scriptMs }) {
  let pool = videos;
  const dropped = [];
  let plan;
  for (;;) {
    plan = planScenes({ scripts, videos: pool, vr, libraryVideos });
    const bad = new Set();
    for (const sc of plan.scenes) {
      if (!STREAM_HOSTS.has(sc.video.host) || !(sc.video.duration > 0)) continue;
      const main = sc.scripts.find((f) => f.axis === '');
      const ms = main ? await scriptMs(main) : null;
      if (ms == null || durationMatches(sc.video.duration, ms)) continue;
      bad.add(sc.video.url);
      dropped.push({ url: sc.video.url, durationSec: sc.video.duration, scriptMs: ms });
    }
    if (!bad.size) break;
    pool = pool.filter((v) => !(STREAM_HOSTS.has(v.host) && bad.has(v.url)));
  }

  const usedPages = new Set(plan.scenes.filter((sc) => STREAM_HOSTS.has(sc.video.host)).map((sc) => sc.video.url));
  const pages = new Map();
  for (const v of videos) {
    if (STREAM_HOSTS.has(v.host) && v.duration > 0 && !usedPages.has(v.url)) pages.set(v.url, [...(pages.get(v.url) || []), v]);
  }
  if (!pages.size) return { ...plan, dropped };
  const keyOf = (base) => coreName(base) || base;
  const paired = new Set(plan.scenes.map((sc) => keyOf(sc.base)));
  const sets = new Map();
  for (const f of scripts) {
    const p = scriptParts(f.name);
    if (!p || /(^|[^a-z])handy([^a-z]|$)/i.test(p.base) || paired.has(keyOf(p.base))) continue;
    const set = sets.get(keyOf(p.base)) || [];
    sets.set(keyOf(p.base), [...set, f]);
  }
  const open = [];
  for (const files of sets.values()) {
    const main = files.find((f) => scriptParts(f.name).axis === '');
    const ms = main ? await scriptMs(main) : null;
    if (ms != null) open.push({ files, ms, axes: new Set(files.map((f) => scriptParts(f.name).axis).filter(Boolean)).size });
  }
  const scenes = [...plan.scenes];
  let { inLibrary } = plan;
  for (const variants of pages.values()) {
    const fits = open.filter((s) => durationMatches(variants[0].duration, s.ms)).sort((a, b) => b.axes - a.axes);
    if (!fits.length) continue;
    open.splice(open.indexOf(fits[0]), 1);
    const sub = planScenes({ scripts: fits[0].files, videos: variants, vr, libraryVideos });
    scenes.push(...sub.scenes);
    inLibrary += sub.inLibrary;
  }
  return { scenes, inLibrary, dropped };
}

/**
 * True when a MEGA link's key has the length MEGA uses: 16 bytes for a
 * folder, 32 for a file (`#<key>` or `!<key>` after the id). A mistyped key
 * (a character too many, say) makes megajs throw outside any promise, which
 * would end the run, so such a link is skipped as unreadable instead.
 */
export function megaKeyOk(url) {
  const m = String(url || '').match(/mega\.(?:nz|io)\/(?:(folder|file)\/[^#?]+#|#(F?)![^!]+!)([A-Za-z0-9_-]+)/);
  if (!m) return false;
  const folder = m[1] === 'folder' || m[2] === 'F';
  const key = m[3];
  return key.length % 4 !== 1 && Math.floor((key.length * 3) / 4) === (folder ? 16 : 32);
}

// ── Folders ─────────────────────────────────────────────────────────────

const safeName = (s) => String(s).replace(/[/\\:*?"<>|]/g, '').replace(/\s+/g, ' ').trim();
// Leading title groups that label the post rather than name its artist,
// resolution and frame-rate tags (`[4K]`, `(1080p)`, `[4K60]`, `[60fps]`) and
// series or serial codes (`【S022】`, `[PMV-64]`, `[EP 03]`) included.
const NOT_A_CREATOR = /multi|axis|(^|[^a-z])vr([^a-z]|$)|initiative|free|sr6|osr|handy|commission|request|suggest|vam|mmd|sfm|fap ?hero|cock ?hero|[hp]mv|r-?18|hentai|^[23]d$|^ai$|^iwara$|^(?:[1-9]k|\d{3,4}p|u?hd|fhd)(?:\s*\d{2,3}(?:\s*fps)?)?$|^\d{2,3}\s*fps$|^[a-z]{1,2}[- ]?\d{2,4}$/i;

/** True for a name that labels a script, genre or series (`Cock Hero`, `PMV`, `Multi-axis`, `S022`) rather than naming an artist. */
export const isCreatorLabel = (name) => NOT_A_CREATOR.test(String(name || ''));
// `[…]`, `（…）`, `【…】`, `{…}`, and `(…)` unless glued to a word (`(G)I-DLE`).
const GROUP_RE = /\[([^\]]*)\]|（([^）]*)）|【([^】]*)】|\{([^}]*)\}|\(([^()]*)\)(?![\p{L}\p{N}])/gu;
// Bracket parts that describe the script or the post, not the video.
const NOISE_PART = /request|suggest|commission|script|organic|waveform|^(sr6|osr\d*|m-?a|updated|source)$/i;
const AXIS_WORDS = /(?:multi|\d+)[- ]?axis|\baxis\b/giu;
// HereSphere / DeoVR projection tokens at the end of a video name (`_180_3DH`,
// `_LRF_Full_SBS`). The headset and the flatten view read them from the file name.
const VR_TAIL = /(?:[_ .-](?:180|360|180x180|360x180|3dh|3dv|lrf?|rl|tbf?|bt|ouf?|sbsf?|full|mono180|mono360|mkx2[02]0|rf52|fisheye\d*|vrca220|fb360|eac360|eac))+$/i;
// Scene stems stay well under the 255-byte name limit with `.generated.twist.funscript`.
const STEM_BYTES = 180;

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
// Letters in a name; a CJK character says as much as a short word, so it counts twice.
const letterCount = (s) => (coreName(s).match(/\p{L}/gu) || []).reduce((n, ch) => n + (/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(ch) ? 2 : 1), 0);
const squash = (s) => normName(s).replace(/ /g, '');

/** Separators a file name can't hold become ` - `; runs of dashes and stray punctuation go. */
function tidy(s) {
  return String(s)
    .replace(/\s+_\s+|\s+\/\s+|:\s+|\s*｜\s*/g, ' - ')
    .replace(/\//g, '-')
    .replace(/_+/g, ' ')
    .replace(/\s*-{2,}\s*/g, ' - ')
    .replace(/\s+/g, ' ')
    .replace(/\s+([,，、])/g, '$1')
    .replace(/^[\s\-–—,;.]+|[\s\-–—,;.]+$/g, '');
}

/** A bracket part worth keeping as metadata, or ''. */
function metaPart(s) {
  if (NOISE_PART.test(s.trim())) return '';
  const t = tidy(s.replace(/\p{So}/gu, ' ').replace(AXIS_WORDS, ' ').replace(/\bfree\b/gi, ' '));
  if (/^vr$/i.test(t)) return 'VR';
  if (!t || NOISE_PART.test(t) || !coreName(t)) return '';
  // Iwara video id: `3rUYW85MLIfCsu`, `QsmFYIhCwVnjBp`.
  if (/^[A-Za-z0-9]{12,}$/.test(t) && (/\d/.test(t) || (t.match(/[a-z][A-Z]/g) || []).length >= 3)) return '';
  return t;
}

/** Bracket groups (leading ones flagged) and the text around them. */
function splitGroups(text) {
  const groups = [];
  let rest = '';
  let last = 0;
  let lead = true;
  for (const m of text.matchAll(GROUP_RE)) {
    const between = text.slice(last, m.index);
    if (between.trim()) lead = false;
    // Keep words on both sides apart; `Name[tag]-2025` closes up to `Name-2025`.
    rest += between + (/^[\p{L}\p{N}]/u.test(text.slice(m.index + m[0].length)) ? ' ' : '');
    groups.push({ text: (m.slice(1).find((g) => g !== undefined) || '').trim(), lead });
    last = m.index + m[0].length;
  }
  return { groups, rest: rest + text.slice(last) };
}

/**
 * Artist, title core and metadata of a post title or a video name. The
 * artist is the first leading `[Name]` / `(Name)` that names someone, else a
 * trailing `by Name`, else `fallback`. Script labels (multi-axis, requested,
 * SR6, …) are dropped; other bracket parts and ` | ` segments become metadata.
 */
function parseName(text, fallback) {
  // A leading `(Name)` glued to the title (`(AlenAbyss)Midnight Ride`) is still the artist;
  // short ones stay glued so names like `(G)I-DLE` keep their brackets.
  const spaced = String(text).replace(/^(\s*\((?:[^()]*?\p{L}){3}[^()]*\))(?=[\p{L}\p{N}])/u, '$1 ');
  const { groups, rest } = splitGroups(spaced.replace(/^\s*(?:axis master|public release\s*:)\s*/i, ''));
  let artist = groups.find((g) => g.lead && g.text && !NOT_A_CREATOR.test(g.text))?.text || '';
  const [first, ...segments] = rest.split(/\s+\|\s+/);
  let core = first.replace(/^\s*iwara\s*-\s*/i, '').replace(AXIS_WORDS, ' ').replace(/\s+updated\s*$/i, '');
  if (!artist) {
    const by = core.match(/^(.*\S)\s+by\s+(\S+(?:\s+\S+){0,2})\s*$/i);
    if (by) [, core, artist] = by;
  }
  artist = tidy(artist || fallback || '');
  if (artist) {
    core = core.replace(new RegExp(`\\s+by\\s+${escapeRe(artist)}(?![\\p{L}\\p{N}])`, 'iu'), ' ')
      .replace(new RegExp(`^\\s*${escapeRe(artist)}(?![\\p{L}\\p{N}])\\s*[-–—]?`, 'iu'), ' ');
  }
  const meta = groups.filter((g) => g.text !== artist).flatMap((g) => g.text.split(/[,，、]/)).map(metaPart);
  for (const s of segments) {
    const m = metaPart(s);
    if (!m) break;
    meta.push(m);
  }
  return { artist, core: tidy(core), meta: meta.filter(Boolean) };
}

/** Cut to `max` UTF-8 bytes on a character boundary. */
function capBytes(s, max) {
  let out = '';
  let n = 0;
  for (const ch of s) {
    n += Buffer.byteLength(ch);
    if (n > max) break;
    out += ch;
  }
  return out.trim();
}

/** Creator folder: the post's artist (see parseName), else the poster; an alias as the artist it names (canonicalArtist). */
export function creatorFolder(title, author) {
  return safeName(canonicalArtist(parseName(title, author).artist)) || 'unknown';
}

/** The artist a title names itself (leading `[Name]` / `(Name)`, trailing `by Name`), or ''. */
export function titleArtist(title) {
  return parseName(title, '').artist;
}

/**
 * Scene folder and file stem, which the library shows as the title:
 * `Artist - Title (metadata)`, with an alias artist under its own name
 * (canonicalArtist). The title comes from the post; a post with
 * several videos (`shared`) names each by its own file instead, unless that
 * name says nothing (`7`, `03`), which then goes into the metadata. A VR
 * projection suffix of the video name (`_180_3DH`) is kept at the end.
 *
 * @param {object} o
 * @param {string} o.title post title
 * @param {string} o.author poster, the artist when the title names none
 * @param {string} o.video video file name
 * @param {boolean} [o.shared] the post has more than one video
 */
export function sceneTitle({ title, author, video, shared = false }) {
  const post = parseName(title, author);
  const base = String(video).replace(VIDEO_EXT_RE, '');
  const tail = (base.match(VR_TAIL) || [''])[0];
  const vrTail = /[a-z0-9]/i.test(tail.replace(/full/gi, '')) ? `_${tail.split(/[_ .-]/).filter(Boolean).join('_')}` : '';
  const own = parseName(vrTail ? base.slice(0, -tail.length) : base, post.artist);
  let { core, meta } = post;
  if (shared || letterCount(core) < 3) {
    const ownMeta = own.artist !== post.artist ? [own.artist, ...own.meta] : own.meta;
    if (letterCount(own.core) >= 3) {
      core = own.core;
      meta = ownMeta;
    } else meta = [tidy(own.core), ...ownMeta, ...meta];
  }
  const artist = canonicalArtist(post.artist);
  const artistKeys = new Set([squash(post.artist), squash(artist)]);
  const seen = new Set();
  meta = meta.filter((m) => {
    const k = squash(m);
    if (!k || artistKeys.has(k) || seen.has(k)) return false;
    seen.add(k);
    return true;
  });
  const named = (m) => safeName(`${artist} - ${core}${m.length ? ` (${m.join(', ')})` : ''}`);
  // Too long: drop metadata from the end, then cut the title.
  while (meta.length && Buffer.byteLength(named(meta)) > STEM_BYTES) meta = meta.slice(0, -1);
  return capBytes(named(meta), STEM_BYTES).replace(/[\s\-–—,;.(]+$/, '') + vrTail;
}

/**
 * Name of a scene whose video comes from vrbangers.com (the pseudo file of
 * vrbangers-host.mjs, or a sidecar's `vrbangers` record: `{performers, title,
 * slug, projection}`): folder = the first performer (canonicalArtist), stem = `<Performers> -
 * <Scene title> (VRBangers)` plus the projection suffix (`_180x180_3dh`), as
 * `Riley Reid - Anal Virginity (VRBangers)_180x180_3dh`. Bracket parts that
 * only name the encode (`(Remastered)`, `[8K]`) leave the title; performers
 * past the length cap are left out from the end.
 */
export function vrbangersSceneName({ performers = [], title = null, slug = '', projection = null }) {
  const people = (performers || []).map((p) => safeName(p)).filter(Boolean);
  const encodeOnly = (inner) => inner.split(/\s+/).filter(Boolean).every((w) => isNoiseWord(foldName(w).replace(/[^\p{L}\p{N}]/gu, '')));
  const core = safeName(String(title || String(slug).replace(/-/g, ' '))
    .replace(/\s*[([]([^()[\]]*)[)\]]/g, (all, inner) => (encodeOnly(inner) ? '' : all))) || safeName(slug);
  const tail = projection ? `_${projection}` : '';
  const named = (who) => `${who.length ? who.join(', ') : 'VRBangers'} - ${core} (VRBangers)`;
  let who = people;
  while (who.length > 1 && Buffer.byteLength(named(who)) > STEM_BYTES) who = who.slice(0, -1);
  return { creator: safeName(canonicalArtist(people[0] || '')) || 'VRBangers', stem: capBytes(named(who), STEM_BYTES).replace(/[\s\-–—,;.(]+$/, '') + tail };
}

/**
 * Folder and stem of a post's scene: vrbangersSceneName for a vrbangers.com
 * video, else the post's `<artist>/<Artist - Title (metadata)>` (creatorFolder, sceneTitle).
 */
export function sceneNameOf(c, sc) {
  if (sc.video?.host === VRBANGERS_HOST) return vrbangersSceneName(sc.video);
  return {
    creator: creatorFolder(c.title, c.author),
    stem: sceneTitle({ title: c.title, author: c.author, video: sc.video.name, shared: c.scenes.length > 1 }),
  };
}

/** File name for a scene's script in the scan's spelling. */
export function scriptFileName(stem, axis) {
  return `${stem}${axis ? `.${axis}` : ''}.funscript`;
}

/**
 * The `eroscripts.json` sidecar of a fetched scene (read by the library
 * scan and by --retitle): the post, the post's video file name and the
 * file on disk. A vrbangers.com scene keeps its site name (`vrbangers`,
 * read by vrbangersSceneName); a rule34video one gets the page's cover as
 * `posterUrl`, which the app makes the scene's thumbnail.
 *
 * @param {object} c candidate (post)
 * @param {object} video the scene's video file (`sc.video`)
 * @param {string} file the video's name on disk
 * @param {string} at ISO time
 */
export function sceneSidecar(c, video, file, at) {
  const posterUrl = video.host === RULE34VIDEO_HOST ? rule34videoPosterUrl(video.url) : null;
  return {
    topicId: c.id, url: c.url, title: c.title, author: c.author, tags: c.tags,
    video: video.name, file, scenes: c.scenes.length,
    ...(video.host === VRBANGERS_HOST ? {
      vrbangers: { slug: video.slug, url: video.url, title: video.title, performers: video.performers, projection: video.projection },
    } : {}),
    ...(posterUrl ? { posterUrl } : {}),
    at,
  };
}

// ── Retitle ─────────────────────────────────────────────────────────────

// exFAT and APFS compare names case- and normalization-insensitively.
const fold = (p) => p.normalize('NFC').toLowerCase();
const pathKey = (p) => fold(path.resolve(p));

/**
 * Moves that give fetched scenes their current `<creator>/<title>` names.
 * Each scene's files that carry the video's stem (video, scripts, axis
 * companions, subtitles) take the new stem; the sidecar keeps its name. A
 * name another scene or folder holds gets ` (2)`, ` (3)` …. Scenes already
 * named right are left out, so a second run plans nothing.
 *
 * @param {{dir: string, files: string[], rec: object}[]} scenes scene folder,
 *   its file names (no `._*`), and its sidecar record
 * @param {string} incoming the scout's download folder
 * @param {(dir: string) => boolean} [exists] whether a folder is already on disk
 * @param {(scene: object, video: string, shared: boolean) => {creator: string, stem: string}} [nameOf]
 *   the scene's name; default: the EroScripts post's (`eroscripts.json`)
 * @returns {{from: string, to: string, video: string, files: {from: string, to: string}[]}[]}
 */
export function retitlePlan(scenes, incoming, exists = () => false, nameOf = postSceneName) {
  const perTopic = new Map();
  for (const s of scenes) perTopic.set(s.rec.topicId, (perTopic.get(s.rec.topicId) || 0) + 1);
  const taken = new Set(scenes.map((s) => pathKey(s.dir)));
  const moves = [];
  for (const s of scenes) {
    const video = s.files.find((n) => n === s.rec.file) || s.files.find(isVideoName);
    if (!video) continue;
    const ext = path.extname(video);
    const oldStem = video.slice(0, -ext.length);
    const { creator, stem } = nameOf(s, video, (s.rec.scenes ?? perTopic.get(s.rec.topicId)) > 1);
    const parent = path.join(incoming, creator);
    taken.delete(pathKey(s.dir));
    const clash = (p) => taken.has(pathKey(p)) || (pathKey(p) !== pathKey(s.dir) && exists(p));
    let to = path.join(parent, stem);
    for (let n = 2; clash(to); n++) to = path.join(parent, `${stem} (${n})`);
    taken.add(pathKey(to));
    const newStem = path.basename(to);
    if (pathKey(to) === pathKey(s.dir) && newStem.normalize('NFC') === oldStem.normalize('NFC')) continue;
    const files = s.files.filter((n) => n.startsWith(`${oldStem}.`)).map((n) => ({ from: n, to: newStem + n.slice(oldStem.length) }));
    moves.push({ from: s.dir, to, video: newStem + ext, files });
  }
  return moves;
}

/** An EroScripts scene's name from its post (`eroscripts.json`); a vrbangers.com video keeps its site name. */
function postSceneName(s, video, shared) {
  const { title, author } = s.rec;
  if (s.rec.vrbangers) return vrbangersSceneName(s.rec.vrbangers);
  return { creator: creatorFolder(title, author), stem: sceneTitle({ title, author, video: s.rec.video || video, shared }) };
}

/**
 * Rewrite paths inside parsed JSON (keys and string values) after
 * `retitlePlan` moves: renamed files map to their new path, anything else
 * under a moved folder follows the folder. Paths match whatever their case
 * or Unicode form, as the volume does. Returns the new value and how many
 * keys and strings changed.
 */
export function remapPaths(value, moves) {
  const files = new Map();
  for (const m of moves) for (const f of m.files) files.set(fold(path.join(m.from, f.from)), path.join(m.to, f.to));
  const dirs = moves.map((m) => ({ key: fold(m.from) + path.sep, depth: m.from.split(path.sep).length, to: m.to }));
  const map = (s) => {
    if (!s.startsWith(path.sep)) return s;
    const k = fold(s);
    if (files.has(k)) return files.get(k);
    const d = dirs.find((x) => k.startsWith(x.key));
    return d ? [d.to, ...s.split(path.sep).slice(d.depth)].join(path.sep) : s;
  };
  let changed = 0;
  const walk = (v) => {
    if (typeof v === 'string') {
      const r = map(v);
      if (r !== v) changed++;
      return r;
    }
    if (Array.isArray(v)) return v.map(walk);
    if (!v || typeof v !== 'object') return v;
    const out = {};
    for (const [k, x] of Object.entries(v)) {
      const nk = map(k);
      if (nk !== k) changed++;
      out[nk] = walk(x);
    }
    return out;
  };
  return { value: walk(value), changed };
}

// ── Scan state ──────────────────────────────────────────────────────────
//
// A scan only judges posts it hasn't judged before. `<userData>/
// eroscripts-scout-state.json` keeps every judged post id with its verdict,
// per feed how deep the backfill has read (page, and `reached`: the oldest
// last activity listed so far), and the posts listed but not judged yet (a
// run stopped by a rate limit goes on with them next time):
//
//   { version, lastScanAt, feeds: {<tag>: {backfillPage, exhausted, reached?, doneSince?, olderFrom?}},
//     topics: {<id>: {verdict, rules, at}}, pending: {<id>: <listed topic>} }
//
// Feeds are newest activity first. A scan reads pages from the top until a
// page holds nothing to judge (or the page cap), then `backfill` more pages
// from where the backfill stopped last time, so older posts get covered a
// little on every run; `since` stops a feed's backfill at a date for good.
// Verdicts made under older rules count as unseen, and a `no-video` post
// with activity after its verdict is judged again.

/** Bump when the screening rules change, so every post is judged again (12: vrbangers.com links count as videos; 13: no sex rule). */
export const RULES_VERSION = 13;

export function emptyScoutState() {
  return { version: 1, lastScanAt: null, feeds: {}, topics: {}, pending: {} };
}

/** The state read from disk, or an empty one when it is missing or foreign. */
export function normalizeScoutState(raw) {
  if (!raw || raw.version !== 1 || typeof raw.topics !== 'object' || typeof raw.feeds !== 'object') return emptyScoutState();
  return { ...emptyScoutState(), ...raw };
}

/** True when post `id` was judged under the current rules (`rules`: another scout's version). */
export function seenTopic(state, id, rules = RULES_VERSION) {
  return state.topics[id]?.rules === rules;
}

/** Verdicts scripters often fix later (adding a video): judged again after new activity. */
export const REJUDGE_ON_BUMP = new Set(['no-video']);

/**
 * True when a listed post needs a verdict: never judged under these rules,
 * or a REJUDGE_ON_BUMP verdict older than the post's last activity
 * (`topic.bumped`; posts without it are judged once).
 */
export function needsJudging(state, topic, rules = RULES_VERSION) {
  const seen = state.topics[topic.id];
  if (seen?.rules !== rules) return true;
  return REJUDGE_ON_BUMP.has(seen.verdict) && !!topic.bumped && Date.parse(topic.bumped) > Date.parse(seen.at);
}

/** What a verdict keeps about its post, for `npm run scout:report`; kept from the earlier verdict when not given. */
export const POST_INFO_KEYS = ['title', 'author', 'created', 'bumped', 'image', 'images'];

/**
 * Remember the verdict on post `id` (`candidate` or a reject reason); it's no longer pending.
 * `info` (postInfo) is stored with it: title, author, dates and images, each kept from the
 * earlier verdict when `info` lacks it.
 */
export function markTopic(state, id, verdict, now = Date.now(), rules = RULES_VERSION, info = null) {
  const prev = state.topics[id] || {};
  const rec = { verdict, rules, at: new Date(now).toISOString() };
  for (const k of POST_INFO_KEYS) {
    const v = info?.[k] || prev[k];
    if (v) rec[k] = v;
  }
  if (prev.gone && !info?.title) rec.gone = true;
  state.topics[id] = rec;
  if (state.pending) delete state.pending[id];
}

/**
 * A post's title, author (the first post's), creation and last-activity dates and
 * image (absolute URL), from what a feed lists (`t`) and its topic payload (`data`,
 * api.getTopicData), whichever has each; `images` (up to 5 absolute URLs, the topic
 * image first: postImages in electron/eroscripts-api.js) only from the payload, `[]`
 * when it has none worth showing, null without a payload.
 */
export function postInfo(t, data = null) {
  const image = t?.image || data?.image_url || null;
  return {
    title: t?.title || data?.title || null,
    author: data?.post_stream?.posts?.[0]?.username || t?.author || null,
    created: t?.created || data?.created_at || null,
    bumped: t?.bumped || data?.bumped_at || null,
    image: image && !/^https?:\/\//.test(image) ? `https://discuss.eroscripts.com${image}` : image,
    images: data ? postImages(data) : null,
  };
}

/**
 * Judged posts `--titles` reads again: those without a title (judged before verdicts kept
 * one) or without `images` (judged before verdicts kept them); posts found gone are left
 * out. The `first` ids (the waiting candidates, in their order) come first, the rest newest first.
 *
 * @param {object} state scan state
 * @param {number[]} [first]
 */
export function topicsMissingInfo(state, first = []) {
  const missing = (t) => t && !t.gone && (!t.title || !Array.isArray(t.images));
  const lead = [...new Set(first)].filter((id) => missing(state.topics[id]));
  const leadSet = new Set(lead);
  const rest = Object.entries(state.topics)
    .filter(([id, t]) => missing(t) && !leadSet.has(Number(id)))
    .map(([id]) => Number(id))
    .sort((a, b) => b - a);
  return [...lead, ...rest];
}

/** Fill in a judged post's info from its topic payload, verdict and date unchanged; `null` data marks it gone (deleted or private). */
export function addPostInfo(state, id, data) {
  const rec = state.topics[id];
  if (!rec) return;
  if (!data) { rec.gone = true; return; }
  for (const [k, v] of Object.entries(postInfo(null, data))) if (v) rec[k] = v;
  delete rec.gone;
}

/** Keep listed posts until they get a verdict, so a run that stops early doesn't lose them. */
export function addPending(state, topics) {
  state.pending ||= {};
  for (const t of topics) state.pending[t.id] = t;
}

/** Posts listed (now or by a run that stopped early) that still need a verdict, newest first; drops the rest from `pending`. */
export function pendingTopics(state, rules = RULES_VERSION) {
  const out = [];
  for (const [id, t] of Object.entries(state.pending || {})) {
    if (needsJudging(state, t, rules)) out.push(t);
    else delete state.pending[id];
  }
  return out.sort((a, b) => Number(b.id) - Number(a.id));
}

/** Backfill cursor of a feed, created on first use. */
export function feedState(state, tag) {
  state.feeds[tag] ||= { backfillPage: 0, exhausted: false };
  return state.feeds[tag];
}

/** Every feed `--feeds` takes: the tag feeds (FEED_TAGS), then the search feeds (SEARCH_FEEDS). */
export const ALL_FEEDS = [...FEED_TAGS, ...Object.keys(SEARCH_FEEDS)];

/** A feed's backfill progress in words: `done` (whole feed read), `done to 2025-01-01 (p65)`, or `p80, back to 2026-08-09` (oldest last activity listed). */
export function feedProgress(feed) {
  if (feed?.exhausted) return 'done';
  if (feed?.doneSince) return `done to ${feed.doneSince} (p${feed.backfillPage})`;
  return `p${feed?.backfillPage ?? 0}${feed?.reached ? `, back to ${feed.reached.slice(0, 10)}` : ''}`;
}

/** Posts with one of `verdicts` judged under older rules and not already pending, newest first (`--rejudge`). */
export function staleTopicIds(state, verdicts, rules = RULES_VERSION) {
  const want = new Set(verdicts);
  return Object.entries(state.topics)
    .filter(([id, t]) => want.has(t.verdict) && t.rules !== rules && !state.pending?.[id])
    .map(([id]) => Number(id))
    .sort((a, b) => b - a);
}

/** A post read by id (`api.getTopicData`) in the shape a feed lists it: `{id, title, tags, likes, created, bumped, image}`. */
export function listedTopic(id, data) {
  return {
    id,
    title: data.title || '',
    tags: (data.tags || []).map((x) => (typeof x === 'string' ? x : x?.name || '')).filter(Boolean),
    likes: data.like_count || 0,
    created: data.created_at || null,
    bumped: data.bumped_at || null,
    image: data.image_url || null,
  };
}

/**
 * True when `--fetch --auto` takes a waiting post: one the user swiped right on in `npm run scout:report`
 * (`wanted`, readSwipes), whatever its score or axes, or hand-made multi-axis (archives count) scoring at
 * least AUTO_FETCH_MIN_SCORE.
 *
 * @param {object} c saved candidate
 * @param {object} [scoring] see candidateScore
 * @param {Set<number>} [wanted] post ids swiped right
 */
export function autoFetchPick(c, scoring = {}, wanted = new Set()) {
  return wanted.has(c.id) || (c.axisType === 'multi' && candidateScore(c, scoring) >= AUTO_FETCH_MIN_SCORE);
}

/** The swipe deck's file in userData: written by `npm run scout:report` only, read here (readSwipes). */
export const SWIPES_FILE = 'eroscripts-scout-swipes.json';

/**
 * The posts swiped in `npm run scout:report`'s deck, from SWIPES_FILE parsed
 * (`{version: 1, swipes: {"<id>": {dir: "right" | "left", at}}}`): `wanted`
 * (right: `--fetch --auto` takes them) and `hidden` (left: only out of the
 * deck; the scout takes no notice). An undone swipe has no entry. Anything
 * else in the file is ignored.
 *
 * @param {object|null} json
 * @returns {{wanted: Set<number>, hidden: Set<number>}}
 */
export function readSwipes(json) {
  const wanted = new Set();
  const hidden = new Set();
  const swipes = json?.swipes;
  if (swipes && typeof swipes === 'object') {
    for (const [key, s] of Object.entries(swipes)) {
      const id = Number(key);
      if (!Number.isSafeInteger(id) || id <= 0) continue;
      if (s?.dir === 'right') wanted.add(id);
      else if (s?.dir === 'left') hidden.add(id);
    }
  }
  return { wanted, hidden };
}

/**
 * A candidate scored under `scoring` (candidateScore), with what the scout report's deck shows of it:
 * `tagScores` (its tags that score, postTagScores) and `creampie` (in the Creampie group of
 * TAG_SCORE_GROUPS, as the score counts it).
 */
export function scoredCandidate(c, scoring = {}) {
  return { ...c, score: candidateScore(c, scoring), tagScores: postTagScores(c.tags, scoring.tagScores), creampie: inTagGroup(c, 'Creampie') };
}

/**
 * `--status`: what the scan state says, as lines: last scan and daily run,
 * each feed's backfill progress, verdicts (and how many predate the current
 * rules), posts left pending, and the waiting list with what `--auto` takes.
 *
 * @param {object} state scan state
 * @param {{candidates?: object[], scoring?: object, wanted?: Set<number>, daily?: {scanAt, scanExit, fetchAt?, fetchExit?}|null, now?: number, rules?: number}} [o]
 */
export function scoutStatus(state, { candidates = [], scoring = {}, wanted = new Set(), daily = null, now = Date.now(), rules = RULES_VERSION } = {}) {
  const when = (iso) => {
    if (!iso) return 'never';
    const h = Math.round((now - Date.parse(iso)) / 3_600_000);
    return `${iso.slice(0, 16).replace('T', ' ')} UTC (${h < 48 ? `${h} h` : `${Math.round(h / 24)} days`} ago)`;
  };
  const count = (list) => Object.entries(list.reduce((m, t) => ({ ...m, [t.verdict]: (m[t.verdict] || 0) + 1 }), {}))
    .sort((a, b) => b[1] - a[1]).map(([v, n]) => `${v} ${n}`).join(' · ');
  const lines = [`Last scan: ${when(state.lastScanAt)}`];
  if (daily) lines.push(`Daily run: scan ${when(daily.scanAt)}, exit ${daily.scanExit}${daily.fetchAt ? `; auto-fetch ${when(daily.fetchAt)}, exit ${daily.fetchExit}` : ''}`);
  lines.push('Backfill:');
  for (const tag of new Set([...FEED_TAGS, ...Object.keys(state.feeds)])) {
    lines.push(`  ${tag.padEnd(12)} ${feedProgress(state.feeds[tag])}${ALL_FEEDS.includes(tag) ? '' : ' (no longer a feed)'}`);
  }
  const all = Object.values(state.topics);
  const stale = all.filter((t) => t.rules !== rules);
  lines.push(`Judged: ${all.length} posts · ${count(all)}`);
  if (stale.length) lines.push(`  ${stale.length} judged under rules older than v${rules} (judged again when a feed lists them; --rejudge <verdicts> for the rest): ${count(stale)}`);
  const pending = Object.keys(state.pending || {}).length;
  if (pending) lines.push(`Listed, not judged yet: ${pending} (the next run goes on with them)`);
  const multi = candidates.filter((c) => c.axisType === 'multi').length;
  const auto = candidates.filter((c) => autoFetchPick(c, scoring, wanted)).length;
  lines.push(`Waiting to download: ${candidates.length} posts (${multi} multi-axis, ${candidates.length - multi} single-axis); --auto takes ${auto} (multi-axis, score ≥ ${AUTO_FETCH_MIN_SCORE}, or swiped right)`);
  return lines;
}

/**
 * listFeed's `listPage` for the forum: a tag feed reads its Free Scripts tag
 * (`api.listFreeTagTopics`), a search feed (SEARCH_FEEDS) a search of Free
 * Scripts first posts for its words (`api.listFreeSearchTopics`). Every
 * request waits for `pacer` (the run's forum pacer); searches also for
 * `searchPacer` (≥ SEARCH_INTERVAL_MS apart: Discourse limits searches per
 * minute). A rate limit or expired session becomes RateLimitedError /
 * SessionExpiredError, any other failure an Error.
 */
export function feedPage(api, { pacer, searchPacer = createPacer({ intervalMs: SEARCH_INTERVAL_MS }) }) {
  return async (feed, page) => {
    const words = Object.hasOwn(SEARCH_FEEDS, feed) ? SEARCH_FEEDS[feed] : null;
    if (words !== null) await searchPacer.wait();
    await pacer.wait();
    const r = words !== null ? await api.listFreeSearchTopics(words, page) : await api.listFreeTagTopics(feed, page);
    if (r.rateLimited) throw new RateLimitedError();
    if (r.authExpired) throw new SessionExpiredError();
    if (r.error) throw new Error(r.error);
    return r;
  };
}

/** True when a candidate has a scene whose video comes from vrbangers.com (`npm run vrbangers:scout` shows and fetches only those). */
export function isVrbangersCandidate(c) {
  return (c.scenes || []).some((sc) => sc.video?.host === VRBANGERS_HOST);
}

/**
 * A saved candidate with its favourite under the current favourites
 * (currentFavourite over what the scan matched: title, tags, author); as
 * it was when `favourites` isn't given.
 *
 * @param {object} c
 * @param {string[]} [favourites]
 */
export function withCurrentFavourite(c, favourites) {
  if (!Array.isArray(favourites)) return c;
  return { ...c, favourite: currentFavourite(c, [c.title, ...(c.tags || []), c.author], favourites) };
}

/**
 * The saved candidate list after a scan: candidates found now, plus earlier
 * ones that still have a scene to download, minus posts the user turned
 * down and posts that break the content rules (contentRuleBreak; lists
 * saved under older rules may hold some, picks too). Best score first, each
 * scored again (scoredCandidate) under the current RANKING, the user's tag
 * scores and the favourite artist bonus, with its favourite under the
 * current favourites.
 *
 * @param {object[]} previous candidates from the last list
 * @param {object[]} fresh candidates found by this scan
 * @param {(topicId: number, videoName: string) => boolean} isDone scene fetched or rejected
 * @param {Set<number>} [turnedDown] posts the user rejected
 * @param {{tagScores?: Object<string, number>, favouriteScore?: number}} [scoring] see candidateScore
 * @param {string[]} [favourites] the current favourite artists (scout-live.mjs scoutPreferences)
 */
export function mergeCandidates(previous, fresh, isDone, turnedDown = new Set(), scoring = {}, favourites = undefined) {
  const byId = new Map();
  for (const c of [...(previous || []), ...(fresh || [])]) byId.set(c.id, c);
  return [...byId.values()]
    .map((c) => withCurrentFavourite(c, favourites))
    .filter((c) => !turnedDown.has(c.id) && !contentRuleBreak(c) && c.scenes.some((sc) => !isDone(c.id, sc.video.name)))
    .map((c) => scoredCandidate(c, scoring))
    .sort((a, b) => b.score - a.score);
}

/**
 * One feed's posts for this scan: pages from the top until one holds
 * nothing to judge (at most `maxNew` pages), then `backfill` pages from the
 * feed's backfill cursor, which moves on. `full` reads all `maxNew` pages.
 * With `since` (YYYY-MM-DD) the backfill skips posts last active before it
 * and, at the first page older than it, records `doneSince` and puts the
 * cursor back on the first page that held an older post (so an earlier date
 * later reads those skipped posts); a feed done to a date isn't backfilled
 * again unless `since` is earlier. `keepPending`
 * adds the posts to judge to `state.pending` as each page is read, so a run
 * stopped mid-listing (rate limit) doesn't lose the pages the cursor passed.
 *
 * @param {object} state scan state (mutated: the feed's cursor, `pending`)
 * @param {string} tag
 * @param {(tag: string, page: number) => Promise<{topics: {id: number, bumped?: string}[], more: boolean}>} listPage
 * @param {{maxNew: number, backfill: number, full?: boolean, rules?: number, since?: string, keepPending?: boolean}} o `rules`: the scout's rules version
 * @returns {Promise<{topics: object[], pagesRead: number}>}
 */
export async function listFeed(state, tag, listPage, { maxNew, backfill, full = false, rules = RULES_VERSION, since = null, keepPending = false }) {
  const feed = feedState(state, tag);
  const topics = [];
  let pagesRead = 0;
  const read = async (page) => {
    const r = await listPage(tag, page);
    pagesRead++;
    return r;
  };
  const keep = (list) => {
    topics.push(...list);
    for (const t of list) if (t.bumped && !(feed.reached && Date.parse(t.bumped) >= Date.parse(feed.reached))) feed.reached = t.bumped;
    if (keepPending) addPending(state, full ? list : list.filter((t) => needsJudging(state, t, rules)));
  };
  let page = 0;
  while (page < maxNew) {
    const r = await read(page);
    keep(r.topics);
    page++;
    if (!r.more) { feed.exhausted = true; break; }
    if (!full && !r.topics.some((t) => needsJudging(state, t, rules))) break;
  }
  feed.backfillPage = Math.max(feed.backfillPage, page);
  const sinceMs = since ? Date.parse(since) : null;
  const older = (t) => !!t.bumped && Date.parse(t.bumped) < sinceMs;
  const reachedDate = () => !!feed.doneSince && (sinceMs === null || Date.parse(feed.doneSince) <= sinceMs);
  for (let i = 0; i < backfill && !feed.exhausted && !reachedDate(); i++) {
    const pageNo = feed.backfillPage;
    const r = await read(pageNo);
    feed.backfillPage++;
    if (!r.more) feed.exhausted = true;
    if (sinceMs === null) { keep(r.topics); continue; }
    keep(r.topics.filter((t) => !older(t)));
    if (r.topics.some(older) && feed.olderFrom?.since !== since) feed.olderFrom = { page: pageNo, since };
    if (r.topics.length && r.topics.every(older)) {
      feed.doneSince = since;
      feed.backfillPage = feed.olderFrom.page;
      delete feed.olderFrom;
    }
  }
  return { topics, pagesRead };
}

// ── Queue & playlist ────────────────────────────────────────────────────

/**
 * Download order: multi-axis posts first, then single-axis; each by score
 * (candidateScore with the user's tag scores and favourite artist bonus, and
 * each post's favourite under the current favourites, so a list saved under
 * older weights or favourites follows the current ones). Posts a saved list
 * holds that break the content rules (contentRuleBreak) are dropped, picks
 * included.
 *
 * @param {object[]} candidates
 * @param {{tagScores?: Object<string, number>, favouriteScore?: number}} [scoring] see candidateScore
 * @param {string[]} [favourites] the current favourite artists
 */
export function downloadQueue(candidates, scoring = {}, favourites = undefined) {
  const ok = candidates
    .map((c) => withCurrentFavourite(c, favourites))
    .filter((c) => c.scenes.length && !contentRuleBreak(c));
  const byScore = (a, b) => candidateScore(b, scoring) - candidateScore(a, scoring);
  return [...ok.filter((c) => c.axisType === 'multi').sort(byScore), ...ok.filter((c) => c.axisType !== 'multi').sort(byScore)];
}

/**
 * The waiting list against the library as it is now, not as it was when the
 * posts were judged (days or weeks earlier): a post the user turned down, or
 * whose topic a library scene is linked to by another way (EroScripts sync,
 * the app's download, a link) is dropped. A scene whose video the library
 * holds now is dropped too, by the same check as at judging (planScenes);
 * archives can't be told before unpacking. Posts the scout fetched in part
 * (`ownTopics`, its sidecars) keep their other scenes: episodes of one post
 * look alike to sameVideo (`Ep1`, `Ep2`), and the fetch skips the fetched ones.
 *
 * @param {object[]} candidates
 * @param {{knownTopics?: Set<number>, ownTopics?: Set<number>, turnedDown?: Set<number>, libraryVideos?: string[]}} held
 * @returns {{queue: object[], posts: number, scenes: number}} the rest, and how many posts and scenes were dropped
 */
export function notYetHeld(candidates, { knownTopics = new Set(), ownTopics = new Set(), turnedDown = new Set(), libraryVideos = [] } = {}) {
  const queue = [];
  let posts = 0;
  let scenes = 0;
  const inLibrary = (sc) => !sc.packed && libraryVideos.some((l) => sameVideo(coreName(sc.base), l) || sameVideo(coreName(sc.video.name), l));
  for (const c of candidates) {
    if (turnedDown.has(c.id)) { posts++; continue; }
    if (ownTopics.has(c.id)) { queue.push(c); continue; }
    if (knownTopics.has(c.id)) { posts++; continue; }
    const open = c.scenes.filter((sc) => !inLibrary(sc));
    scenes += c.scenes.length - open.length;
    if (!open.length) { posts++; continue; }
    queue.push(open.length === c.scenes.length ? c : { ...c, scenes: open });
  }
  return { queue, posts, scenes };
}

/**
 * Make sure the scout's download folder (`{name: 'EroScripts'|'FapTap', path}`)
 * is a library source in a parsed config.json (mutated). True when added.
 */
export function addLibrarySource(config, folder, { newId }) {
  config.settings ||= {};
  config.settings.library ||= {};
  const lib = config.settings.library;
  lib.sources ||= [];
  if (lib.sources.some((s) => path.resolve(s.path) === path.resolve(folder.path))) return false;
  lib.sources.push({ id: newId(), name: folder.name, path: folder.path, enabled: true });
  return true;
}

/** Key of one fetched scene in the `eroscripts.json` sidecars: post id + chosen video file. */
export function fetchedKey(topicId, videoName) {
  return `${topicId}|${videoName}`;
}

// ── Downloads by hand ───────────────────────────────────────────────────
//
// Some hosts can't be fetched by a script: pixeldrain answers 403
// `file_rate_limited_captcha_required` once a file has hotlink protection,
// and only its own page (with a CAPTCHA) serves it. The user downloads those
// files in a browser; `--fetch` then takes them from the Downloads folder
// instead of the network (any host: a MEGA file saved by hand counts too).

/** Hosts whose files the user may have to download by hand. */
export const MANUAL_HOSTS = new Set(['pixeldrain']);

/** The page a person downloads `file` from: pixeldrain's file viewer, else the file's URL. */
export function manualPageUrl(file) {
  const id = String(file.url || '').match(/pixeldrain\.(?:com|net)\/api\/file\/([A-Za-z0-9]+)/)?.[1];
  return id ? `https://pixeldrain.com/u/${id}` : file.url;
}

/**
 * The posts of a download queue with files on MANUAL_HOSTS, in queue order:
 * `[{id, title, url, files: [{name, size, page}]}]`, every script and video
 * of the post's wanted scenes that sits on such a host. Scenes `isDone`
 * (already fetched or turned down) are left out.
 *
 * @param {object[]} queue downloadQueue's result
 * @param {(id: number, videoName: string) => boolean} [isDone]
 */
export function manualDownloads(queue, isDone = () => false) {
  const out = [];
  for (const c of queue) {
    const files = new Map();
    for (const sc of c.scenes || []) {
      if (isDone(c.id, sc.video.name)) continue;
      for (const f of [...(sc.scripts || []), sc.video]) {
        if (f && MANUAL_HOSTS.has(f.host) && !files.has(f.name)) files.set(f.name, { name: f.name, size: f.size || null, page: manualPageUrl(f) });
      }
    }
    if (files.size) out.push({ id: c.id, title: c.title, url: c.url, files: [...files.values()] });
  }
  return out;
}

/**
 * The text of the download list for the user (`download-for-fs.txt`):
 * instructions, then each post with its files' pages.
 *
 * @param {ReturnType<typeof manualDownloads>} posts
 * @param {{folder: string, generatedAt?: string}} o folder: where the files go
 */
export function manualListText(posts, { folder, generatedAt = new Date().toISOString() }) {
  const size = (n) => (n ? ` (${n >= 1e9 ? `${(n / 1e9).toFixed(2)} GB` : `${Math.max(0.01, n / 1e6).toFixed(2)} MB`})` : '');
  const total = posts.reduce((n, p) => n + p.files.reduce((m, f) => m + (f.size || 0), 0), 0);
  const lines = [
    'FunSync: files to download by hand',
    `Made ${generatedAt} by: npm run eroscripts:scout -- --manual-list`,
    '',
    'pixeldrain blocks scripted downloads of these files (hotlink protection), but its',
    'own page still serves them after a CAPTCHA.',
    '',
    'How:',
    '  1. Run `npm run manual:open` in the repo: it opens each post next to its download pages,',
    '     5 posts at a time (Enter for the next 5). Or open each link below by hand.',
    '     On each pixeldrain page press Download and solve the CAPTCHA.',
    `  2. Save the file into ${folder} (or leave it in ~/Downloads). Keep the name exactly`,
    '     as pixeldrain gives it: the scout matches files by name (and size).',
    '  3. Download every file of a post (its scripts and its video), or the post is skipped.',
    '     RAR / ZIP / 7z files are unpacked by the scout; download them as they are.',
    '  4. pixeldrain lets a free user download about 6 GB a day.',
    '  5. Then tell omp "downloaded", or run:',
    `       npm run eroscripts:scout -- --fetch --ids ${posts.map((p) => p.id).join(',') || '<ids>'}`,
    '     Each file is moved into the library (EroScripts folder), the scene is named as',
    '     usual, and the file is taken out of Downloads.',
    '',
    `${posts.length} posts, ${posts.reduce((n, p) => n + p.files.length, 0)} files${total ? `, about ${(total / 1e9).toFixed(1)} GB` : ''}`,
  ];
  posts.forEach((p, i) => {
    lines.push('', `${i + 1}. ${p.title}`, `   Post: ${p.url}  (id ${p.id})`);
    for (const f of p.files) lines.push(`   - ${f.name}${size(f.size)}`, `     ${f.page}`);
  });
  return `${lines.join('\n')}\n`;
}

/**
 * The copy of `file` the user saved by hand, from `entries` (`[{name, path,
 * size}]`, the Downloads folder's files): the same name (Unicode form
 * aside) and, when the list knows it, the same size. null when there's none.
 */
export function findLocalCopy(file, entries) {
  const want = String(file?.name || '').normalize('NFC');
  if (!want) return null;
  // A browser saves a second download of a name as `name (1).ext`.
  const plain = (n) => String(n).normalize('NFC').replace(/ \(\d+\)(\.[^.]+)$/, '$1');
  const sized = (e) => !file.size || e.size === file.size;
  const list = entries || [];
  return list.find((e) => String(e.name).normalize('NFC') === want && sized(e)) || list.find((e) => plain(e.name) === want && sized(e)) || null;
}
