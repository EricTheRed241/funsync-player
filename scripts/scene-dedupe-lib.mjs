// Duplicate scenes: the same video in the library more than once, as other
// encodes (1080p / 4K, passthrough), another upload (FapTap re-uploads, a
// FapTap copy of an EroScripts scene) or with another script. Pure logic for
// `npm run dedupe:scenes` and the scouts; probing and frame checks are
// injected (see scripts/dedupe-scenes.mjs).
//
// Two scenes are the same video when they last as long (±1.5 s), have the
// same projection and orientation (VR stays apart from flat, portrait from
// landscape), their titles match once quality words are dropped (the shorter
// title's words all appear in the longer one, so `Rina` never matches
// `Yixuan`), and either their main scripts are identical or their frames are.
//
// Which one stays: DEDUPE_KEEP_ORDER in renderer/js/content-preferences.js
// (the first difference decides).

import { DEDUPE_KEEP_ORDER, VR_MAX_FPS, VR_MAX_HEIGHT } from '../renderer/js/content-preferences.js';

export const DURATION_TOLERANCE_S = 1.5;
/** Mean grey-level difference (0–255) under which two videos' frames are the same picture. */
export const SAME_FRAMES_MAX = 10;

// Words that describe an encode or a cut, not what the video shows.
const QUALITY_WORDS = new Set([
  'hq', 'lq', 'sd', 'hd', 'fhd', 'uhd', 'upscale', 'upscaled', 'remaster', 'remastered', 'passthrough', 'pt',
  'hmv', 'pmv', 'high', 'low', 'bitrate', 'lowbitrate', 'highbitrate', 'version', 'ver', 'final', 'fixed', 'full',
  'hevc', 'h264', 'h265', 'x264', 'x265', 'av1', 'mp4', 'mkv', 'compressed', 'uncompressed', 'edit',
]);
// `1080p`, `4k`, `60fps`, and run together: `4k120fps`, `1080p60fps`, `1080p60`.
const QUALITY_TOKEN = /^((\d{3,4}p|\d{1,2}k)(\d{2,3}(fps)?)?|\d{2,3}fps|fps\d{2,3}|v\d+(\.\d+)?|\d+bit)$/;
const VR_MARKER = /(^|[^a-z0-9])(vr|sbs|tb|ou|lr|rl|lrf|180|360|3dh|3dv|fisheye\d*|mkx\d+|vrca\d+|rf52|equirect)([^a-z0-9]|$)/i;

const fold = (s) => String(s).normalize('NFC').toLowerCase();

/** Words of a scene title that say what it shows (artist, quality and projection words dropped). */
export function titleWords(name, artist = '') {
  let s = fold(name).replace(/\.[a-z0-9]{2,4}$/, '');
  s = s.replace(/\s*\(\d\)\s*$/, ''); // "(2)": a second download of the same name
  const a = fold(artist).trim();
  if (a && s.startsWith(a)) s = s.slice(a.length);
  const words = s.split(/[^\p{L}\p{N}]+/u).filter(Boolean)
    .filter((w) => !QUALITY_WORDS.has(w) && !QUALITY_TOKEN.test(w) && !VR_MARKER.test(` ${w} `));
  return new Set(words);
}

/** True when every word of the shorter title is in the longer one. */
export function titlesMatch(a, b) {
  const [small, large] = a.size <= b.size ? [a, b] : [b, a];
  if (!small.size || ![...small].some((w) => w.length >= 3 || /\p{Script=Han}|\p{Script=Hiragana}|\p{Script=Katakana}|\p{Script=Hangul}/u.test(w))) return false;
  for (const w of small) if (!large.has(w)) return false;
  return true;
}

/** A picture shaped like VR: side by side (2:1) or top-bottom (1:1), at least 4K wide. */
function vrShape(width, height) {
  if (!width || !height || width < 2880) return false;
  const r = width / height;
  return Math.abs(r - 2) < 0.05 || Math.abs(r - 1) < 0.02;
}

/** VR or flat (name marker or picture shape), portrait or landscape. */
export function projectionOf({ name = '', vr, width = 0, height = 0 }) {
  return { vr: vr ?? (VR_MARKER.test(name) || vrShape(width, height)), portrait: height > width };
}

export function sameProjection(a, b) {
  const pa = projectionOf(a);
  const pb = projectionOf(b);
  return pa.vr === pb.vr && pa.portrait === pb.portrait;
}

/** Average speed of a funscript in position units per second (0 when it doesn't move). */
export function scriptSpeed(actions) {
  const a = (actions || []).filter((x) => Number.isFinite(x?.at) && Number.isFinite(x?.pos)).sort((x, y) => x.at - y.at);
  if (a.length < 2) return 0;
  let travel = 0;
  for (let i = 1; i < a.length; i++) travel += Math.abs(a[i].pos - a[i - 1].pos);
  const seconds = (a[a.length - 1].at - a[0].at) / 1000;
  return seconds > 0 ? travel / seconds : 0;
}

// How many VR playback caps a video breaks (flat videos have none).
const capBreaches = (s) => (projectionOf(s).vr ? ((s.height || 0) > VR_MAX_HEIGHT) + (Math.round(s.fps || 0) > VR_MAX_FPS) : 0);

// Each keep-order rule as a key; higher is better.
const KEY_OF = {
  'EroScripts over FapTap': (s) => (s.source === 'faptap' ? 0 : 1),
  'hand-made axes': (s) => (s.handmadeAxes > 0 ? 1 : 0),
  passthrough: (s) => (s.passthrough ? 1 : 0),
  'within VR caps': (s) => -capBreaches(s),
  'higher resolution': (s) => (s.width || 0) * (s.height || 0),
  'higher fps': (s) => Math.round(s.fps || 0),
  'faster script': (s) => Math.round((s.speed || 0) * 10) / 10,
  'higher bitrate': (s) => s.bitrate || 0,
};
// Keep order: earlier keys decide first.
const KEYS = DEDUPE_KEEP_ORDER.map((reason) => {
  if (!KEY_OF[reason]) throw new Error(`DEDUPE_KEEP_ORDER: unknown rule "${reason}"`);
  return [reason, KEY_OF[reason]];
});

/**
 * Order two scenes: negative when `a` should stay. Also returns why.
 * @returns {{order: number, reason: string}}
 */
export function compareScenes(a, b) {
  for (const [reason, key] of KEYS) {
    const d = key(b) - key(a);
    if (d !== 0) return { order: d, reason };
  }
  return { order: String(a.video).localeCompare(String(b.video)), reason: 'same in every way (kept the first)' };
}

/** True when two scenes may be the same video, before their frames are compared. */
export function maybeSame(a, b) {
  if (a.video === b.video) return false;
  if (!(Math.abs(a.duration - b.duration) <= DURATION_TOLERANCE_S)) return false;
  if (!sameProjection(a, b)) return false;
  return titlesMatch(a.words, b.words);
}

/**
 * The first of `others` with the same length (±1.5 s), projection and title
 * as `item`, or null. No frame check: for the scouts, before a download.
 * Items: `{words: Set<string>, duration, vr?, width?, height?}`.
 */
export function firstDuplicate(item, others) {
  return others.find((o) => Math.abs((o.duration || 0) - (item.duration || 0)) <= DURATION_TOLERANCE_S
    && sameProjection(o, item) && titlesMatch(o.words, item.words)) || null;
}

/**
 * Group duplicate scenes and pick the one to keep in each group.
 *
 * @param {object[]} scenes `{video, dir, duration, width, height, fps, bitrate, vr?, source, handmadeAxes,
 *   passthrough, speed, scriptHash, words: Set<string>}`
 * @param {(a: object, b: object) => Promise<boolean>} sameFrames compares the two videos' pictures
 * @returns {Promise<{keep: object, remove: {scene: object, reason: string}[]}[]>}
 */
export async function findDuplicates(scenes, sameFrames) {
  const sorted = [...scenes].filter((s) => s.duration > 0).sort((a, b) => a.duration - b.duration);
  const parent = new Map(sorted.map((s) => [s, s]));
  const root = (s) => { while (parent.get(s) !== s) s = parent.get(s); return s; };
  for (let i = 0; i < sorted.length; i++) {
    for (let j = i + 1; j < sorted.length && sorted[j].duration - sorted[i].duration <= DURATION_TOLERANCE_S; j++) {
      const a = sorted[i];
      const b = sorted[j];
      if (root(a) === root(b) || !maybeSame(a, b)) continue;
      const same = (a.scriptHash && a.scriptHash === b.scriptHash) || await sameFrames(a, b);
      if (same) parent.set(root(b), root(a));
    }
  }
  const groups = new Map();
  for (const s of sorted) {
    const r = root(s);
    groups.set(r, [...(groups.get(r) || []), s]);
  }
  return [...groups.values()].filter((g) => g.length > 1).map((g) => {
    const ranked = [...g].sort((a, b) => compareScenes(a, b).order);
    const keep = ranked[0];
    return { keep, remove: ranked.slice(1).map((scene) => ({ scene, reason: compareScenes(keep, scene).reason })) };
  });
}
