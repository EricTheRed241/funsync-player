'use strict';

// Library scan: walk the source folders and pair every video with its
// funscript, variants and subtitle. Shared by the `scan-directory` IPC
// handler and by CLI tools (scripts/eroscripts-sync.mjs) so both see the
// same pairing. No Electron dependency.

const path = require('path');
const fs = require('fs');
const { underscoreVariant } = require('./script-variant');

// macOS Finder AppleDouble resource-fork sidecars (`._Video.mp4`).
// Leading basename prefix only — `foo._bar.mp4` is a real file and is kept.
// Never delete these; skip them at ingest.
function isAppleDoubleSidecar(name) {
  return typeof name === 'string' && name.startsWith('._');
}

/**
 * Hand the main process's event loop back for one turn.
 *
 * The library scan does the directory walk asynchronously, but then runs a
 * dozen SYNCHRONOUS passes over the full entry list. On a large or networked
 * library that is one long block on the main thread, which freezes everything
 * the main process owns — including modal dialogs, which is how Dave hit it
 * (2026-08-11: "everything just freezes, on the modal as well").
 *
 * `setImmediate` yields after I/O callbacks, so pending IPC and window paints
 * get serviced between passes. It converts one long freeze into several short
 * ones; it does not make the scan faster, and it cannot help WITHIN a single
 * pass over a huge list — see the chunked loop below for that.
 */
const yieldTick = () => new Promise((resolve) => setImmediate(resolve));

const SILENT_LOG = { info() {}, warn() {} };

/**
 * @param {string|string[]} dirPathOrPaths one source folder or several
 * @param {Record<string,string>} [sourceMap] source path → display name
 * @param {{log?: {info: Function, warn: Function}}} [opts]
 * @returns {Promise<{videos: object[], unmatchedFunscripts: object[],
 *   unmatchedSubtitles: object[], allFunscripts: object[], failedPaths: string[]}>}
 */
async function scanLibrary(dirPathOrPaths, sourceMap, { log = SILENT_LOG } = {}) {
  // Accept a single path or array of paths (multi-source)
  // sourceMap: optional { path: sourceName } mapping for VR content server grouping
  const dirPaths = Array.isArray(dirPathOrPaths) ? dirPathOrPaths : [dirPathOrPaths];
  const dirPath = dirPaths[0]; // for backward compat logging
  const _sourceMap = sourceMap || {};
  const VIDEO_EXTS = ['.mp4', '.m4v', '.mkv', '.webm', '.avi', '.mov', '.mp3', '.wav', '.ogg', '.flac', '.m4a'];
  const FUNSCRIPT_EXT = '.funscript';
  const SUBTITLE_EXTS = ['.srt', '.vtt'];
  const AXIS_SUFFIXES = new Set(['surge','sway','twist','roll','pitch','vib','lube','pump','suction','valve']);

  // Canonicalise a path for cross-source dedup: forward slashes, lowercased
  // drive letter. Matches renderer/js/path-utils.js::canonicalPath so the
  // settings overlap warning and the scan dedup agree on identity.
  const canonicalise = (p) => {
    if (!p) return '';
    let out = String(p).replace(/\\/g, '/');
    while (out.length > 1 && out.endsWith('/')) out = out.slice(0, -1);
    if (/^[A-Za-z]:/.test(out)) out = out[0].toLowerCase() + out.slice(1);
    return out;
  };

  let entries = [];
  const seenPaths = new Set(); // canonical full paths — drops dupes when sources overlap
  const failedPaths = [];
  let rawEntryCount = 0;
  const scanStart = Date.now();
  for (const dp of dirPaths) {
    if (!dp) continue;
    try {
      const dirEntries = await fs.promises.readdir(dp, { withFileTypes: true, recursive: true });
      rawEntryCount += dirEntries.length;
      for (const entry of dirEntries) {
        if (isAppleDoubleSidecar(entry.name)) continue;
        const parent = entry.parentPath || entry.path || dp;
        const full = canonicalise(path.join(parent, entry.name));
        if (seenPaths.has(full)) continue;
        seenPaths.add(full);
        entries.push(entry);
      }
    } catch (err) {
      log.warn(`[Library] Failed to scan ${dp}: ${err.message}`);
      failedPaths.push(dp);
    }
  }
  const dupesDropped = rawEntryCount - entries.length;
  if (dupesDropped > 0) {
    log.info(`[Library] Scanned ${entries.length} unique entries (${dupesDropped} duplicates dropped from overlapping sources) in ${Date.now() - scanStart}ms from ${dirPaths.length} source(s)`);
  } else {
    log.info(`[Library] Scanned ${entries.length} entries in ${Date.now() - scanStart}ms from ${dirPaths.length} source(s)`);
  }

  // Normalize a basename for matching: lowercase, replace separators with spaces, collapse
  const normalizeName = (name) => name.toLowerCase().replace(/[_.\-]/g, ' ').replace(/\s+/g, ' ').trim();

  // Helper: get the full path for a recursive dirent
  const entryPath = (entry) => {
    // Node recursive dirent: entry.parentPath or entry.path contains the parent directory
    const parent = entry.parentPath || entry.path || dirPath;
    return path.join(parent, entry.name);
  };

  // Helper: get the directory of an entry (for same-directory matching)
  const entryDir = (entry) => {
    return entry.parentPath || entry.path || dirPath;
  };

  // Pre-compute the set of video basenames (full, including any
  // parenthetical) so funscript classification can distinguish a
  // real-title parenthetical from a variant suffix. Example:
  //   "Title (Nude).mp4" + "Title (Nude).funscript"  → the "(Nude)" is
  //      part of the title; the funscript is that video's PRIMARY.
  //   "Title.mp4" + "Title (Soft).funscript"         → no "Title (Soft)"
  //      video exists; "(Soft)" is a variant label of "Title".
  // Without this, "Title (Nude).funscript" was always classified as a
  // "(Nude)" variant of "Title" and absorbed into "Title.mp4", leaving
  // "Title (Nude).mp4" script-less and hidden under the Matched tab.
  const videoBaseLocal = new Set();  // dir + '\0' + normalizedBase
  const videoBaseGlobal = new Set(); // normalizedBase
  const videoStemsByDir = new Map(); // dir → raw basenames, for `Title_label` variants
  await yieldTick();
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    const vext = path.extname(entry.name).toLowerCase();
    if (!VIDEO_EXTS.includes(vext)) continue;
    const vstem = path.basename(entry.name, vext);
    const vbase = normalizeName(vstem);
    const vdir = entryDir(entry);
    videoBaseLocal.add(vdir + '\0' + vbase);
    videoBaseGlobal.add(vbase);
    if (!videoStemsByDir.has(vdir)) videoStemsByDir.set(vdir, []);
    videoStemsByDir.get(vdir).push(vstem);
  }

  // Collect all funscripts with variant/axis classification
  const funscriptList = [];
  await yieldTick();
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    const ext = path.extname(entry.name).toLowerCase();
    if (ext !== FUNSCRIPT_EXT) continue;

    const nameNoExt = path.basename(entry.name, ext);
    const fullPath = entryPath(entry);

    // Check for axis suffix: "video.vib" -> suffix "vib"
    const dotIdx = nameNoExt.lastIndexOf('.');
    const dotSuffix = dotIdx >= 0 ? nameNoExt.slice(dotIdx + 1).toLowerCase() : null;
    const isAxis = dotSuffix && AXIS_SUFFIXES.has(dotSuffix);

    // Check for parenthesized variant: "video (Soft)" -> label "Soft"
    const parenMatch = nameNoExt.match(/^(.+?)\s*\(([^)]+)\)\s*$/);

    let videoBase, variantLabel;
    let isAmbiguousDotVariant = false;
    // Runs before the dot rule: `Animex 3.0.1_hard` would otherwise be
    // split at its last dot and demoted to a primary with no video.
    const uVariant = (isAxis || parenMatch) ? null
      : underscoreVariant(nameNoExt, videoStemsByDir.get(entryDir(entry)));
    if (isAxis) {
      let stem = nameNoExt.slice(0, dotIdx);
      if (stem.toLowerCase().endsWith('.generated')) stem = stem.slice(0, -'.generated'.length);
      videoBase = normalizeName(stem);
      variantLabel = null; // axis, not a variant
    } else if (parenMatch) {
      const fullBase = normalizeName(nameNoExt);
      const fsDir = entryDir(entry);
      if (videoBaseLocal.has(fsDir + '\0' + fullBase) || videoBaseGlobal.has(fullBase)) {
        // A video whose real title includes the parenthetical exists
        // (e.g. "Title (Nude).mp4"). The parens are part of the title,
        // not a variant suffix — treat this funscript as that video's
        // primary so the video isn't left script-less and hidden.
        videoBase = fullBase;
        variantLabel = null;
      } else {
        // Parenthesized variant — `"Title (Soft).funscript"` with only
        // "Title.mp4" present. Unambiguous user intent; treat as variant.
        videoBase = normalizeName(parenMatch[1]);
        variantLabel = parenMatch[2].trim();
      }
    } else if (uVariant) {
      videoBase = normalizeName(uVariant.stem);
      variantLabel = uVariant.label;
    } else if (dotSuffix && dotIdx > 0) {
      // AMBIGUOUS — could be a real variant ("video.intense.funscript")
      // OR just a filename that happens to contain a dot ("S01.E03.funscript",
      // "Title.2024.funscript"). We can't tell from the name in isolation.
      // Classify provisionally as a variant and let the post-pass below
      // demote it to primary if no sibling funscript exists with the
      // dot-stripped base (i.e., nothing for it to be a "variant of").
      videoBase = normalizeName(nameNoExt.slice(0, dotIdx));
      variantLabel = dotSuffix;
      isAmbiguousDotVariant = true;
    } else {
      videoBase = normalizeName(nameNoExt);
      variantLabel = null; // default/primary
    }

    funscriptList.push({
      name: entry.name,
      path: fullPath,
      dir: entryDir(entry),
      videoBase,
      variantLabel,
      isAxis,
      axisSuffix: isAxis ? dotSuffix : null,
      // Set for funscripts whose variant-ness was inferred from a dot
      // in the filename. Post-pass below demotes to primary when no
      // matching primary sibling exists.
      isAmbiguousDotVariant,
      // Cache the full-name normalisation for the demotion path so we
      // don't recompute it after the maps are built.
      fullNormalisedBase: normalizeName(nameNoExt),
      _used: false,
    });
  }

  // Post-pass: demote ambiguous dot-variants to primaries when no
  // sibling exists with the dot-stripped base. This rescues filenames
  // like "S01.E03.funscript" and "Title.2024.funscript" that the
  // classifier above would otherwise route to a non-existent base.
  // A sibling counts if it's a non-axis funscript in the same dir
  // (preferred) or anywhere (fallback) whose `videoBase` matches the
  // ambiguous variant's `videoBase`.
  const siblingBasesLocal = new Set();
  const siblingBasesGlobal = new Set();
  await yieldTick();
  for (const fs of funscriptList) {
    if (fs.isAxis || fs.isAmbiguousDotVariant) continue;
    siblingBasesLocal.add(fs.dir + '\0' + fs.videoBase);
    siblingBasesGlobal.add(fs.videoBase);
  }
  await yieldTick();
  for (const fs of funscriptList) {
    if (!fs.isAmbiguousDotVariant) continue;
    const localKey = fs.dir + '\0' + fs.videoBase;
    if (siblingBasesLocal.has(localKey) || siblingBasesGlobal.has(fs.videoBase)) {
      // Confirmed variant — a real primary exists. Clear the flag so
      // downstream code can treat the classification as settled.
      fs.isAmbiguousDotVariant = false;
      continue;
    }
    // Demote to primary using the full normalised name. "Title.2024"
    // becomes its own thing, matching "Title.2024.mp4".
    fs.videoBase = fs.fullNormalisedBase;
    fs.variantLabel = null;
    fs.isAmbiguousDotVariant = false;
  }

  // Build two maps: same-directory (preferred) and global (fallback)
  const funscriptMapLocal = new Map(); // dir+base → fs
  const funscriptMapGlobal = new Map(); // base → fs
  await yieldTick();
  for (const fs of funscriptList) {
    const localKey = fs.dir + '\0' + fs.videoBase;
    const globalKey = fs.videoBase;
    if (!fs.isAxis && !fs.variantLabel) {
      if (!funscriptMapLocal.has(localKey)) funscriptMapLocal.set(localKey, fs);
      if (!funscriptMapGlobal.has(globalKey)) funscriptMapGlobal.set(globalKey, fs);
    }
  }
  // Fallback: if no default variant, use first matching
  await yieldTick();
  for (const fs of funscriptList) {
    const localKey = fs.dir + '\0' + fs.videoBase;
    const globalKey = fs.videoBase;
    if (!fs.isAxis) {
      if (!funscriptMapLocal.has(localKey)) funscriptMapLocal.set(localKey, fs);
      if (!funscriptMapGlobal.has(globalKey)) funscriptMapGlobal.set(globalKey, fs);
    }
  }

  // Collect subtitle basenames — local (same-dir) and global maps
  const subtitleMapLocal = new Map();
  const subtitleMapGlobal = new Map();
  await yieldTick();
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    const ext = path.extname(entry.name).toLowerCase();
    if (SUBTITLE_EXTS.includes(ext)) {
      const baseName = normalizeName(path.basename(entry.name, ext));
      const dir = entryDir(entry);
      const localKey = dir + '\0' + baseName;
      const sub = { name: entry.name, path: entryPath(entry), dir, _used: false };
      if (!subtitleMapLocal.has(localKey)) subtitleMapLocal.set(localKey, sub);
      if (!subtitleMapGlobal.has(baseName)) subtitleMapGlobal.set(baseName, sub);
    }
  }

  // Build video list with funscript + subtitle + variant pairing
  const videos = [];
  await yieldTick();
  for (const entry of entries) {
    if (!entry.isFile()) continue;
    const ext = path.extname(entry.name).toLowerCase();
    if (!VIDEO_EXTS.includes(ext)) continue;

    const baseName = normalizeName(path.basename(entry.name, ext));
    const dir = entryDir(entry);
    const localKey = dir + '\0' + baseName;

    // Funscript: prefer same directory, fall back to global
    const fsEntry = funscriptMapLocal.get(localKey) || funscriptMapGlobal.get(baseName) || null;
    const funscriptPath = fsEntry ? fsEntry.path : null;
    if (fsEntry) fsEntry._used = true;

    // Subtitle: prefer same directory, fall back to global
    const subEntry = subtitleMapLocal.get(localKey) || subtitleMapGlobal.get(baseName) || null;
    const subtitlePath = subEntry ? subEntry.path : null;
    if (subEntry) subEntry._used = true;

    // Collect variants: same directory first, then global matches
    const variants = [];
    const seenVariantPaths = new Set();
    // Pass 1: same directory
    for (const fs of funscriptList) {
      if (fs.videoBase !== baseName || fs.isAxis || fs.dir !== dir) continue;
      fs._used = true;
      seenVariantPaths.add(fs.path);
      variants.push({ label: fs.variantLabel || 'Default', path: fs.path, name: fs.name });
    }
    // Pass 2: global (other directories) — only if not already found locally
    if (variants.length === 0) {
      for (const fs of funscriptList) {
        if (fs.videoBase !== baseName || fs.isAxis || seenVariantPaths.has(fs.path)) continue;
        fs._used = true;
        variants.push({ label: fs.variantLabel || 'Default', path: fs.path, name: fs.name });
      }
    }
    // Sort: Default first, then alphabetical
    variants.sort((a, b) => {
      if (a.label === 'Default') return -1;
      if (b.label === 'Default') return 1;
      return a.label.localeCompare(b.label);
    });

    // Resolve source name from sourceMap (match longest prefix)
    const fullPath = entryPath(entry);
    let sourceName = '';
    for (const [srcPath, srcName] of Object.entries(_sourceMap)) {
      if (fullPath.startsWith(srcPath) && srcPath.length > sourceName.length) {
        sourceName = srcName;
      }
    }

    videos.push({
      name: entry.name,
      path: fullPath,
      ext,
      hasFunscript: funscriptPath !== null,
      funscriptPath,
      hasSubtitle: subtitlePath !== null,
      subtitlePath,
      // Pass ALL detected variants through, even a single one. The
      // renderer decides whether to SHOW the switcher (it hides the
      // dropdown when the combined auto+manual count is < 2), and it
      // combines these with manually-associated variants — so dropping a
      // lone auto variant here used to hide the switcher for a video that
      // had 1 auto script + 1 manual variant (2 real scripts). It also
      // preserved the real label/path instead of a synthesised "Default".
      variants,
      sourceName: sourceName || path.basename(path.dirname(fullPath)) || 'Library',
      // `dateAdded` is populated in a batched stat pass below. Using `mtimeMs`
      // as a pragmatic "date added" proxy — cross-platform reliable (unlike
      // birthtimeMs on Linux ext4). Users typically want "recently appeared
      // in my folder", which matches mtime for fresh-dropped files.
      dateAdded: 0,
    });
  }

  // Batched stat pass to fill `dateAdded`. Runs all stats in parallel
  // (libuv's thread pool naturally caps concurrency). Typical cost for
  // 1000 videos: ~10-50ms on SSD, ~100-500ms on HDD. Failures fall back
  // to 0 which sorts to the "oldest" end of the list.
  await Promise.all(videos.map(async (v) => {
    try {
      const st = await fs.promises.stat(v.path);
      v.dateAdded = st.mtimeMs || 0;
    } catch {
      v.dateAdded = 0;
    }
  }));

  // Fuzzy match pass: pair unmatched videos with high-confidence funscript matches
  // Uses token overlap (Jaccard index) — same logic as renderer fuzzy-match.js
  const fuzzyTokenize = (s) => s.toLowerCase().replace(/[_.\-()[\]{}]/g, ' ').replace(/\s+/g, ' ').trim().split(' ').filter(Boolean);
  const fuzzyScore = (a, b) => {
    const tokA = new Set(fuzzyTokenize(a));
    const tokB = new Set(fuzzyTokenize(b));
    if (tokA.size === 0 || tokB.size === 0) return 0;
    let inter = 0;
    for (const t of tokA) if (tokB.has(t)) inter++;
    const union = new Set([...tokA, ...tokB]).size;
    return Math.round((inter / union) * 100);
  };

  for (const video of videos) {
    if (video.hasFunscript) continue; // already matched
    const videoBase = path.basename(video.name, path.extname(video.name));
    let bestFs = null;
    let bestScore = 0;
    for (const fs of funscriptList) {
      if (fs._used || fs.isAxis) continue;
      const fsBase = path.basename(fs.name, '.funscript');
      const score = fuzzyScore(videoBase, fsBase);
      if (score > bestScore) { bestScore = score; bestFs = fs; }
    }
    if (bestFs && bestScore >= 98) {
      video.hasFunscript = true;
      video.funscriptPath = bestFs.path;
      video._fuzzyMatched = true;
      bestFs._used = true;

      // Also collect variants for this fuzzy match
      const fuzzyVariants = [];
      const seenPaths = new Set();
      for (const fs of funscriptList) {
        if (fs.isAxis) continue;
        const fsBase = path.basename(fs.name, '.funscript');
        const s = fuzzyScore(videoBase, fsBase);
        if (s >= 98 && !seenPaths.has(fs.path)) {
          seenPaths.add(fs.path);
          fuzzyVariants.push({ label: fs.variantLabel || 'Default', path: fs.path, name: fs.name });
        }
      }
      if (fuzzyVariants.length > 1) {
        fuzzyVariants.sort((a, b) => a.label === 'Default' ? -1 : b.label === 'Default' ? 1 : a.label.localeCompare(b.label));
        video.variants = fuzzyVariants;
      }
    }
  }

  // Axis companions belong to the stroke script they are named after
  // (`Title_hardcore.generated.twist` → `Title_hardcore.funscript`), or to a
  // video of that name when the scene ships axes only. Without this pass
  // every axis file of a paired scene was listed as unmatched.
  const pairedStems = new Set();
  for (const fs of funscriptList) {
    if (fs._used && !fs.isAxis) pairedStems.add(fs.dir + '\0' + fs.fullNormalisedBase);
  }
  for (const fs of funscriptList) {
    if (!fs.isAxis) continue;
    const key = fs.dir + '\0' + fs.videoBase;
    if (pairedStems.has(key) || videoBaseLocal.has(key)) fs._used = true;
  }

  // Collect unmatched funscripts (not paired to any video)
  const unmatchedFunscripts = [];
  await yieldTick();
  for (const fs of funscriptList) {
    if (!fs._used) {
      unmatchedFunscripts.push({ name: fs.name, path: fs.path });
    }
  }
  unmatchedFunscripts.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));

  // Collect unmatched subtitles
  const unmatchedSubtitles = [];
  for (const subEntry of subtitleMapGlobal.values()) {
    if (!subEntry._used) {
      unmatchedSubtitles.push({ name: subEntry.name, path: subEntry.path });
    }
  }
  unmatchedSubtitles.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));

  // Collect all funscripts for multi-axis dropdowns
  const allFunscripts = [];
  await yieldTick();
  for (const fs of funscriptList) {
    allFunscripts.push({ name: fs.name, path: fs.path });
  }
  allFunscripts.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));

  // Sort alphabetically
  videos.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }));
  log.info(`[Library] Result: ${videos.length} videos, ${unmatchedFunscripts.length} unmatched fs, ${allFunscripts.length} total fs (${Date.now() - scanStart}ms total)`);

  return { videos, unmatchedFunscripts, unmatchedSubtitles, allFunscripts, failedPaths };
}

module.exports = { scanLibrary, isAppleDoubleSidecar };
