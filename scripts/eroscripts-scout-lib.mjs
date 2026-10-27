// Discovery scout: find free EroScripts posts worth adding to the library,
// plan which scripts and which video file to download, and queue the
// finished scenes into the app's "Discovery" playlist.
//
// Logic for `npm run eroscripts:scout` (scripts/eroscripts-scout.mjs), kept
// apart from the CLI so it can be tested without a forum, MEGA or pixeldrain.
//
// Rules (the tags, words, lengths, caps and weights are in
// renderer/js/content-preferences.js; this module applies them)
//   • Free Scripts only (the CLI lists that category).
//   • Never excluded content (tags, title or first post). Scenes already in
//     the library stay.
//   • Never monsters nor feet (tags, title words or first post), favourites
//     and posts picked by id (`--add`) included.
//   • Animated: an animation tag and no `real` tag. A music post by a
//     creator already in the library is accepted as well. Live action by a
//     favourite (title, tags or poster; scripts/scout-live.mjs) is taken;
//     other live action waits for the live quota (both scouts).
//   • Sex required, picks included: a penetrative sex tag or sex word in the
//     title, else in the first post. A music video counts unless it's a
//     dance or its title is about a non-sex act. Never a post tagged sfw /
//     non-porn. Candidates keep where the sex showed (`sexual: tags | title
//     | text`). `--fetch` and every scan drop saved candidates that now
//     break these content rules (contentRuleBreak); `--recheck` reads the
//     first post of those saved before the evidence was kept.
//   • Videos shorter than MIN_VIDEO_MS (by the main script) aren't
//     downloaded; shorter scenes fetched under older rules stay.
//   • Multi-axis scenes only, except the single-axis cases `takesSingleAxis`
//     allows.
//   • Ranking: RANKING weights (VR, music, library creator, preferred acts,
//     multi-axis, likes; 2D hentai only ranks lower). There's no hentai
//     feed; hentai comes in through the other feeds.
//   • Skip posts the library already links and scenes whose video is
//     already in the library.
//   • Video variant: within the VR caps, low bitrate when offered; flat
//     within its cap. Previews and trailers never.
//   • Single-axis scenes get generated twist / roll / pitch companions after
//     download (scripts/upgrade-multi-axis.mjs); hand-made axes always win.

import path from 'node:path';
import { favouriteIn } from './scout-live.mjs';
import { titleWords, titlesMatch } from './scene-dedupe-lib.mjs';
import {
  ANIMATION_TAGS, DANCE_TITLE, EXCLUDED_TEXT, FEET_TAGS, FEET_TITLE, FLAT_HENTAI_TAGS, FLAT_MAX_HEIGHT, MIN_VIDEO_MS, MONSTER_TAGS,
  MONSTER_TITLE, MUSIC_TAGS, NON_SEX_ACT_TITLE, NON_SEX_PHRASES, PMV_TAGS, PMV_TITLE, RANKING, SEX_TAGS, SEX_WORDS, SEX_WORDS_CJK,
  SFW_TAGS, SINGLE_AXIS_CREAMPIE_TITLE, THREE_D_TAGS, THREE_D_TITLE, VR_MAX_FPS, VR_MAX_HEIGHT, WEREWOLF_TOKEN,
} from '../renderer/js/content-preferences.js';

/** The minimum as words for messages: `2 min`. */
export const MIN_VIDEO_LABEL = `${MIN_VIDEO_MS / 60_000} min`;

export const DISCOVERY_NAME = 'Discovery';

const VIDEO_EXT_RE = /\.(mp4|m4v|mkv|webm|mov)$/i;
const SCRIPT_AXES = new Set(['surge', 'sway', 'twist', 'roll', 'pitch', 'vib', 'lube', 'pump', 'suction', 'valve']);
// TCode channel suffixes some posts use instead of names (`Scene.R0.funscript`).
const TCODE_AXES = { L0: '', L1: 'surge', L2: 'sway', R0: 'twist', R1: 'roll', R2: 'pitch' };
// Words that only describe the encode, never the scene.
const QUALITY_WORD = /^(\d{3,4}p|\d+k|\d+k\d+fps|\d+fps|vr|sbs|low|high|bitrate|source|uncompressed|hq|lq|h264|h265|hevc|x264|x265|hd|uhd)$/;

// ── Names ───────────────────────────────────────────────────────────────

export const isVideoName = (name) => VIDEO_EXT_RE.test(String(name || ''));

/** Lower-case letters and digits of any script (CJK included), single-spaced. */
export function normName(name) {
  return String(name || '').toLowerCase().replace(VIDEO_EXT_RE, '').replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
}

/** normName without encode words (`1080p`, `8K60FPS`, `VR`, `low bitrate` …). */
export function coreName(name) {
  return normName(name).split(' ').filter((w) => w && !QUALITY_WORD.test(w)).join(' ');
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
  if (Math.min(a.length, b.length) >= 8 && (a.includes(b) || b.includes(a))) return true;
  return aWords.length >= 3 && aWords.filter((w) => bWords.has(w)).length / aWords.length >= 0.75;
}

// ── Screening ───────────────────────────────────────────────────────────

/** True when text names excluded content (whole words, any script). */
export function hasExcludedText(text) {
  return EXCLUDED_TEXT.test(String(text || ''));
}

/** True when a post's tags or title name a monster (tentacles, werewolves, horse guys, orcs …). */
export function hasMonster({ tags = [], title = '' }) {
  return tags.some((t) => MONSTER_TAGS.has(t)) || MONSTER_TITLE.test(title) || WEREWOLF_TOKEN.test(title);
}

/** True when a post's tags or title are about feet (footjob, soles, toes …). */
export function hasFeet({ tags = [], title = '' }) {
  return tags.some((t) => FEET_TAGS.has(t)) || FEET_TITLE.test(title);
}

/** `monster` or `feet` when a first post's text names one (the title words, whole words), else null. */
export function textRuleBreak(text) {
  const s = String(text || '');
  if (MONSTER_TITLE.test(s) || WEREWOLF_TOKEN.test(s)) return 'monster';
  if (FEET_TITLE.test(s)) return 'feet';
  return null;
}

/**
 * Why a post or video breaks the content rules every scout list obeys
 * (favourites and picks included), judged by its tags and title, so a
 * saved candidate can be checked again: `monster`, `feet`, `not-sexual`
 * (see isSexual), or null when it doesn't. A candidate whose first post
 * showed the sex (`sexual: 'text'`) passes the sex rule; one marked
 * `contentBreak` (by `--recheck`) breaks that rule for good.
 *
 * @param {{tags?: string[], title?: string, sexual?: string, contentBreak?: string}} post
 */
export function contentRuleBreak({ tags = [], title = '', sexual, contentBreak }) {
  if (contentBreak) return contentBreak;
  if (hasMonster({ tags, title })) return 'monster';
  if (hasFeet({ tags, title })) return 'feet';
  if (!isSexual({ tags, title }) && !(sexual === 'text' && !tags.some((t) => SFW_TAGS.has(t)))) return 'not-sexual';
  return null;
}

/**
 * First pass on a listed topic, before any request for its files. The sex
 * rule waits for the first post (judgeFirstPost), except for posts tagged
 * sfw / non-porn.
 *
 * @param {{id: number, title: string, tags: string[]}} topic
 * @param {object} ctx
 * @param {Set<number>} ctx.knownTopics posts the library already links
 * @param {string[]} ctx.creators creator names in the library (lower case)
 * @param {string[]} ctx.sceneNames normName of every library scene folder
 * @param {string[]} [ctx.favourites] favourite creators and actresses (scripts/scout-live.mjs)
 * @returns {{ok: false, reason: string} | {ok: true, music: boolean, vr: boolean, artist: string|null,
 *   live: boolean, favourite: string|null, liveQuota: boolean, hentaiOnly: boolean}}
 *   reasons: known, excluded (also monsters and feet), not-sexual, in-library
 */
export function screenTopic(topic, { knownTopics = new Set(), creators = [], sceneNames = [], favourites = [] } = {}) {
  const tags = topic.tags || [];
  if (knownTopics.has(topic.id)) return { ok: false, reason: 'known' };
  if (tags.some(hasExcludedText) || hasExcludedText(topic.title)) return { ok: false, reason: 'excluded' };
  if (hasMonster({ tags, title: topic.title }) || hasFeet({ tags, title: topic.title })) return { ok: false, reason: 'excluded' };
  if (tags.some((t) => SFW_TAGS.has(t))) return { ok: false, reason: 'not-sexual' };
  const title = normName(topic.title);
  const artist = creators.find((c) => c.length > 3 && title.includes(normName(c))) || null;
  const live = !(tags.some((t) => ANIMATION_TAGS.has(t)) && !tags.includes('real'));
  const music = tags.some((t) => MUSIC_TAGS.has(t));
  const favourite = favouriteIn([topic.title, ...tags], favourites);
  if (sceneNames.some((s) => s.length > 8 && (title.includes(s) || s.includes(title)))) return { ok: false, reason: 'in-library' };
  return {
    ok: true, music, vr: tags.includes('vr'), artist, live, favourite, liveQuota: liveOnQuota({ live, favourite, music, artist }),
    hentaiOnly: isHentaiOnly({ tags, title: topic.title }),
  };
}

/**
 * Second pass, on the first post's text (tags stripped): the exclusions and
 * monster / feet words in the text, then the sex rule with the text as a
 * last resort. `sexual` says where the sex showed (saved on the candidate).
 *
 * @param {{tags?: string[], title?: string}} topic
 * @param {string} text
 * @returns {{ok: false, reason: 'excluded' | 'not-sexual'} | {ok: true, sexual: 'tags' | 'title' | 'text'}}
 */
export function judgeFirstPost({ tags = [], title = '' }, text) {
  if (hasExcludedText(text) || textRuleBreak(text)) return { ok: false, reason: 'excluded' };
  const sexual = sexEvidence({ tags, title, text });
  return sexual ? { ok: true, sexual } : { ok: false, reason: 'not-sexual' };
}

/** True for a waiting candidate `--recheck` reads the first post of: it breaks only the sex rule and has no evidence saved. */
export function needsSexRecheck(c) {
  return !c.sexual && !c.contentBreak && contentRuleBreak(c) === 'not-sexual';
}

/**
 * A waiting candidate judged again with its first post's text (`--recheck`):
 * excluded words in the text mark it `contentBreak` for good, sex in the
 * text saves `sexual: 'text'`, anything else leaves it as it was (skipped).
 *
 * @returns {{outcome: 'excluded' | 'text' | 'not-sexual', candidate: object}}
 */
export function recheckWithText(c, text) {
  const rule = textRuleBreak(text) || (hasExcludedText(text) ? 'excluded' : null);
  if (rule) return { outcome: 'excluded', candidate: { ...c, contentBreak: rule } };
  if (sexEvidence({ tags: c.tags, title: c.title, text })) return { outcome: 'text', candidate: { ...c, sexual: 'text' } };
  return { outcome: 'not-sexual', candidate: c };
}

/** Discourse category of Free Scripts (Paid Scripts posts link to a store instead of files). */
export const FREE_SCRIPTS_CATEGORY = 14;

/**
 * Screen a post the user picked by id (`--add`): the exclusions, the
 * content rules (also on the first post, judgeFirstPost), posts the library
 * already links or the user turned down, and scenes the library already has
 * count. The animation and multi-axis rules don't apply, and a pick never
 * waits for the live quota.
 */
export function screenPick(topic, ctx) {
  const s = screenTopic(topic, ctx);
  return s.ok ? { ...s, liveQuota: false, picked: true } : s;
}

/** True when a live post isn't a favourite's nor a library creator's music video, so it waits for the live quota. */
export function liveOnQuota({ live, favourite, music, artist }) {
  return !!live && !favourite && !(music && artist);
}

/**
 * Whether a post's single-axis scenes may be downloaded: only VR + creampie
 * + (a creator already in the library, or a PMV / HMV). Multi-axis scenes
 * don't need this.
 *
 * @param {{tags?: string[], title?: string, vr?: boolean, artistInLibrary?: boolean}} post
 */
export function takesSingleAxis({ tags = [], title = '', vr = false, artistInLibrary = false }) {
  const creampie = tags.some((t) => t === 'creampie' || t.endsWith('-creampie')) || SINGLE_AXIS_CREAMPIE_TITLE.test(title);
  const pmv = tags.some((t) => PMV_TAGS.has(t)) || PMV_TITLE.test(title);
  return vr && creampie && (artistInLibrary || pmv);
}

/**
 * The scenes of a post worth downloading: every multi-axis one, and the
 * single-axis ones only when `takesSingleAxis` allows them.
 *
 * @param {{tags?: string[], title?: string, vr?: boolean, artistInLibrary?: boolean, scenes: {axes: string[]}[]}} post
 */
export function wantedScenes(post) {
  return takesSingleAxis(post) ? post.scenes : post.scenes.filter((sc) => sc.axes.length);
}

/**
 * Where a post shows penetrative sex, or null: `tags` for a sex tag
 * (vaginal, anal, riding, doggy, missionary, creampie, threesome …) or a
 * music-video tag, `title` for a sex word in the title (English, Japanese,
 * Chinese or Korean) or a music video named so, `text` for a sex word in
 * the first post. Blowjob, handjob, titjob or paizuri alone isn't sex. A
 * music video (PMV / HMV / Cock Hero / Fap Hero) counts unless it's a dance
 * or its title is about a non-sex act (`Blowjob PMV`, `Titjob HMV`, `JOI`)
 * without a sex word. A post tagged sfw / non-porn never is.
 *
 * @param {{tags?: string[], title?: string, text?: string}} post
 * @returns {'tags' | 'title' | 'text' | null}
 */
export function sexEvidence({ tags = [], title = '', text = '' }) {
  if (tags.some((t) => SFW_TAGS.has(t))) return null;
  if (tags.some((t) => SEX_TAGS.has(t))) return 'tags';
  if (hasSexWords(title)) return 'title';
  const musicTag = tags.some((t) => PMV_TAGS.has(t));
  if ((musicTag || PMV_TITLE.test(title)) && !(tags.includes('dance') || DANCE_TITLE.test(title)) && !NON_SEX_ACT_TITLE.test(title)) {
    return musicTag ? 'tags' : 'title';
  }
  return text && hasSexWords(text) ? 'text' : null;
}

const hasSexWords = (s) => {
  const words = String(s).replace(NON_SEX_PHRASES, ' ');
  return SEX_WORDS.test(words) || SEX_WORDS_CJK.test(words);
};

/** True when a post shows penetrative sex (sexEvidence) by its tags, title or, when given, first-post text. */
export function isSexual(post) {
  return sexEvidence(post) !== null;
}

/**
 * True for 2D hentai: animation tags only hentai / 2d / anime, no 3D tag
 * (3d, sfm, vam, mmd, cgi, blender, koikatsu …) and no 3D / SFM / VaM / MMD
 * in the title. Such posts rank lower (RANKING.hentaiOnly).
 *
 * @param {{tags?: string[], title?: string}} post
 */
export function isHentaiOnly({ tags = [], title = '' }) {
  return tags.some((t) => FLAT_HENTAI_TAGS.has(t)) && !tags.some((t) => THREE_D_TAGS.has(t)) && !THREE_D_TITLE.test(title);
}

/** Last action time of a parsed funscript in ms; 0 when it has none. */
export function scriptDurationMs(script) {
  const actions = Array.isArray(script?.actions) ? script.actions : [];
  return actions.reduce((max, a) => (Number.isFinite(a?.at) && a.at > max ? a.at : max), 0);
}

/** Ranking score; higher downloads first. */
export function scoreCandidate({ vr = false, music = false, artistInLibrary = false, preferred = [], multiAxis = false, likes = 0, hentaiOnly = false }) {
  return (vr ? RANKING.vr : 0)
    + (music ? RANKING.music : 0)
    + (artistInLibrary ? RANKING.artistInLibrary : 0)
    + (preferred.length ? RANKING.preferred + RANKING.preferredExtra * (preferred.length - 1) : 0)
    + (multiAxis ? RANKING.multiAxis : 0)
    + Math.min(RANKING.likesMax, likes / RANKING.likesPer)
    - (hentaiOnly ? RANKING.hentaiOnly : 0);
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
  const isVr = /(^|[^a-z])(vr|sbs|180|fisheye)([^a-z]|$)/.test(n);
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

// ── Folders ─────────────────────────────────────────────────────────────

const safeName = (s) => String(s).replace(/[/\\:*?"<>|]/g, '').replace(/\s+/g, ' ').trim();
// Leading title groups that label the post rather than name its artist.
const NOT_A_CREATOR = /multi|axis|(^|[^a-z])vr([^a-z]|$)|initiative|free|sr6|osr|handy|commission|request|suggest|vam|mmd|sfm|fap ?hero|cock ?hero|[hp]mv|r-?18|hentai|^[23]d$|^ai$|^iwara$/i;

/** True for a name that labels a script or genre (`Cock Hero`, `PMV`, `Multi-axis`) rather than naming an artist. */
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
  const { groups, rest } = splitGroups(String(text).replace(/^\s*(?:axis master|public release\s*:)\s*/i, ''));
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

/** Creator folder: the post's artist (see parseName), else the poster. */
export function creatorFolder(title, author) {
  return safeName(parseName(title, author).artist) || 'unknown';
}

/** The artist a title names itself (leading `[Name]` / `(Name)`, trailing `by Name`), or ''. */
export function titleArtist(title) {
  return parseName(title, '').artist;
}

/**
 * Scene folder and file stem, which the library shows as the title:
 * `Artist - Title (metadata)`. The title comes from the post; a post with
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
  const artistKey = squash(post.artist);
  const seen = new Set();
  meta = meta.filter((m) => {
    const k = squash(m);
    if (!k || k === artistKey || seen.has(k)) return false;
    seen.add(k);
    return true;
  });
  const named = (m) => safeName(`${post.artist} - ${core}${m.length ? ` (${m.join(', ')})` : ''}`);
  // Too long: drop metadata from the end, then cut the title.
  while (meta.length && Buffer.byteLength(named(meta)) > STEM_BYTES) meta = meta.slice(0, -1);
  return capBytes(named(meta), STEM_BYTES).replace(/[\s\-–—,;.(]+$/, '') + vrTail;
}

/** File name for a scene's script in the scan's spelling. */
export function scriptFileName(stem, axis) {
  return `${stem}${axis ? `.${axis}` : ''}.funscript`;
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
 * @param {string} incoming the Discovery folder
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

/** An EroScripts scene's name from its post (`eroscripts.json`). */
function postSceneName(s, video, shared) {
  const { title, author } = s.rec;
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
// per feed how deep the backfill has read, and the posts listed but not
// judged yet (a run stopped by a rate limit goes on with them next time):
//
//   { version, lastScanAt, feeds: {<tag>: {backfillPage, exhausted, doneSince?, olderFrom?}},
//     topics: {<id>: {verdict, rules, at}}, pending: {<id>: <listed topic>} }
//
// Feeds are newest activity first. A scan reads pages from the top until a
// page holds nothing to judge (or the page cap), then `backfill` more pages
// from where the backfill stopped last time, so older posts get covered a
// little on every run; `since` stops a feed's backfill at a date for good.
// Verdicts made under older rules count as unseen, and a `no-video` or
// `single-axis` post with activity after its verdict is judged again.

/** Bump when the screening rules change, so every post is judged again. */
export const RULES_VERSION = 5;

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

/** Verdicts scripters often fix later (adding a video or axes): judged again after new activity. */
export const REJUDGE_ON_BUMP = new Set(['no-video', 'single-axis']);

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

/** Remember the verdict on post `id` (`candidate` or a reject reason); it's no longer pending. */
export function markTopic(state, id, verdict, now = Date.now(), rules = RULES_VERSION) {
  state.topics[id] = { verdict, rules, at: new Date(now).toISOString() };
  if (state.pending) delete state.pending[id];
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

/**
 * The saved candidate list after a scan: candidates found now, plus earlier
 * ones that still have a scene to download, minus posts the user turned
 * down and posts that break the content rules (contentRuleBreak; lists
 * saved under older rules may hold some, picks too). Best score first.
 *
 * @param {object[]} previous candidates from the last list
 * @param {object[]} fresh candidates found by this scan
 * @param {(topicId: number, videoName: string) => boolean} isDone scene fetched or rejected
 * @param {Set<number>} [turnedDown] posts the user rejected
 */
export function mergeCandidates(previous, fresh, isDone, turnedDown = new Set()) {
  const byId = new Map();
  for (const c of [...(previous || []), ...(fresh || [])]) byId.set(c.id, c);
  return [...byId.values()]
    .filter((c) => !turnedDown.has(c.id) && !contentRuleBreak(c) && c.scenes.some((sc) => !isDone(c.id, sc.video.name)))
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
 * Download order: multi-axis posts first, then single-axis; each by score.
 * Live action on the quota comes last, so the run's other downloads can earn
 * it a slot. Posts a saved list holds that break the content rules
 * (contentRuleBreak) are dropped, picks included; so are scenes the rules no
 * longer want, except on posts the user picked (`--add`), which keep every
 * scene.
 */
export function downloadQueue(candidates) {
  const ok = candidates
    .filter((c) => !contentRuleBreak(c))
    .map((c) => ({ ...c, scenes: c.picked ? c.scenes : wantedScenes({ ...c, artistInLibrary: !!c.artistInLibrary }) }))
    .filter((c) => c.scenes.length);
  const byScore = (a, b) => b.score - a.score;
  const own = ok.filter((c) => !c.liveQuota);
  return [
    ...own.filter((c) => c.axisType === 'multi').sort(byScore), ...own.filter((c) => c.axisType !== 'multi').sort(byScore),
    ...ok.filter((c) => c.liveQuota).sort(byScore),
  ];
}

/**
 * Add videos to the Discovery playlist and make sure the incoming folder is a
 * library source, in a parsed config.json (mutated). Returns what changed.
 */
export function addToDiscovery(config, videoPaths, { incomingDir, newId, now = Date.now() }) {
  const changes = { sourceAdded: false, playlistCreated: false, added: [] };
  config.settings ||= {};
  config.settings.library ||= {};
  const lib = config.settings.library;
  lib.sources ||= [];
  if (!lib.sources.some((s) => path.resolve(s.path) === path.resolve(incomingDir))) {
    lib.sources.push({ id: newId(), name: DISCOVERY_NAME, path: incomingDir, enabled: true });
    changes.sourceAdded = true;
  }
  config.playlists ||= [];
  let pl = config.playlists.find((p) => String(p.name).trim().toLowerCase() === DISCOVERY_NAME.toLowerCase());
  if (!pl) {
    pl = { id: newId(), name: DISCOVERY_NAME, createdAt: now, videoPaths: [] };
    config.playlists.push(pl);
    changes.playlistCreated = true;
  }
  pl.videoPaths ||= [];
  for (const p of videoPaths) if (!pl.videoPaths.includes(p)) { pl.videoPaths.push(p); changes.added.push(p); }
  return changes;
}

/** Key of one fetched scene in the `eroscripts.json` sidecars: post id + chosen video file. */
export function fetchedKey(topicId, videoName) {
  return `${topicId}|${videoName}`;
}
