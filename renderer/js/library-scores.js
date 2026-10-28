// Preference score of every library scene: the scouts' ranking
// (preference-score.js, RANKING in content-preferences.js) applied to what
// the library knows about the scene, so the library and the scouting agree.
//
// Inputs per video:
//   • vr: the backend's VR decision (`GET /api/media/vr-flags`, through
//     vr-detect.js, which also applies VR Format and Mark as VR/Flat)
//   • multi-axis: axis kind `handmade` from `GET /api/media/axis-flags`
//   • tags: the post tags in `library.scriptTags` (EroScripts / FapTap),
//     scored with the tag scores (TAG_SCORE_GROUPS defaults plus the user's
//     changes in `library.tagScores`, set from the web remote); 2D hentai,
//     monsters, feet and FFM also from the title (the file name), as the
//     scouts judge a post
//   • artist in library: the video's artist folder (artist-playlists.js) is
//     a favourite or holds at least MIN_ARTIST_VIDEOS videos
//   • likes: from the post record when it holds them, else 0
// Scenes without post tags still get a score from VR, axes and the title.
//
// Derived and recomputed after each scan, when the backend's VR / axis
// decisions arrive, and when a tag score changes or the web remote asks
// (Recalculate scores); kept in `library.preferenceScores` ({videoPath: score},
// written only when it changed) so sorting works right after a start, and
// sent to the backend for the web remote and the headsets.

import { artistOf } from './artist-playlists.js';
import { MIN_ARTIST_VIDEOS } from './content-preferences.js';
import { candidateScore, effectiveTagScores, tagScoreInfo } from './preference-score.js';
import { SCRIPT_TAGS_KEY } from './script-tags.js';

export const PREFERENCE_SCORES_KEY = 'library.preferenceScores';
export const TAG_SCORES_KEY = 'library.tagScores';

/**
 * The tag-score part of a backend registration (`/api/media/register`):
 * every tag's score, defaults included (`tagScores`), and the groups and
 * built-in weights the web remote's Tags view shows (`tagScoreInfo`).
 *
 * @param {object} data DataService (get)
 */
export function tagScoreRegistration(data) {
  return { tagScores: effectiveTagScores(data.get(TAG_SCORES_KEY)), tagScoreInfo: tagScoreInfo() };
}

const fold = (s) => String(s || '').normalize('NFC').trim().toLocaleLowerCase();
const titleOf = (p) => (String(p).split(/[\\/]/).pop() || '').replace(/\.[^/.]+$/, '');

/**
 * One library scene as a scout candidate, for candidateScore.
 *
 * @param {object} o
 * @param {string} [o.title] the scene's name
 * @param {string[]} [o.tags] its post tags
 * @param {number} [o.likes]
 * @param {boolean} [o.vr]
 * @param {'handmade'|'generated'|'single'|null} [o.axisKind]
 * @param {boolean} [o.artistInLibrary]
 */
export function sceneCandidate({ title = '', tags = [], likes = 0, vr = false, axisKind = null, artistInLibrary = false }) {
  const t = [...new Set((Array.isArray(tags) ? tags : []).filter((x) => typeof x === 'string').map((x) => x.trim().toLowerCase()))];
  return {
    title,
    tags: t,
    likes: Number(likes) || 0,
    vr: !!vr,
    axisType: axisKind === 'handmade' ? 'multi' : 'single',
    artistInLibrary: !!artistInLibrary,
  };
}

/**
 * Score of every video.
 *
 * A scripted video the backend hasn't decided the axes of yet (left out of
 * the axis flags while it reads embedded axes) keeps its previous score, so
 * a hand-made scene doesn't drop for a minute after every start.
 *
 * @param {object} o
 * @param {{path: string, hasFunscript?: boolean}[]} o.videos
 * @param {string[]} [o.sourcePaths] library source folders
 * @param {Object<string, {tags?: string[], likes?: number}>} [o.scriptTags] `library.scriptTags`
 * @param {(path: string) => boolean} [o.vrOf]
 * @param {Object<string, string>} [o.axisFlags] videoPath → axis kind
 * @param {string[]} [o.favourites] favourite artist names
 * @param {Object<string, number>} [o.previous] the stored scores
 * @param {Object<string, number>} [o.tagScores] `library.tagScores` (tag → score)
 * @returns {Object<string, number>} videoPath → score
 */
export function planPreferenceScores({ videos = [], sourcePaths = [], scriptTags = {}, vrOf = () => false, axisFlags = {}, favourites = [], previous = {}, tagScores = {} }) {
  const artists = new Map(videos.map((v) => [v.path, artistOf(v.path, sourcePaths)]));
  const counts = new Map();
  for (const a of artists.values()) if (a) counts.set(fold(a), (counts.get(fold(a)) || 0) + 1);
  const favouriteKeys = new Set(favourites.map(fold).filter(Boolean));

  const scores = {};
  for (const v of videos) {
    const kind = axisFlags[v.path];
    if (v.hasFunscript && kind == null && typeof previous[v.path] === 'number') {
      scores[v.path] = previous[v.path];
      continue;
    }
    const artist = artists.get(v.path);
    const key = artist ? fold(artist) : null;
    const record = scriptTags[v.path] || {};
    scores[v.path] = candidateScore(sceneCandidate({
      title: titleOf(v.path),
      tags: record.tags,
      likes: record.likes,
      vr: vrOf(v.path),
      axisKind: kind,
      artistInLibrary: !!key && (favouriteKeys.has(key) || counts.get(key) >= MIN_ARTIST_VIDEOS),
    }), tagScores);
  }
  return scores;
}

/** True when two score maps hold the same scores for the same videos. */
export function sameScores(a = {}, b = {}) {
  const ka = Object.keys(a);
  return ka.length === Object.keys(b).length && ka.every((k) => b[k] === a[k]);
}

/**
 * Score the library's videos and store the scores when they changed.
 *
 * @param {object} data DataService (get, set)
 * @param {{path: string, hasFunscript?: boolean}[]} videos
 * @param {{vrOf?: (path: string) => boolean, axisFlags?: Object<string, string>, favourites?: string[]}} inputs
 * @returns {{scores: Object<string, number>, changed: boolean}}
 */
export function applyPreferenceScores(data, videos, { vrOf, axisFlags, favourites } = {}) {
  const previous = data.get(PREFERENCE_SCORES_KEY) || {};
  const scores = planPreferenceScores({
    videos,
    sourcePaths: (data.get('library.sources') || []).map((s) => s?.path).filter(Boolean),
    scriptTags: data.get(SCRIPT_TAGS_KEY) || {},
    vrOf,
    axisFlags,
    favourites,
    previous,
    tagScores: data.get(TAG_SCORES_KEY) || {},
  });
  const changed = !sameScores(scores, previous);
  if (changed) data.set(PREFERENCE_SCORES_KEY, scores);
  return { scores, changed };
}
