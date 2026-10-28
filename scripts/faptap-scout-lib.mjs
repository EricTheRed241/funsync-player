// FapTap Discovery scout: find faptap.net videos worth adding to the
// library, pick the stream to download, and turn FapTap's script into a
// funscript.
//
// Logic for `npm run faptap:scout` (scripts/faptap-scout.mjs), kept apart
// from the CLI so it can be tested without faptap.net.
//
// faptap.net has an open JSON API (no login): `/api/videos` lists videos
// newest first with their tags and description, `/api/videos/<id>` adds the
// script, `/api/assets/scripts/<id>.csv` is the script as `ms,pos` lines, and
// `/api/videos/<id>/bunny-sources` lists HLS streams per quality. Every
// FapTap script is single-axis (a script has one url, no axes), so every
// scene gets generated twist / roll / pitch companions after download.
//
// Rules: those of the EroScripts scout (scripts/eroscripts-scout-lib.mjs)
// except the multi-axis one, narrowed to what the user wants from FapTap.
// The values are in renderer/js/content-preferences.js.
//   • Never excluded content (tags, title or description), nor FURRY_TAGS.
//     Monsters and feet (tags or title, as on EroScripts) come in, ranked
//     lower.
//   • Animated, unless a favourite: an animation tag, no live-action tag,
//     and no FapTap model / pornstar named in the title or listed as the
//     video's performer (FapTap tags are free text; live videos get
//     animation tags too). A music video by a creator already in the
//     library is taken too; a library folder named after a genre (`Cock
//     Hero`, `PMV`) is no creator. Live action by a favourite (title,
//     uploader, performer or tag; scripts/scout-live.mjs) is taken; other
//     live action waits for the live quota.
//   • No audio roleplays.
//   • Focus, except for favourites: at least FAPTAP_FOCUS_MIN of the
//     FAPTAP_FOCUS themes.
//   • Videos the user picks by id (`--add`) skip the animation and focus
//     rules; the exclusions, content rules, audio, length and library
//     checks stay.
//   • Sex required (as on EroScripts; picks too), at least MIN_VIDEO_MS
//     long. `--fetch` and every scan drop saved candidates that now break
//     the content rules (stillWanted).
//   • Ranking as on EroScripts (candidateScore; monsters, feet and 2D
//     hentai only rank lower), saved lists scored again under it.
//   • Skip videos the library or the EroScripts scout list already has.
//   • Stream: the best Bunny HLS stream within the flat / VR height cap;
//     none below MIN_STREAM_HEIGHT.
//
// Tags: FapTap's free-text tags become the tags the library already uses
// (EroScripts slugs, which the auto categories and collections match):
// `Cum Inside Pussy` → `creampie`, `Reverse Cowgirl Position` →
// `reverse-cowgirl`, `Animated 3d` → `3d`, `Mercy (Overwatch)` → also
// `overwatch`.

import {
  candidateScore, contentRuleBreak, hasExcludedText, isBlockedCreator, isCreatorLabel, isHentaiOnly, isSexual, normName, sameVideo, sceneTitle,
  titleArtist,
} from './eroscripts-scout-lib.mjs';
import {
  ANIMATION_TAGS, AUDIO_TITLE, FAPTAP_ANIMATION_TAGS, FAPTAP_FOCUS, FAPTAP_FOCUS_MIN, FLAT_MAX_HEIGHT, FURRY_TAGS, LIVE_TAGS,
  MIN_STREAM_HEIGHT, MIN_VIDEO_MS, MUSIC_TAGS, PREFERRED_TAGS, VR_MAX_HEIGHT, isAudioRoleplayTag,
} from '../renderer/js/content-preferences.js';
import { favouriteIn } from './scout-live.mjs';
import { firstDuplicate, titleWords } from './scene-dedupe-lib.mjs';

export const FAPTAP_ORIGIN = 'https://faptap.net';
export const SIDECAR = 'faptap.json';
/** Bump when the screening rules change, so every video is judged again. */
export const FAPTAP_RULES_VERSION = 6;
/** The one feed: every video, newest first. */
export const FEED = 'all';

// FapTap spelling → the tag the library already uses.
const TAG_ALIASES = Object.fromEntries(Object.entries({
  animation: ['animated', 'animated-cartoon', 'animated-porn', 'animation-sex', 'animation-w-sound'],
  '3d': ['animated-3d', '3d-animation'],
  '2d': ['animated-2d', '2d-animated', '2d-animation'],
  hentai: ['hentai-anime', 'hentai-joi'],
  creampie: ['cum-in-pussy', 'cum-inside-pussy', 'cum-inside', 'cowgirl-creampie', 'balls-deep-creampie', 'breeding-creampie', 'multiple-creampie', 'tomboy-creampie'],
  'anal-creampie': ['cum-in-ass'],
  'cum-in-mouth': ['cim', 'cum-mouth', 'blowjob-cum-in-mouth'],
  throatpie: ['cum-in-throat'],
  'reverse-cowgirl': ['reverse-cowgirl-position', 'reverse-cowgirl-pov'],
  cowgirl: ['cowgirl-position', 'cowgirl-pov', 'girl-on-top'],
  riding: ['cock-riding', 'riding-cock', 'riding-dick', 'pov-riding', 'cowgirl-riding', 'best-cock-riding', 'ride', 'rides', 'riding-orgasm'],
  doggystyle: ['doggystyle-position', 'doggystyle-pov', 'dog-style', 'from-behined'],
  'standing-doggy': ['standing-doggystyle'],
  pronebone: ['prone-bone'],
  missionary: ['missionary-position', 'missionary-pov'],
  blowjob: ['blow-jobs', 'blow', 'blowjob-pov', 'pov-blowjob', 'close-up-blowjob', 'deep-blowjob', 'sloppy-blowjob', 'slow-blowjob', 'blowjob-face',
    'dick-sucking', 'sucking-dick', 'cocksucking', 'fellatio', 'oral', 'oral-sex', 'asian-blowjob', 'dildo-blowjob', 'double-blowjob', 'sloppy-head',
    'suck', 'sucking', 'mamada', 'pijpen'],
  deepthroat: ['extreme-deepthroat', 'sloppy-deepthroat', 'asian-deepthroat'],
  facefuck: ['face-fuck', 'facefucking', 'throat-fuck'],
  handjob: ['hj', 'pov-handjob', 'fast-handjob', 'edging-handjob', 'femdom-handjob'],
  milking: ['milking-machine'],
  titfuck: ['titty-fuck'],
  orgy: ['group-sex', 'group'],
  tentacle: ['tentacles', 'tentacle-sex', 'tentacle-monster'],
  monstergirls: ['monster-girl'],
  pmv: ['pmv-compilation'],
  'cock-hero': ['selfmadecockhero'],
  'beat-based': ['fap-to-beat', 'beat-bar', 'metronome'],
  vr: ['virtual-reality'],
  pov: ['point-of-view', 'partial-pov', 'punto-de-vista', 'sexo-pov', 'pov-sex'],
  overwatch: ['ow'],
  vaginal: ['vaginal-sex', 'penis-in-pussy'],
  anal: ['anal-sex', 'ass-fuck', 'deep-anal'],
  cumshot: ['cumshots', 'big-cumshot', 'huge-cumshot', 'multiple-cumshots'],
}).flatMap(([tag, spellings]) => spellings.map((s) => [s, tag])));

/**
 * The focus themes a video has (`PMV / HMV`, `creampie`, `reverse cowgirl`,
 * `VR`), by its library-spelled tags, its title, or FapTap's VR flag.
 *
 * @param {{tags?: string[], title?: string, vr?: boolean}} video
 */
export function focusOf({ tags = [], title = '', vr = false }) {
  return FAPTAP_FOCUS.filter((f) => f.tags.some((t) => tags.includes(t)) || f.title.test(title) || (f.name === 'VR' && vr)).map((f) => f.name);
}

const MAX_HEIGHT = { flat: FLAT_MAX_HEIGHT, vr: VR_MAX_HEIGHT };
// FapTap projection → the file-name suffix HereSphere / DeoVR (and the
// backend's detect_vr_format) read. Flat and flat 3D get none.
const PROJECTION_SUFFIX = {
  STEREO_180_LR: '_180_LR',
  STEREO_180_LR_SPHERICAL: '_180_LR',
  STEREO_180_TB: '_180_TB',
  STEREO_180_TB_SPHERICAL: '_180_TB',
  STEREO_360_LR: '_360_LR',
  STEREO_360_TB: '_360_TB',
  STEREO_FISHEYE_200_LR: '_MKX200',
  MONO_360: '_MONO360',
};

/** A FapTap tag name as an EroScripts-style slug: `Cum in Mouth` → `cum-in-mouth`. */
export function tagSlug(name) {
  return String(name || '').trim().toLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-').replace(/^-+|-+$/g, '');
}

/**
 * A listed video's tags in the library's spelling, without duplicates:
 * aliases replaced (see TAG_ALIASES), and the franchise of a character tag
 * (`Mercy (Overwatch)`) added.
 */
export function videoTags(video) {
  const out = new Set();
  for (const t of video?.tags || []) {
    const slug = tagSlug(t?.name);
    if (slug) out.add(TAG_ALIASES[slug] || slug);
    const franchise = String(t?.name || '').match(/\(([^()]+)\)\s*$/);
    if (franchise && tagSlug(franchise[1])) out.add(tagSlug(franchise[1]));
  }
  return [...out];
}

const htmlText = (html) => String(html || '').replace(/<[^>]+>/g, ' ').replace(/&nbsp;/g, ' ');
const isLive = (t) => LIVE_TAGS.has(t) || t === 'amateur' || t.startsWith('amateur-');

/** HereSphere / DeoVR suffix for a FapTap projection; '' for flat video. */
export function vrSuffix(projection) {
  return PROJECTION_SUFFIX[projection] || '';
}

/**
 * FapTap's thumbnail for a video as an absolute URL: Bunny's frame
 * (`https://vz-….b-cdn.net/…/thumbnail.jpg`) or an uploaded one
 * (`thumbnails/<id>.webp`, served under `/api/assets/`). null when none.
 */
export function thumbnailUrl(video) {
  const t = String(video?.thumbnail_url || '').trim();
  if (!t) return null;
  if (/^https:\/\//i.test(t)) return t;
  return /^[\w./-]+$/.test(t) && !t.includes('..') ? `${FAPTAP_ORIGIN}/api/assets/${t.replace(/^\/+/, '')}` : null;
}

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
// Performers as `{name, category}` (a bare name counts as unknown category).
const people = (performers) => (performers || [])
  .map((p) => (typeof p === 'string' ? { name: p.trim(), category: '' } : { name: String(p?.name ?? '').trim(), category: String(p?.category ?? '') }))
  .filter((p) => p.name);

/**
 * Who a FapTap scene is filed under (`<artist>/<Artist - Title>`): the
 * person who made or stars in the video, not the scripter who uploaded it.
 * In order: a favourite named in the title, performers or tags; the title's
 * own artist (leading `[Name]`, `(by Name)`, trailing `by Name`); a library
 * creator named in the title or listed as performer; a performer FapTap
 * calls a model or pornstar (other performers are often characters); the
 * uploader.
 *
 * @param {{title: string, uploader?: string, performers?: ({name: string, category?: string}|string)[], tags?: string[]}} video
 * @param {{favourites?: string[], creatorNames?: string[]}} [ctx] creatorNames: library creator folders as spelled
 */
export function faptapArtist({ title, uploader, performers, tags = [] }, { favourites = [], creatorNames = [] } = {}) {
  const cast = people(performers);
  const favourite = favouriteIn([title, ...cast.map((p) => p.name), ...tags], favourites);
  if (favourite) return favourite;
  const by = String(title || '').match(/\(\s*by\s+([^()]+?)\s*\)/i)?.[1];
  // `[Script Overhaul]`, `[Extended]` … label the upload, not a person.
  const own = [titleArtist(title), by].find((a) => a && !/script|overhaul|remake|remaster|edit|version|extended|update|request/i.test(a));
  if (own) return own;
  const creators = creatorNames.filter((c) => c.length > 3 && !isCreatorLabel(c));
  const words = ` ${normName(title)} `;
  const known = creators.find((c) => words.includes(` ${normName(c)} `))
    || creators.find((c) => cast.some((p) => normName(p.name) === normName(c)));
  return known || cast.find((p) => LIVE_CATEGORIES.has(p.category))?.name || String(uploader || '').trim() || 'unknown';
}

/**
 * The scene's creator folder and file stem, `Artist - Title (metadata)` as
 * for EroScripts posts: the artist (faptapArtist) leads, its mentions leave
 * the title, a leading bracket naming someone else becomes metadata, and a
 * VR projection suffix ends the stem.
 *
 * @param {{title: string, uploader?: string, performers?: object[], tags?: string[], vr?: boolean, projection?: string}} video
 * @param {{favourites?: string[], creatorNames?: string[]}} [ctx]
 * @returns {{artist: string, creator: string, stem: string}}
 */
export function faptapSceneName(video, ctx = {}) {
  const artist = faptapArtist(video, ctx);
  const a = escapeRe(artist);
  let title = String(video.title || '')
    .replace(new RegExp(`[\\[(（【]\\s*(?:by\\s+)?${a}\\s*[\\])）】]`, 'giu'), ' ')
    .replace(new RegExp(`(?<![\\p{L}\\p{N}])(?:by\\s+)?${a}(?![\\p{L}\\p{N}])`, 'giu'), ' ')
    .replace(/\s+/g, ' ').trim();
  // A leading `[Someone]` that isn't the artist moves to the metadata.
  const lead = title.match(/^[\[(（【]([^\])）】]+)[\])）】]\s*/);
  if (lead) title = `${title.slice(lead[0].length)} (${lead[1].trim()})`;
  title = title.replace(/^[\s\-–—|:,]+|[\s\-–—|:,]+$/g, '');
  // Nothing left but brackets (`THEOBROBINE (Fap Hero)`): their text is the title.
  if (!title.replace(/[\[(（【][^\])）】]*[\])）】]/g, '').replace(/[^\p{L}\p{N}]/gu, '')) {
    title = title.replace(/[\[(（【\])）】]/g, ' ').replace(/\s+/g, ' ').trim();
  }
  title ||= String(video.title || '');
  const suffix = video.vr ? vrSuffix(video.projection) : '';
  const stem = sceneTitle({ title: `[${artist}] ${title}`, author: artist, video: `${title}${suffix}.mp4` });
  const creator = artist.replace(/[/\\:*?"<>|]/g, '').replace(/\s+/g, ' ').trim() || 'unknown';
  return { artist, creator, stem };
}

/**
 * Judge a video before any further request (a listed one, or one the user
 * picked by id).
 *
 * @param {{id: string, name: string, description?: string, duration?: number, tags?: {name: string}[],
 *   vr?: boolean, projection?: string, user?: {username?: string}, performers?: {name: string}[]}} video
 * @param {object} ctx
 * @param {string[]} ctx.creators creator names in the library (lower case)
 * @param {string[]} ctx.knownTitles normName of every library scene folder and video, and of the EroScripts scout's waiting posts
 * @param {string[]} [ctx.livePerformers] normName of FapTap's models and pornstars (see livePerformerNames)
 * @param {string[]} [ctx.favourites] favourite creators and actresses (scripts/scout-live.mjs)
 * @param {boolean} [ctx.picked] the user asked for this video by id
 * @returns {{ok: false, reason: string} | {ok: true, tags: string[], vr: boolean, music: boolean, artist: string|null,
 *   focus: string[], preferred: string[], live: boolean, favourite: string|null, liveQuota: boolean, picked: boolean, hentaiOnly: boolean}}
 *   reasons: excluded, blocked, audio-only, off-focus, not-sexual, short, in-library
 */
export function screenVideo(video, { creators = [], knownTitles = [], livePerformers = [], favourites = [], picked = false } = {}) {
  const tags = videoTags(video);
  const title = String(video.name || '');
  const text = htmlText(video.description);
  if (tags.some((t) => hasExcludedText(t) || FURRY_TAGS.has(t)) || hasExcludedText(title) || hasExcludedText(text)) {
    return { ok: false, reason: 'excluded' };
  }
  if (isBlockedCreator({ title, author: video.user?.username, performers: (video.performers || []).map((p) => p?.name) })) {
    return { ok: false, reason: 'blocked' };
  }
  const name = normName(title);
  const words = ` ${name} `;
  const uploader = String(video.user?.username || '').toLowerCase();
  const artists = creators.filter((c) => c.length > 3 && !isCreatorLabel(c));
  const artist = artists.find((c) => words.includes(` ${normName(c)} `)) || (artists.includes(uploader) ? uploader : null);
  const favourite = favouriteIn([title, video.user?.username, ...(video.performers || []).map((p) => p?.name), ...tags], favourites);
  const live = !(tags.some((t) => ANIMATION_TAGS.has(t) || FAPTAP_ANIMATION_TAGS.has(t)) && !tags.some(isLive)
    && !livePerformers.some((p) => words.includes(` ${p} `)));
  const music = tags.some((t) => MUSIC_TAGS.has(t));
  if (tags.some(isAudioRoleplayTag) || AUDIO_TITLE.test(title)) return { ok: false, reason: 'audio-only' };
  const vr = !!video.vr || tags.includes('vr');
  const focus = focusOf({ tags, title, vr });
  if (!picked && !favourite && focus.length < FAPTAP_FOCUS_MIN) return { ok: false, reason: 'off-focus' };
  if (!isSexual({ tags, title })) return { ok: false, reason: 'not-sexual' };
  if (!(Number(video.duration) * 1000 >= MIN_VIDEO_MS)) return { ok: false, reason: 'short' };
  // Both ways: a five-word title sharing four generic words isn't the same video.
  if (knownTitles.some((k) => k.length > 8 && sameVideo(name, k) && sameVideo(k, name))) return { ok: false, reason: 'in-library' };
  const liveQuota = live && !picked && !favourite && !(music && artist);
  return {
    ok: true, tags, vr, music, artist, focus, preferred: tags.filter((t) => PREFERRED_TAGS.has(t)), live, favourite, liveQuota, picked,
    hentaiOnly: isHentaiOnly({ tags, title }),
  };
}

/**
 * A screened video once its details are known: a model or pornstar listed
 * as performer makes it live action, and a favourite among the performers
 * makes it a favourite's.
 *
 * @param {object} screen screenVideo's ok result
 * @param {{name: string, category?: string}[]} performers the details' performers
 * @param {string[]} favourites
 */
export function withPerformers(screen, performers, favourites) {
  const live = screen.live || hasLivePerformer({ performers });
  const favourite = screen.favourite || favouriteIn((performers || []).map((p) => p?.name), favourites);
  const liveQuota = live && !screen.picked && !favourite && !(screen.music && screen.artist);
  return { ...screen, live, favourite, liveQuota };
}

/** Performer categories FapTap gives real people. */
const LIVE_CATEGORIES = new Set(['model', 'pornstar']);

/**
 * Names worth matching in titles: FapTap's models and pornstars with at
 * least two words (`Molly Little`), as normName.
 *
 * @param {{name: string, category?: string}[]} performers `/api/performers`
 */
export function livePerformerNames(performers) {
  return [...new Set((performers || [])
    .filter((p) => LIVE_CATEGORIES.has(p?.category))
    .map((p) => normName(p.name))
    .filter((n) => n.split(' ').length >= 2 && n.length >= 6))];
}

/** True when a video's details list a model or pornstar as its performer. */
export function hasLivePerformer(detail) {
  return (detail?.performers || []).some((p) => LIVE_CATEGORIES.has(p?.category));
}

/**
 * The stream to download: the highest HLS (or MP4) quality within the cap,
 * or null when the best one is below the floor.
 *
 * @param {{url: string, quality: string|number, format?: string}[]} sources `/bunny-sources`
 * @param {boolean} vr
 */
export function pickSource(sources, vr) {
  const kind = vr ? 'vr' : 'flat';
  const usable = (sources || [])
    .map((s) => ({ ...s, height: parseInt(s?.quality, 10) }))
    .filter((s) => s.url && ['hls', 'mp4'].includes(String(s.format || 'hls').toLowerCase()) && s.height > 0 && s.height <= MAX_HEIGHT[kind])
    .sort((a, b) => b.height - a.height);
  return usable[0] && usable[0].height >= MIN_STREAM_HEIGHT[kind] ? usable[0] : null;
}

/**
 * FapTap's `ms,pos` script as a funscript: actions in time order, one per
 * millisecond (the last wins), positions clamped to 0..100. Lines that
 * aren't two numbers are skipped.
 */
export function csvToFunscript(text) {
  const byTime = new Map();
  for (const line of String(text || '').split(/\r?\n/)) {
    const m = line.trim().match(/^(\d+(?:\.\d+)?)\s*[,;]\s*(-?\d+(?:\.\d+)?)$/);
    if (!m) continue;
    byTime.set(Math.round(Number(m[1])), Math.min(100, Math.max(0, Math.round(Number(m[2])))));
  }
  const actions = [...byTime].sort((a, b) => a[0] - b[0]).map(([at, pos]) => ({ at, pos }));
  return { version: '1.0', inverted: false, range: 100, actions };
}

/** A screened video as a saved candidate. */
export function toCandidate(video, screen) {
  const c = {
    id: String(video.id),
    title: String(video.name || '').trim(),
    url: `${FAPTAP_ORIGIN}/v/${video.id}`,
    author: video.user?.username || 'unknown',
    tags: screen.tags,
    likes: Number(video.rating?.likes) || 0,
    duration: Number(video.duration) || 0,
    vr: screen.vr,
    projection: video.projection || 'MONO_FLAT',
    music: screen.music,
    focus: screen.focus,
    preferred: screen.preferred,
    artistInLibrary: screen.artist,
    live: !!screen.live,
    favourite: screen.favourite || null,
    liveQuota: !!screen.liveQuota,
    picked: !!screen.picked,
  };
  return { ...c, score: candidateScore(c) };
}

/** Key of a fetched or rejected FapTap scene; the same as electron/rejected-videos.js `faptapRejectionKey`. */
export function fetchedKey(id) {
  return `faptap|${id}`;
}

/**
 * A `Cookie` header after a response's `Set-Cookie` lines: each cookie
 * replaces the one of the same name; one set empty or expired goes.
 *
 * @param {string} jar current `name=value; name=value`
 * @param {string[]} setCookies `Set-Cookie` header values
 */
export function mergeCookies(jar, setCookies) {
  const cookies = new Map(String(jar || '').split(/;\s*/).filter((kv) => kv.includes('=')).map((kv) => [kv.slice(0, kv.indexOf('=')), kv]));
  for (const line of setCookies || []) {
    const kv = String(line).split(';')[0].trim();
    const eq = kv.indexOf('=');
    if (eq <= 0) continue;
    const name = kv.slice(0, eq);
    // Base64 values end in `=`; only a value with nothing after the first `=` is empty.
    if (eq === kv.length - 1 || /;\s*max-age=0(;|$)/i.test(line) || /;\s*expires=thu, 01 jan 1970/i.test(line)) cookies.delete(name);
    else cookies.set(name, kv);
  }
  return [...cookies.values()].join('; ');
}

// The focus rule on a saved candidate (lists saved under older rules may miss it); picks and favourites need none.
const inFocus = (c) => !!(c.picked || c.favourite) || focusOf(c).length >= FAPTAP_FOCUS_MIN;

/** True when a saved candidate may still be downloaded: in focus, and within the content rules (sex; picks too). */
export const stillWanted = (c) => inFocus(c) && !contentRuleBreak(c);

/**
 * FapTap often holds one video uploaded several times, with other scripts.
 * Of candidates with the same title and length (scripts/scene-dedupe-lib.mjs)
 * the best stream stays, then the most liked.
 *
 * @returns {{kept: object[], dropped: object[]}}
 */
export function dropDuplicateCandidates(candidates) {
  const item = (c) => ({ words: titleWords(c.title || ''), duration: Number(c.duration) || 0, vr: !!c.vr });
  const kept = [];
  const dropped = [];
  for (const c of [...candidates].sort((a, b) => (b.quality || 0) - (a.quality || 0) || (b.likes || 0) - (a.likes || 0))) {
    if (firstDuplicate(item(c), kept.map(item))) dropped.push(c);
    else kept.push(c);
  }
  return { kept, dropped };
}

/**
 * The saved list after a scan: candidates found now plus earlier ones not
 * yet fetched or rejected and still wanted (`stillWanted`), one per video
 * (`dropDuplicateCandidates`), best score first, each scored again under
 * the current RANKING (candidateScore).
 *
 * @param {object[]} previous
 * @param {object[]} fresh
 * @param {(id: string) => boolean} isDone fetched, or turned down in the app
 */
export function mergeCandidates(previous, fresh, isDone) {
  const byId = new Map();
  for (const c of [...(previous || []), ...(fresh || [])]) byId.set(String(c.id), c);
  const open = [...byId.values()].filter((c) => !isDone(String(c.id)) && stillWanted(c)).map((c) => ({ ...c, score: candidateScore(c) }));
  return dropDuplicateCandidates(open).kept.sort((a, b) => b.score - a.score);
}
