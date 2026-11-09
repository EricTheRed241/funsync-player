// Duplicate scenes: the same video in the library more than once, as other
// encodes (1080p / 4K, passthrough), another upload (FapTap re-uploads, a
// FapTap copy of an EroScripts scene) or with another script. Pure logic for
// `npm run dedupe:scenes` and the scouts; probing and frame extraction are
// injected (see scripts/dedupe-scenes.mjs).
//
// Two scenes can only be the same video with the same projection and
// orientation (VR stays apart from flat, portrait from landscape; the
// backend's stereo check decides VR when it has looked at the file). Titles
// match once quality, script-variant and projection words are dropped (the
// shorter title's words all appear in the longer one, so `Rina` never matches
// `Yixuan`; `Extreme` and `Hard` are dropped; genre words like `VAM` alone
// match nothing; a trailing `(2)` only goes when the rest is the same, so
// `Part 1 (1)` isn't `Part 1 (17)`).
//   • Matching titles, ±1.5 s: identical main scripts or matching frames.
//   • Matching titles, up to ±max(3 s, 1 %) (a trimmed or padded encode):
//     matching frames only.
//   • Other titles (a re-upload under another name), ±1.5 s: every sampled
//     frame must match and show a picture, not a blank or fade. Titles one
//     word apart each (`Haley` / `Tess`: one animation, another character;
//     or a label, `Queen Marika 2` / `Queen Marika - Sub`) must also match
//     everywhere in the picture, not just overall.
// Frames are compared as perceptual hashes (dHash) of the picture inside any
// black bars, at the same times counted from the start and from the end, each
// with a small search (±1 s) for the best alignment.
//
// A copy is removed only when it was itself compared with the copy that stays
// and matched it: A = B and B = C doesn't remove C next to A.
//
// Which video stays: DEDUPE_KEEP_ORDER; which scripts it keeps:
// DEDUPE_SCRIPT_ORDER (renderer/js/content-preferences.js; the first
// difference decides).

import {
  DEDUPE_KEEP_ORDER, DEDUPE_SCRIPT_ORDER, GENRE_WORDS, NAME_NOISE_TOKEN, NAME_NOISE_WORDS, VR_MAX_FPS, VR_MAX_HEIGHT,
} from '../renderer/js/content-preferences.js';

/** Any two titles: the same video lasts as long within this. */
export const DURATION_TOLERANCE_S = 1.5;
/** Matching titles: a trimmed or padded encode, within max(TITLE_WINDOW_MIN_S, TITLE_WINDOW_SHARE × the longer). */
export const TITLE_WINDOW_MIN_S = 3;
export const TITLE_WINDOW_SHARE = 0.01;
/** Most differing bits (of 144) between two frame hashes that show the same picture … */
export const SAME_HASH_MAX = 20;
/** … and at most this many on average over a video's samples (an overlay such as a Fap Hero beat bar stays apart). */
export const SAME_HASH_MEAN_MAX = 8;
/**
 * Titles that differ by one word each (`Haley` / `Tess`: one animation with
 * another character, only a face differs): the frames must also match
 * everywhere, every 16×16 cell of the picture within this many grey levels
 * (re-encodes stay within 5, a changed face or hair differs by 19+).
 */
export const SWAP_CELL_MAX = 12;
/** Grey-level spread (0–255) a frame needs to count as a picture (blanks and fades look alike). */
export const FRAME_DETAIL_MIN = 12;
/** Matching titles: at least this many sampled times must show a picture, and all of those must match. */
export const LOOSE_MIN_ANCHORS = 2;
/** Where frames are sampled: shares of the shorter video's length, from the start and from the end. */
export const FRAME_ANCHORS = [1, 2, 3, 4, 5].map((k) => k / 6);
/** Sample times are rounded to this, so a video's frames serve every pair it's in (they're cached). */
export const ANCHOR_GRID_S = 2;
/** Each sample is a burst of frames around its time: the alignment search. */
export const BURST_OFFSETS_S = [-0.5, 0, 0.5];
/** Lengths differing by more than this are also compared aligned at the end. */
export const END_ALIGN_MIN_S = 1;
/** Grey frame size the frames are extracted at (before the bars are cropped). */
export const FRAME_W = 128;
export const FRAME_H = 72;

const VR_MARKER = /(^|[^a-z0-9])(vr|sbs|tb|ou|lr|rl|lrf|180|360|3dh|3dv|fisheye\d*|mkx\d+|vrca\d+|rf52|equirect)([^a-z0-9]|$)/i;
const COPY_TOKEN = /^\(\d+\)$/;
const CJK = /\p{Script=Han}|\p{Script=Hiragana}|\p{Script=Katakana}|\p{Script=Hangul}/u;

/**
 * Names as compared everywhere: compatibility forms folded (NFKC, so `𝓣𝓪𝓷𝓪`
 * is `tana`), lower case, accents and other combining marks dropped (a
 * volume's decomposed `NÊMESIS` equals a forum's composed one).
 */
export function fold(s) {
  return String(s ?? '').normalize('NFKD').toLowerCase().replace(/\p{M}+/gu, '').normalize('NFKC');
}

/** True for a word that describes the encode, cut, variant or projection, not the video (NAME_NOISE_WORDS). */
export function isNoiseWord(w) {
  return NAME_NOISE_WORDS.has(w) || NAME_NOISE_TOKEN.test(w) || VR_MARKER.test(` ${w} `);
}

/** True for a word that can tell two titles apart: not a genre word (GENRE_WORDS) or a copy number, 3+ letters or CJK. */
export function isDistinctive(w) {
  return !GENRE_WORDS.has(w) && !COPY_TOKEN.test(w) && (w.length >= 3 || CJK.test(w));
}

/**
 * Words of a scene title that say what it shows (artist, quality, variant and
 * projection words dropped). A trailing `(n)` (a second download, or a
 * series number) is kept as the word `(n)`; titlesMatch drops it when the
 * rest is the same.
 */
export function titleWords(name, artist = '') {
  let s = fold(name).replace(/\.[a-z0-9]{2,4}$/, '');
  const a = fold(artist).trim();
  if (a && s.startsWith(a)) s = s.slice(a.length);
  const copy = s.match(/\s*\((\d{1,3})\)\s*$/);
  if (copy) s = s.slice(0, copy.index);
  s = s.replace(/(^|[^\p{L}\p{N}])(fap|cock)[\s_-]*hero(?![\p{L}\p{N}])/gu, '$1$2hero');
  const words = s.split(/[^\p{L}\p{N}]+/u).filter((w) => w && !isNoiseWord(w));
  if (copy) words.push(`(${Number(copy[1])})`);
  return new Set(words);
}

const withoutCopy = (s) => new Set([...s].filter((w) => !COPY_TOKEN.test(w)));
const sameSet = (a, b) => a.size === b.size && [...a].every((w) => b.has(w));

/**
 * True when every word of the shorter title is in the longer one, and it has
 * a distinctive word. Copy numbers: `X (2)` matches `X`, `Part 1 (1)` doesn't
 * match `Part 1 (17)` (two numbers are a series) nor `Part 1 Extended`.
 */
export function titlesMatch(a, b) {
  const ca = [...a].find((w) => COPY_TOKEN.test(w));
  const cb = [...b].find((w) => COPY_TOKEN.test(w));
  if (ca !== cb) {
    if (ca && cb) return false;
    const sa = withoutCopy(a);
    const sb = withoutCopy(b);
    if (sameSet(sa, sb)) [a, b] = [sa, sb];
  }
  const [small, large] = a.size <= b.size ? [a, b] : [b, a];
  if (![...small].some(isDistinctive)) return false;
  for (const w of small) if (!large.has(w)) return false;
  return true;
}

/**
 * True when two titles differ by exactly one word each: maybe one animation
 * with another character (`Haley` / `Tess`), maybe a label (`Queen Marika 2`
 * / `Queen Marika - Sub`). Only frames that match everywhere make them one.
 */
export function characterSwap(a, b) {
  const onlyA = [...a].filter((w) => !b.has(w));
  const onlyB = [...b].filter((w) => !a.has(w));
  return onlyA.length === 1 && onlyB.length === 1 && a.size > 1;
}

/** A picture shaped like VR: side by side (2:1) or top-bottom (1:1), at least 4K wide. */
function vrShape(width, height) {
  if (!width || !height || width < 2880) return false;
  const r = width / height;
  return Math.abs(r - 2) < 0.05 || Math.abs(r - 1) < 0.02;
}

/**
 * VR or flat, portrait or landscape. `vr` given (the backend's stereo check,
 * see scene-probe.mjs stereoVr) wins; else a name marker or the picture's
 * shape. `portrait` is null when the size is unknown.
 */
export function projectionOf({ name = '', vr, width = 0, height = 0 }) {
  return { vr: vr ?? (VR_MARKER.test(name) || vrShape(width, height)), portrait: width && height ? height > width : null };
}

/** Same VR / flat, and same orientation unless one's is unknown. */
export function sameProjection(a, b) {
  const pa = projectionOf(a);
  const pb = projectionOf(b);
  return pa.vr === pb.vr && (pa.portrait === null || pb.portrait === null || pa.portrait === pb.portrait);
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

// Each rule as a key; higher is better.
const KEY_OF = {
  'within VR caps': (s) => -capBreaches(s),
  'bigger video': (s) => s.size || 0,
  'hand-made axes': (s) => (s.handmadeAxes > 0 ? 1 : 0),
  'faster script': (s) => Math.round((s.speed || 0) * 10) / 10,
};
const keysOf = (order, name) => order.map((reason) => {
  if (!KEY_OF[reason]) throw new Error(`${name}: unknown rule "${reason}"`);
  return [reason, KEY_OF[reason]];
});
const VIDEO_KEYS = keysOf(DEDUPE_KEEP_ORDER, 'DEDUPE_KEEP_ORDER');
const SCRIPT_KEYS = keysOf(DEDUPE_SCRIPT_ORDER, 'DEDUPE_SCRIPT_ORDER');

function compareBy(keys, a, b) {
  for (const [reason, key] of keys) {
    const d = key(b) - key(a);
    if (d !== 0) return { order: d, reason };
  }
  return { order: String(a.video).localeCompare(String(b.video)), reason: 'same in every way (kept the first)' };
}

/**
 * Order two copies by their video (DEDUPE_KEEP_ORDER): negative when `a`'s
 * video should stay. Also returns why.
 * @returns {{order: number, reason: string}}
 */
export function compareScenes(a, b) {
  return compareBy(VIDEO_KEYS, a, b);
}

/** Order two copies by their scripts (DEDUPE_SCRIPT_ORDER): negative when `a`'s scripts should stay. */
export function compareScripts(a, b) {
  return compareBy(SCRIPT_KEYS, a, b);
}

/** How far apart two scenes with matching titles may last: max(3 s, 1 % of the longer). */
export function titleWindow(a, b) {
  return Math.max(TITLE_WINDOW_MIN_S, TITLE_WINDOW_SHARE * Math.max(a.duration || 0, b.duration || 0));
}

// ── Frames ──────────────────────────────────────────────────────────────

/**
 * The picture inside dark, flat bars (letterbox, pillarbox) of a grey frame
 * `w`×`h`: `{x, y, w, h}`. The whole frame when the bars would leave less
 * than a quarter of either side (a dark frame).
 */
export function pictureBox(px, w, h) {
  const dark = (from, step, count) => {
    let min = 255; let max = 0;
    for (let i = 0, p = from; i < count; i++, p += step) { const v = px[p]; if (v < min) min = v; if (v > max) max = v; }
    return max <= 24 && max - min <= 12;
  };
  let top = 0;
  while (top < h && dark(top * w, 1, w)) top++;
  let bottom = h;
  while (bottom > top && dark((bottom - 1) * w, 1, w)) bottom--;
  let left = 0;
  while (left < w && dark(top * w + left, w, bottom - top)) left++;
  let right = w;
  while (right > left && dark(top * w + right - 1, w, bottom - top)) right--;
  if ((right - left) * 4 < w || (bottom - top) * 4 < h) return { x: 0, y: 0, w, h };
  return { x: left, y: top, w: right - left, h: bottom - top };
}

// `n`×`n` mean grey levels of the picture box.
function cellMeans(px, w, box, n) {
  const out = new Float64Array(n * n);
  for (let r = 0; r < n; r++) {
    const y0 = box.y + Math.floor((r * box.h) / n);
    const y1 = Math.max(y0 + 1, box.y + Math.floor(((r + 1) * box.h) / n));
    for (let c = 0; c < n; c++) {
      const x0 = box.x + Math.floor((c * box.w) / n);
      const x1 = Math.max(x0 + 1, box.x + Math.floor(((c + 1) * box.w) / n));
      let sum = 0;
      for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) sum += px[y * w + x];
      out[r * n + c] = sum / ((y1 - y0) * (x1 - x0));
    }
  }
  return out;
}

const HASH_GRID = 9;
/** Cells per side of a frame's fine grid (`cells`, for SWAP_CELL_MAX). */
export const CELL_GRID = 16;

/**
 * Perceptual hash of a grey frame `w`×`h` (bytes): the picture inside its
 * bars averaged to 9×9 cells, then whether each cell is brighter than its
 * right and its lower neighbour (dHash, 144 bits as hex). `detail`: those
 * cells' grey-level spread (a blank or a fade has little). `cells`: the
 * picture as 16×16 grey levels.
 * @returns {{hash: string, detail: number, cells: Uint8Array}}
 */
export function frameHash(px, w = FRAME_W, h = FRAME_H) {
  const box = pictureBox(px, w, h);
  const g = cellMeans(px, w, box, HASH_GRID);
  const n = HASH_GRID;
  const bits = [];
  for (let r = 0; r < n; r++) for (let c = 0; c < n - 1; c++) bits.push(g[r * n + c] > g[r * n + c + 1]);
  for (let r = 0; r < n - 1; r++) for (let c = 0; c < n; c++) bits.push(g[r * n + c] > g[(r + 1) * n + c]);
  let hash = '';
  for (let i = 0; i < bits.length; i += 4) hash += ((bits[i] << 3) | (bits[i + 1] << 2) | (bits[i + 2] << 1) | bits[i + 3]).toString(16);
  let min = 255; let max = 0;
  for (const v of g) { if (v < min) min = v; if (v > max) max = v; }
  return { hash, detail: Math.round(max - min), cells: Uint8Array.from(cellMeans(px, w, box, CELL_GRID), Math.round) };
}

/** Differing bits between two frame hashes (Infinity when they can't be compared). */
export function hashDistance(a, b) {
  if (!a || !b || a.length !== b.length) return Infinity;
  let d = 0;
  for (let i = 0; i < a.length; i++) {
    let x = parseInt(a[i], 16) ^ parseInt(b[i], 16);
    while (x) { d += x & 1; x >>= 1; }
  }
  return d;
}

/** Largest difference between two frames' fine grids, brightness evened out (Infinity when either is missing). */
export function cellDistance(a, b) {
  if (!a || !b || a.length !== b.length || !a.length) return Infinity;
  const mean = (x) => x.reduce((s, v) => s + v, 0) / x.length;
  const shift = mean(a) - mean(b);
  let peak = 0;
  for (let i = 0; i < a.length; i++) peak = Math.max(peak, Math.abs(a[i] - b[i] - shift));
  return peak;
}

/**
 * Where to sample two videos: the same times from the start, and when their
 * lengths differ by more than END_ALIGN_MIN_S also from the end (an intro
 * or outro one of them lacks). `[{a: number[], b: number[]}]` in seconds.
 */
export function framePlan(a, b) {
  const shorter = Math.min(a.duration, b.duration);
  const offsets = FRAME_ANCHORS.map((f) => Math.max(ANCHOR_GRID_S, Math.round((shorter * f) / ANCHOR_GRID_S) * ANCHOR_GRID_S));
  const plan = [{ a: offsets, b: offsets }];
  if (Math.abs(a.duration - b.duration) > END_ALIGN_MIN_S) {
    plan.push({ a: offsets.map((o) => a.duration - o), b: offsets.map((o) => b.duration - o) });
  }
  return plan;
}

/** Index of the frame at the sample time itself in a burst. */
export const BURST_CENTRE = BURST_OFFSETS_S.indexOf(0);

/**
 * One sample time of two videos: `blank` when either has no picture there
 * (or no frame); else the fewest differing bits between the two bursts'
 * frames (the best alignment) and whether that's the same picture. `swap`:
 * the frames at the time itself must also match everywhere (SWAP_CELL_MAX).
 * @param {({hash: string, detail: number, cells?: Uint8Array}|null)[]|null} fa burst at BURST_OFFSETS_S
 * @param {({hash: string, detail: number, cells?: Uint8Array}|null)[]|null} fb
 * @returns {{verdict: 'blank'|'same'|'different', distance?: number}}
 */
export function burstVerdict(fa, fb, { swap = false } = {}) {
  const ca = fa?.[BURST_CENTRE];
  const cb = fb?.[BURST_CENTRE];
  if (!ca || !cb || ca.detail < FRAME_DETAIL_MIN || cb.detail < FRAME_DETAIL_MIN) return { verdict: 'blank' };
  // Blank frames (a cut through black) would match each other: only pictures align.
  const pictures = (f) => f.filter((x) => x && x.detail >= FRAME_DETAIL_MIN);
  let distance = Infinity;
  for (const x of pictures(fa)) for (const y of pictures(fb)) distance = Math.min(distance, hashDistance(x.hash, y.hash));
  const same = distance <= SAME_HASH_MAX && (!swap || cellDistance(ca.cells, cb.cells) <= SWAP_CELL_MAX);
  return { verdict: same ? 'same' : 'different', distance };
}

/**
 * Whether two videos show the same picture, at framePlan's times, aligned at
 * the start or at the end: every sample with a picture matches, and on
 * average within SAME_HASH_MEAN_MAX. Loose (matching titles): at least
 * LOOSE_MIN_ANCHORS samples show a picture. Strict (other titles): every
 * sample does. Swap (strict, titles one word apart each): see SWAP_CELL_MAX.
 *
 * @param {object} a scene `{video, duration, …}`
 * @param {object} b
 * @param {(scene: object, at: number) => Promise<({hash, detail, cells?}|null)[]|null>} burstAt frames
 *   around `at` seconds (BURST_OFFSETS_S), null when they can't be read
 * @param {{strict?: boolean, swap?: boolean}} [opts]
 */
export async function framesMatch(a, b, burstAt, { strict = false, swap = false } = {}) {
  strict ||= swap;
  for (const times of framePlan(a, b)) {
    const distances = [];
    let ok = true;
    for (let k = 0; ok && k < times.a.length; k++) {
      const { verdict, distance } = burstVerdict(await burstAt(a, times.a[k]), await burstAt(b, times.b[k]), { swap });
      if (verdict === 'different' || (strict && verdict === 'blank')) ok = false;
      else if (verdict === 'same') distances.push(distance);
    }
    if (ok && distances.length >= (strict ? times.a.length : LOOSE_MIN_ANCHORS)
      && distances.reduce((s, d) => s + d, 0) / distances.length <= SAME_HASH_MEAN_MAX) return true;
  }
  return false;
}

/**
 * The first of `others` with the same length (±1.5 s), projection and title
 * as `item`, or null. No frame check: for the scouts, before a download.
 * Items: `{words: Set<string>, duration, vr?, width?, height?}`; an item
 * without a size matches either orientation.
 */
export function firstDuplicate(item, others) {
  return others.find((o) => Math.abs((o.duration || 0) - (item.duration || 0)) <= DURATION_TOLERANCE_S
    && sameProjection(o, item) && titlesMatch(o.words, item.words)) || null;
}

/**
 * Group duplicate scenes; in each group pick the copy whose video stays
 * (compareScenes) and the copy whose scripts stay (compareScripts; often the
 * same one). Every removed copy matched the kept one itself.
 *
 * @param {object[]} scenes `{video, dir, duration, width, height, fps, size, vr?, handmadeAxes, speed,
 *   scriptHash, words: Set<string>}`
 * @param {(a: object, b: object, opts: {strict: boolean, swap: boolean}) => Promise<boolean>} sameFrames
 *   compares the two videos' pictures (see framesMatch); `strict` when their titles don't match, `swap`
 *   when they differ by one word each (characterSwap)
 * @returns {Promise<{keep: object, script: object, scriptReason: string|null, remove: {scene: object, reason: string}[]}[]>}
 */
export async function findDuplicates(scenes, sameFrames) {
  const sorted = [...scenes].filter((s) => s.duration > 0).sort((a, b) => a.duration - b.duration);
  const matched = new Map(sorted.map((s) => [s, new Set()]));
  for (let i = 0; i < sorted.length; i++) {
    for (let j = i + 1; j < sorted.length && sorted[j].duration - sorted[i].duration <= titleWindow(sorted[i], sorted[j]); j++) {
      const a = sorted[i];
      const b = sorted[j];
      if (a.video === b.video || !sameProjection(a, b)) continue;
      const close = b.duration - a.duration <= DURATION_TOLERANCE_S;
      let same = false;
      if (titlesMatch(a.words, b.words)) {
        same = (close && a.scriptHash && a.scriptHash === b.scriptHash) || await sameFrames(a, b, { strict: false });
      } else if (close) {
        same = await sameFrames(a, b, { strict: true, swap: characterSwap(a.words, b.words) });
      }
      if (same) { matched.get(a).add(b); matched.get(b).add(a); }
    }
  }
  // Best copies first; each keeps the copies it matched that no better copy took.
  const byKeep = [...sorted].sort((a, b) => compareScenes(a, b).order);
  const taken = new Set();
  const groups = [];
  for (const keep of byKeep) {
    if (taken.has(keep)) continue;
    const copies = byKeep.filter((s) => !taken.has(s) && matched.get(keep).has(s));
    if (!copies.length) continue;
    for (const s of [keep, ...copies]) taken.add(s);
    const g = [keep, ...copies];
    const best = [...g].sort((a, b) => compareScripts(a, b).order)[0];
    // A tie, or the very same script: the kept video's own scripts stay.
    const tie = SCRIPT_KEYS.every(([, key]) => key(best) === key(keep)) || (best.scriptHash && best.scriptHash === keep.scriptHash);
    const script = tie ? keep : best;
    groups.push({
      keep,
      script,
      scriptReason: script === keep ? null : compareScripts(script, keep).reason,
      remove: copies.map((scene) => ({ scene, reason: compareScenes(keep, scene).reason })),
    });
  }
  return groups;
}
