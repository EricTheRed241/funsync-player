// upgrade-multi-axis — materialise generated extra axes as companion files.
//
// Live playback already drives unscripted twist/roll/pitch via motion-source.js
// (tcode-sync). That does not upgrade the file: another player, or a later
// session with generation off, still sees stroke-only. This module writes the
// same generated values to OFS-canonical companions (`video.twist.funscript`
// and so on) that FunSync's library scanner already round-trips.
//
// The original stroke file is never written. An axis whose companion already
// exists is skipped. A script with any hand-made axes (see handmade-axes.js)
// gets nothing at all: hand-made axis scripts always win.

import { generateAxisActions, shapeGeneratedValue, axisPhase, createNoise1D } from './motion-source.js';
import { getBaseName, detectCompanionFiles } from './multi-axis.js';
import { buildCompanionFiles } from './embedded-multi-axis.js';

/**
 * Default extra axes for OSR / MiraBot Lite.
 *
 * None of them is a full copy of L0. Each keyframe is scaled to one of
 * 0 / 25 / 50 / 75 / 100% around centre. Twist's odds rise through the
 * video; roll and pitch fall. On top of that, smooth noise changes the
 * result by 10–60% of that scaled motion, so the build stays visible
 * and the path is not a schedule. A 0% rest is centre, and it cannot
 * last more than 10 seconds.
 */
export const DEPTH_STEPS = Object.freeze([0, 25, 50, 75, 100]);
export const REST_LIMIT_MS = 10_000;
const HOLD_MS = 4_000;

export const DEFAULT_UPGRADE_AXES = Object.freeze([
  { suffix: 'twist', tcode: 'R0', mode: 'link', ramp: 'up' },
  { suffix: 'roll',  tcode: 'R1', mode: 'random', ramp: 'down' },
  { suffix: 'pitch', tcode: 'R2', mode: 'pattern', pattern: 'sine', speed: 0.5, ramp: 'down' },
]);

/**
 * Surge / sway / valve. Not invented unless the caller says the user already
 * enabled that TCode channel. Vibe / pump / lube / suction are never generated.
 */
export const OPTIONAL_UPGRADE_AXES = Object.freeze([
  { suffix: 'surge', tcode: 'L1', mode: 'random' },
  { suffix: 'sway',  tcode: 'L2', mode: 'random' },
  { suffix: 'valve', tcode: 'A0', mode: 'pattern', pattern: 'sine', speed: 0.35 },
]);

/**
 * Companion path next to a primary funscript.
 * `video.funscript` → `video.generated.twist.funscript`.
 * A call with `video.generated.twist.funscript` still yields
 * `video.generated.roll.funscript`, not a doubled marker.
 *
 * @param {string} scriptPath
 * @param {string} suffix
 * @returns {string}
 */
export function companionPathForScript(scriptPath, suffix) {
  if (!scriptPath || !suffix) return '';
  const idx = Math.max(scriptPath.lastIndexOf('/'), scriptPath.lastIndexOf('\\'));
  const dir = idx >= 0 ? scriptPath.slice(0, idx + 1) : '';
  return `${dir}${getBaseName(scriptPath)}.generated.${suffix}.funscript`;
}

/**
 * @param {Iterable<string>|Set<string>|null|undefined} enabledTcodes
 *        TCode ids (`L1`) or suffixes (`surge`) the user has explicitly enabled.
 * @returns {Array<{suffix:string, tcode:string, mode:string, pattern?:string, speed?:number}>}
 */
export function resolveUpgradeAxes(enabledTcodes) {
  const enabled = enabledTcodes instanceof Set
    ? enabledTcodes
    : new Set(enabledTcodes || []);
  const extra = [];
  for (const axis of OPTIONAL_UPGRADE_AXES) {
    if (enabled.has(axis.tcode) || enabled.has(axis.suffix)) extra.push(axis);
  }
  return [...DEFAULT_UPGRADE_AXES, ...extra];
}

/**
 * Suffixes already present as companions of `primaryPath` in `allFiles`.
 * @param {string} primaryPath
 * @param {string[]} allFiles
 * @returns {Set<string>}
 */
export function existingSuffixesFromFiles(primaryPath, allFiles) {
  return new Set(
    detectCompanionFiles(primaryPath, allFiles).map((c) => c.axis.suffix),
  );
}

/**
 * Relative weight of each depth step. `bias` 0 prefers 0%, 1 prefers 100%.
 * Every step keeps a small floor so a late twist can still rest, and an
 * early one can still go full — the ramp is odds, not a hard fade.
 *
 * @param {number} bias 0..1
 * @returns {number[]}
 */
export function depthWeights(bias) {
  const b = Math.min(1, Math.max(0, bias));
  return DEPTH_STEPS.map((step) => {
    const d = Math.abs(step / 100 - b);
    return Math.exp(-d * d * 3) + 0.08;
  });
}

function pickWeighted(weights, unit) {
  const sum = weights.reduce((a, c) => a + c, 0) || 1;
  let x = Math.min(0.999999, Math.max(0, unit)) * sum;
  for (let i = 0; i < DEPTH_STEPS.length; i++) {
    x -= weights[i];
    if (x < 0) return DEPTH_STEPS[i];
  }
  return DEPTH_STEPS[DEPTH_STEPS.length - 1];
}

/**
 * Piecewise depth for one axis. Non-zero steps hold for 4s. A 0% step
 * holds at most 10s, and the next step cannot be 0%.
 *
 * @param {Array<{at:number}>} strokeActions
 * @param {{ramp?: 'up'|'down', noise?: (t:number)=>number, phase?: number}} [opts]
 * @returns {Array<{from:number, until:number, depth:number}>}
 */
export function depthSchedule(strokeActions, { ramp = 'up', noise = null, phase = 0 } = {}) {
  const times = [];
  for (const a of strokeActions || []) {
    if (a && Number.isFinite(a.at)) times.push(a.at);
  }
  if (times.length < 2) return [];
  const start = times[0];
  const end = times[times.length - 1];
  const span = Math.max(1, end - start);
  const sample = noise || (() => 0);
  const segments = [];
  let t = start;
  let restSince = null;
  while (t < end) {
    const u = (t - start) / span;
    const bias = ramp === 'down' ? 1 - u : u;
    let weights = depthWeights(bias);
    if (restSince != null && t - restSince >= REST_LIMIT_MS) {
      weights = weights.map((w, i) => (DEPTH_STEPS[i] === 0 ? 0 : w));
    }
    const unit = (sample(t / 1000 + phase) + 1) / 2;
    const depth = pickWeighted(weights, unit);
    const hold = depth === 0 ? REST_LIMIT_MS : HOLD_MS;
    const until = Math.min(end, t + hold);
    segments.push({ from: t, until: until >= end ? end + 1 : until, depth });
    restSince = depth === 0 ? (restSince ?? t) : null;
    if (until <= t) break;
    t = until;
  }
  return segments;
}

function depthAt(segments, at) {
  for (const s of segments) {
    if (at >= s.from && at < s.until) return s.depth;
  }
  return segments.length ? segments[segments.length - 1].depth : 100;
}

/** Noise is a percentage of the scaled motion, never a flat point offset. */
const NOISE_MIN = 0.10;
const NOISE_MAX = 0.60;

function applyDepth(actions, segments, noise, phase) {
  const sample = noise || (() => 0);
  return actions.map((a) => {
    const shaped = shapeGeneratedValue(a.pos, { depth: depthAt(segments, a.at) });
    const n = sample(a.at / 1000 + phase + 23);
    const mag = NOISE_MIN + ((n + 1) / 2) * (NOISE_MAX - NOISE_MIN);
    const sign = sample(a.at / 1000 + phase + 47) >= 0 ? 1 : -1;
    return { at: a.at, pos: shapeGeneratedValue(50 + (shaped - 50) * (1 + sign * mag)) };
  });
}

/**
 * Plan companion writes for a stroke-only script. Pure — no I/O.
 *
 * @param {object} opts
 * @param {string} opts.primaryPath — stroke .funscript path (never overwritten)
 * @param {Array<{at:number, pos:number}>} opts.strokeActions
 * @param {Iterable<string>|Set<string>} [opts.existingSuffixes]
 * @param {Iterable<string>} [opts.handmadeAxes] — the script's hand-made axis
 *        suffixes (`handmadeAxisSuffixes`); any → no writes, error 'handmade-axes'
 * @param {Iterable<string>|Set<string>} [opts.enabledTcodes]
 * @param {(t:number)=>number} [opts.noise] — inject a seeded field; do not use Math.random
 * @returns {{
 *   writes: Array<{suffix:string, tcode:string, mode:string, path:string, actions:Array<{at:number, pos:number}>, segments:Array}>,
 *   skipped: Array<{suffix:string, tcode:string, path:string, reason:string}>,
 *   error: string|null,
 * }}
 */
export function planUpgrade({
  primaryPath,
  strokeActions,
  existingSuffixes,
  handmadeAxes,
  enabledTcodes,
  noise,
} = {}) {
  const stroke = Array.isArray(strokeActions) ? strokeActions : [];
  const existing = existingSuffixes instanceof Set
    ? existingSuffixes
    : new Set(existingSuffixes || []);

  if (!primaryPath) {
    return { writes: [], skipped: [], error: 'no-path' };
  }
  const handmade = [...(handmadeAxes || [])];
  if (handmade.length > 0) {
    return { writes: [], skipped: [], error: 'handmade-axes', handmadeAxes: handmade };
  }
  let actionCount = 0;
  for (const a of stroke) {
    if (a && Number.isFinite(a.at)) actionCount++;
  }
  if (actionCount < 2) {
    return { writes: [], skipped: [], error: 'need-actions' };
  }
  const field = noise || createNoise1D();
  const writes = [];
  const skipped = [];
  for (const axis of resolveUpgradeAxes(enabledTcodes)) {
    const path = companionPathForScript(primaryPath, axis.suffix);
    if (existing.has(axis.suffix)) {
      skipped.push({ suffix: axis.suffix, tcode: axis.tcode, path, reason: 'exists' });
      continue;
    }
    const phase = axisPhase(axis.tcode);
    const full = generateAxisActions(stroke, {
      mode: axis.mode,
      pattern: axis.pattern,
      speed: axis.speed,
      depth: 100,
    }, { tcode: axis.tcode, noise: field });
    const segments = axis.ramp
      ? depthSchedule(stroke, { ramp: axis.ramp, noise: field, phase })
      : null;
    const actions = segments ? applyDepth(full, segments, field, phase) : full;
    writes.push({
      suffix: axis.suffix,
      tcode: axis.tcode,
      mode: axis.mode,
      path,
      actions,
      segments,
    });
  }
  return { writes, skipped, error: null };
}

/**
 * Serialise planned writes into `.funscript`-shaped blobs the scanner already
 * reads (same shape as extract-embedded → companions).
 *
 * @param {Array<{suffix:string, actions:Array}>} writes
 * @returns {Array<{suffix:string, content:object}>}
 */
export function serialiseUpgradeWrites(writes) {
  const map = new Map();
  for (const w of writes || []) {
    if (!w?.suffix || !Array.isArray(w.actions)) continue;
    map.set(w.suffix, w.actions);
  }
  return buildCompanionFiles(map);
}
