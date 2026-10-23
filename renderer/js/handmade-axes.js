// Hand-made axes — the one rule for "does this stroke script have axes a
// scripter made?", shared by the player, `npm run upgrade:axes` and
// `npm run prune:generated`.
//
// Policy: hand-made axis scripts always win. A scene (a stroke script and
// its video) has hand-made axes when either
//   - a companion `<stem>.<axis>.funscript` exists that is not a
//     `<stem>.generated.<axis>.funscript`, or
//   - the main script embeds extra axes (any of the six formats read by
//     `extractEmbeddedAxes`; an axis needs at least 2 actions).
// Such a scene never plays, gets or keeps generated companions.

import { getAxisBySuffix, isGeneratedCompanion, listCompanionFiles } from './multi-axis.js';
import { extractEmbeddedAxes } from './embedded-multi-axis.js';

/** Embedded-extractor suffix (`suction`) → companion-file suffix (`suck`). */
export function canonicalAxisSuffix(suffix) {
  return getAxisBySuffix(suffix)?.suffix || suffix;
}

const dirOf = (p) => {
  const s = String(p || '').replace(/\\/g, '/');
  const i = s.lastIndexOf('/');
  return i >= 0 ? s.slice(0, i) : '';
};

/**
 * The entries of `allFiles` that sit in the same folder as any of `anchors`
 * (a scene's video and stroke script). Keeps a same-named script elsewhere
 * in the library from counting as this scene's companion.
 *
 * @param {string[]} allFiles — paths
 * @param {...string} anchors — paths; falsy entries are ignored
 * @returns {string[]}
 */
export function filesBeside(allFiles, ...anchors) {
  const dirs = new Set(anchors.filter(Boolean).map(dirOf));
  return (allFiles || []).filter((f) => dirs.has(dirOf(f)));
}

/**
 * Canonical suffixes of the hand-made axes of a stroke script, sorted.
 *
 * @param {object} opts
 * @param {string} opts.primaryPath — the stroke .funscript (or its anchor path)
 * @param {string[]} [opts.allFiles] — filenames or paths to look for companions in
 * @param {object|null} [opts.parsed] — the parsed main script, for embedded axes
 * @returns {string[]}
 */
export function handmadeAxisSuffixes({ primaryPath, allFiles = [], parsed = null } = {}) {
  const found = new Set();
  for (const c of listCompanionFiles(primaryPath, allFiles)) {
    if (!c.generated) found.add(c.axis.suffix);
  }
  for (const suffix of extractEmbeddedAxes(parsed).keys()) found.add(canonicalAxisSuffix(suffix));
  return [...found].sort();
}

/** True when the stroke script has any hand-made axes. Same options as `handmadeAxisSuffixes`. */
export function hasHandmadeAxes(opts) {
  return handmadeAxisSuffixes(opts).length > 0;
}

/**
 * Generated companions (`<stem>.generated.<axis>.funscript`) of a stroke
 * script in `allFiles`, in input order.
 *
 * @param {string} primaryPath
 * @param {string[]} allFiles
 * @returns {string[]}
 */
export function generatedCompanionFiles(primaryPath, allFiles) {
  return listCompanionFiles(primaryPath, allFiles).filter((c) => c.generated).map((c) => c.path);
}

/**
 * Apply the policy to a saved multi-axis map (suffix → path). When the
 * script has hand-made axes, every generated path is dropped and the
 * script's hand-made companion files fill the axes they cover. Embedded
 * axes need no path: the player feeds them from the main script.
 *
 * @param {Object<string,string>} axes
 * @param {object} opts — as `handmadeAxisSuffixes`
 * @returns {Object<string,string>} `axes` itself when nothing changes
 */
export function preferHandmadeAxes(axes, { primaryPath, allFiles = [], parsed = null } = {}) {
  if (!axes || !hasHandmadeAxes({ primaryPath, allFiles, parsed })) return axes;
  const out = {};
  let changed = false;
  for (const [suffix, path] of Object.entries(axes)) {
    if (path && isGeneratedCompanion(path)) changed = true;
    else out[suffix] = path;
  }
  for (const c of listCompanionFiles(primaryPath, allFiles)) {
    if (c.generated || out[c.axis.suffix]) continue;
    out[c.axis.suffix] = c.path;
    changed = true;
  }
  return changed ? out : axes;
}
