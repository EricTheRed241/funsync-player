// Pair stroke funscripts with videos for upgrade-multi-axis.mjs, using the
// same rules as the library scan so generated companions land where the
// player looks for them:
//
//   Title.mp4 + Title.funscript            → companions named after the video
//   Title.mp4 + Title (Soft).funscript     → variant, companions named after
//   Title.mp4 + Title_hardcore.funscript     the variant script
//
// A script found by walking a folder that pairs with no video is skipped:
// its companions would never play, and copies such as `Title.funscript`
// beside `Title_4k.mp4` + `Title_4k.funscript` only produced duplicates.
// A script named explicitly on the command line is always upgraded.

import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { underscoreVariant } = require('../electron/script-variant.js');

function normStem(name) {
  return name.toLowerCase().replace(/[_.\-]/g, ' ').replace(/\s+/g, ' ').trim();
}

const stemOf = (p) => path.basename(p, path.extname(p));

/**
 * @param {string[]} videos
 * @param {string[]} scripts — stroke scripts (no axis companions)
 * @param {Set<string>} [explicit] — scripts named on the command line
 * @returns {{
 *   jobs: Array<{video: string|null, script: string}>,
 *   unmatchedVideos: string[],
 *   orphanScripts: string[],
 * }} `video` is set only when companions should take the video's name.
 */
export function pairUpgradeJobs(videos, scripts, explicit = new Set()) {
  if (videos.length === 1 && scripts.length === 1) {
    return { jobs: [{ video: videos[0], script: scripts[0] }], unmatchedVideos: [], orphanScripts: [] };
  }
  const byStem = new Map();
  for (const script of scripts) {
    const stem = normStem(stemOf(script));
    if (!byStem.has(stem)) byStem.set(stem, []);
    byStem.get(stem).push(script);
  }
  const used = new Set();
  const jobs = [];
  const unmatchedVideos = [];
  for (const video of videos) {
    const hit = (byStem.get(normStem(stemOf(video))) || []).find((s) => !used.has(s));
    if (!hit) {
      unmatchedVideos.push(video);
      continue;
    }
    used.add(hit);
    jobs.push({ video, script: hit });
  }

  const stemsByDir = new Map();
  for (const video of videos) {
    const dir = path.dirname(video);
    if (!stemsByDir.has(dir)) stemsByDir.set(dir, []);
    stemsByDir.get(dir).push(stemOf(video));
  }
  const orphanScripts = [];
  for (const script of scripts) {
    if (used.has(script)) continue;
    const name = stemOf(script);
    const dirStems = stemsByDir.get(path.dirname(script)) || [];
    const paren = name.match(/^(.+?)\s*\(([^)]+)\)\s*$/);
    const isVariant = (paren && dirStems.some((s) => normStem(s) === normStem(paren[1])))
      || underscoreVariant(name, dirStems) !== null;
    if (isVariant || explicit.has(script)) jobs.push({ video: null, script });
    else orphanScripts.push(script);
  }
  return { jobs, unmatchedVideos, orphanScripts };
}
