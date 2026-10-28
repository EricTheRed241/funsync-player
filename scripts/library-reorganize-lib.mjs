// Logic for `npm run library:reorganize` (scripts/library-reorganize.mjs):
// sort every scene folder of the library into one folder per source,
//
//   <root>/EroScripts/<creator>/<scene>/   an `eroscripts.json` sidecar, or a video linked to a post (eroscripts:sync)
//   <root>/FapTap/<creator>/<scene>/       a `faptap.json` sidecar
//   <root>/Other/<creator>/<scene>/        everything else
//
// and work out the library sources that follow. Pure: the CLI walks the
// disk, moves the folders and rewrites the paths (remapPaths).

import path from 'node:path';

export const SOURCE_FOLDERS = Object.freeze(['EroScripts', 'FapTap', 'Other']);

// exFAT and APFS compare names case- and normalization-insensitively.
const fold = (p) => path.resolve(p).normalize('NFC').toLowerCase();
const inside = (p, dir) => fold(p).startsWith(fold(dir) + path.sep);

/**
 * The source folder a scene belongs in: where its files came from. A scout
 * sidecar decides (EroScripts first when a folder holds both, as in the dedupe
 * keep order); a scene without one goes to EroScripts when a video is linked
 * to a post (eroscripts:sync), else Other. A FapTap download later linked to
 * the same script's post stays FapTap.
 *
 * @param {{files: string[], linked?: boolean}} scene file names in the scene folder; linked: a video has an EroScripts link
 */
export function sceneSource({ files, linked = false }) {
  if (files.includes('eroscripts.json')) return 'EroScripts';
  if (files.includes('faptap.json')) return 'FapTap';
  return linked ? 'EroScripts' : 'Other';
}

/**
 * Plan the moves. Each scene folder keeps its path below its source
 * (`<creator>/<scene>`), under `<root>/<source folder>`. A scene whose target
 * already exists, that two scenes would share, or that sits loose in a source
 * root is left where it is and reported.
 *
 * @param {object} o
 * @param {{dir: string, sourcePath: string, files: string[], linked?: boolean}[]} o.scenes
 * @param {string} o.root folder the source folders go in
 * @param {(p: string) => boolean} [o.exists] does a path exist on disk
 * @returns {{moves: {from: string, to: string, files: []}[], conflicts: {dir: string, to?: string, reason: string}[],
 *   counts: Object<string, number>}}
 */
export function planReorganize({ scenes, root, exists = () => false }) {
  const moves = [];
  const conflicts = [];
  const counts = Object.fromEntries(SOURCE_FOLDERS.map((f) => [f, 0]));
  const taken = new Map();
  const rel = (s) => path.relative(s.sourcePath, s.dir);
  const own = (s) => rel(s) && !rel(s).startsWith('..');
  // A scene folder inside another scene folder moves along with it. Videos
  // loose in a source root have no folder to move and contain nothing.
  const dirs = scenes.filter(own).map((s) => s.dir);
  const top = scenes.filter((s) => !dirs.some((d) => inside(s.dir, d)));
  for (const s of top) {
    const folder = sceneSource(s);
    counts[folder]++;
    if (!own(s)) { conflicts.push({ dir: s.dir, reason: 'not in a folder of its own under its source' }); continue; }
    const to = path.join(root, folder, rel(s));
    if (fold(to) === fold(s.dir)) continue;
    if (taken.has(fold(to))) { conflicts.push({ dir: s.dir, to, reason: `same target as ${taken.get(fold(to))}` }); continue; }
    if (exists(to)) { conflicts.push({ dir: s.dir, to, reason: 'target already exists' }); continue; }
    taken.set(fold(to), s.dir);
    moves.push({ from: s.dir, to, files: [] });
  }
  return { moves, conflicts, counts };
}

/**
 * The library sources after the move: one per source folder that holds
 * scenes (an existing source with that path keeps its id and settings), then
 * every other source, minus the old sources the move emptied.
 *
 * @param {{id: string, name: string, path: string, enabled?: boolean}[]} sources
 * @param {object} o
 * @param {string} o.root
 * @param {string[]} o.folders source folders that hold scenes after the move
 * @param {string[]} o.emptied paths of old sources left without scenes
 * @param {() => string} o.newId
 */
export function sourcesAfter(sources, { root, folders, emptied, newId }) {
  const gone = new Set(emptied.map(fold));
  const made = SOURCE_FOLDERS.filter((f) => folders.includes(f)).map((name) => {
    const p = path.join(root, name);
    const had = sources.find((s) => fold(s.path) === fold(p));
    return had ? { ...had, name, enabled: true } : { id: newId(), name, path: p, enabled: true };
  });
  const madeKeys = new Set(made.map((s) => fold(s.path)));
  return [...made, ...sources.filter((s) => !madeKeys.has(fold(s.path)) && !gone.has(fold(s.path)))];
}

/** The moves that put a reorganize run back (`--undo`). */
export function reverseMoves(moves) {
  return moves.map((m) => ({ from: m.to, to: m.from, files: [] })).reverse();
}
