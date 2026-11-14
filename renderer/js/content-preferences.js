// Content preferences: every choice about which videos and scripts the
// library takes, ranks, keeps and how it sorts them, in one place.
//
// Read by the scouts (scripts/eroscripts-scout-lib.mjs,
// scripts/faptap-scout-lib.mjs, scripts/scout-live.mjs), the duplicate
// cleanup (scripts/scene-dedupe-lib.mjs) and the app (renderer/js/
// reject-rule.js, ratings.js, artist-playlists.js, auto-categories.js,
// auto-collections.js, score-bands.js, play-queue.js). Those modules hold the logic; the
// values live here.
// Changing a screening rule here means bumping RULES_VERSION in
// scripts/eroscripts-scout-lib.mjs (and FAPTAP_RULES_VERSION in
// scripts/faptap-scout-lib.mjs) so judged posts are judged again.
//
// Tags are EroScripts slugs (FapTap tags are aliased to them first). Title
// and text patterns match whole words only: `(?<![\p{L}\p{N}])…(?![\p{L}\p{N}])`.
//
// Values the Rust CLIs read too live in content-preferences.json next to this
// file (the shared part of the preferences: Rust embeds it at compile time,
// crates/age-screen, crates/pornstar-tags, crates/slr-scout). The ones the app
// uses are exported here like the rest (`pornstarTags` and `slrScout` are read
// by Rust only); change them there: the ratings threshold, the underage
// screener, LIBRARY_MAX_VIDEOS, the VR / flat caps, FAVOURITES, RANKING,
// TAG_SCORE_GROUPS (whose tag lists MUSIC_TAGS, POV_TAGS, MONSTER_TAGS,
// FEET_TAGS and FFM_TAGS are) and ANIMATION_TAGS.
// Values with a copy outside JavaScript: TOP_RATED_MIN and the SCORE_BANDS
// thresholds are mirrored in backend/services/vr_sections.py, DEFAULT_SORT in
// backend/web-remote/sort-default.js (the phone can't import this file).

import data from './content-preferences.json' with { type: 'json' };

/** A TAG_SCORE_GROUPS group's tags. */
const groupTags = (name) => data.tagScoreGroups.find((g) => g.name === name).tags;

// ── Discovery: what never comes in ──────────────────────────────────────

/** Excluded content, by tag, title or first post / description. Hyphenated tags (`futa-on-male`) match, `Futaba` doesn't. */
export const EXCLUDED_TEXT = /(?<![\p{L}\p{N}])(loli|lolicon|shota|shotacon|furry|furries|anthro|yiff|futa|futas|futanari|dickgirls?)(?![\p{L}\p{N}])/iu;
/** Creators never taken by either scout (picks included): named in the title, as the poster / uploader, or as a performer. Case, spaces and punctuation don't matter. */
export const BLOCKED_CREATORS = ['AidenHet'];
/**
 * Tags never taken by either scout (picks and favourites included): MMD
 * animations by the `mmd` tag (FapTap's slug too), and dildo riding (user,
 * 2026-11-04: "no dildo riding videos") …
 */
export const BLOCKED_TAGS = new Set(['mmd', 'dildo-riding', 'riding-dildo', 'dildo-ride']);
/** … or MMD / dildo as a word in the title. */
export const BLOCKED_TITLE = /(?<![\p{L}\p{N}])(mmd|dildos?)(?![\p{L}\p{N}])/iu;
/** FapTap tags that are furry without saying so. */
export const FURRY_TAGS = new Set([
  'scalie', 'knot', 'knotted', 'horse', 'dog', 'animal', 'my-little-pony', 'animal-crossing', 'krystal-star-fox',
  'renamon-digimon', 'five-nights-at-freddy-s', 'roxanne-wolf-five-nights-at-freddy-s',
]);
/** Monsters, by tag or title word (`X WW` = werewolf; EroScripts also by these words in the first post): not preferred, they rank lower (the Monster group of TAG_SCORE_GROUPS) but still come in. Succubi, demon girls and elves aren't monsters. */
export const MONSTER_TAGS = new Set(groupTags('Monster'));
export const MONSTER_TITLE = /(?<![\p{L}\p{N}])(monsters?|monster ?girls?|tentacles?|w[ae]rewol(?:f|ves)|horse ?guys?|horse ?cocks?|orcs?|goblins?|minotaurs?|ogres?|trolls?|aliens?|beasts?|bestiality)(?![\p{L}\p{N}])|触手/iu;
/** `Mercy X WW`: Solace2K and Rekin3D's short for werewolf, a standalone capital token. */
export const WEREWOLF_TOKEN = /(?<![\p{L}\p{N}])WW(?![\p{L}\p{N}])/u;
/** Feet, by tag or title word (EroScripts also in the first post): not preferred, they rank lower (the Feet group of TAG_SCORE_GROUPS) but still come in. */
export const FEET_TAGS = new Set(groupTags('Feet'));
export const FEET_TITLE = /(?<![\p{L}\p{N}])(feet|foot|footjobs?|toes|soles|footsie)(?![\p{L}\p{N}])|足コキ/iu;
/** FFM (two women, one man), by tag or title word (EroScripts also in the first post): not preferred, ranks lower (the FFM group of TAG_SCORE_GROUPS) but still comes in. */
export const FFM_TAGS = new Set(groupTags('FFM'));
export const FFM_TITLE = /(?<![\p{L}\p{N}])ffm(?![\p{L}\p{N}])/iu;
/** Tags that say a post has no porn in it: never taken. */
export const SFW_TAGS = new Set(['sfw', 'safe-for-work', 'non-porn', 'nonporn', 'non-nude']);
/** FapTap audio roleplays (erotic audio / ASMR over a still or a loop): never taken. */
export const AUDIO_TITLE = /(?<![\p{L}\p{N}])(erotic audio|asmr|audio role ?play|f4m)(?![\p{L}\p{N}])/iu;
/** Audio roleplay tags: `asmr-audio-roleplay`, `erotic-audio-for-men`, `lewdlexiaudio`. */
export const isAudioRoleplayTag = (t) => t.includes('asmr') || t.includes('audio-roleplay') || (t.endsWith('audio') && t !== 'audio');
/** Shortest video worth downloading, by its main script's last action (FapTap also by listed length). 1:50 since 2026-11-09 (was 2:00). */
export const MIN_VIDEO_MS = 110_000;

// ── Discovery: animated or live action ──────────────────────────────────

/** Animation tags: a post with one (and no `real` tag) is animated; anything else is live action. AI video counts. slr:scout drops them from SLR's categories (live action; its `3D` means stereoscopic). */
export const ANIMATION_TAGS = new Set(data.animationTags);
/** More animation tags, as FapTap uses them. */
export const FAPTAP_ANIMATION_TAGS = new Set(['2d', 'ai-video', 'iwara']);
/** FapTap tags that make a video live action even with an animation tag (also any `amateur…` tag, and FapTap models / pornstars as performers). */
export const LIVE_TAGS = new Set([
  'real', 'real-sex', 'real-couple-homemade', 'real-orgasm', 'reality', 'homemade', 'pornstar', 'onlyfans', 'brazzers',
  'blacked', 'chaturbate', 'webcam', 'verified-amateurs', 'jav', 'celebrity', 'tiktok', 'cosplay', 'anime-cosplay', 'kbj',
  'egirl', 'sislovesme', 'dadcrush',
]);

// ── Discovery: scripts and axes ─────────────────────────────────────────

/** EroScripts feeds the scout lists (Free Scripts tagged …). No hentai feed: hentai comes in through the others; no mmd feed: MMD is blocked (BLOCKED_TAGS). */
export const FEED_TAGS = ['multi-axis', 'animation', '3d', 'vam', 'sfm'];
/**
 * Feeds the forum has no tag for, listed by a search of Free Scripts first posts instead (feed → search words).
 * Not in a default scan: `--feeds vrbangers`, or `npm run vrbangers:scout` (VRBangers posts only); `--feeds slr`
 * (posts whose video is on SexLikeReal, all performers; downloaded with the user's SLR account).
 */
export const SEARCH_FEEDS = { vrbangers: 'vrbangers', slr: 'sexlikereal' };
/** PMV / HMV / Cock Hero / Fap Hero, by tag … */
export const PMV_TAGS = new Set(['pmv', 'hmv', 'cock-hero', 'fap-hero', 'music-video']);
/** … or by title. */
export const PMV_TITLE = /(?<![\p{L}\p{N}])(pmv|hmv|cock ?hero|fap ?hero)(?![\p{L}\p{N}])/iu;
// Both scouts take single-axis and multi-axis scenes (user, 2026-11-07:
// hand-made axes are a preference, not a requirement). Single-axis scenes
// get generated twist / roll / pitch; hand-made axes always win, rank
// RANKING.multiAxis higher and download first. FapTap has no multi-axis
// scripts, so all of its scenes come in single-axis.
/** Most videos the library holds: at this many, a scout `--fetch` downloads nothing more (delete scenes or raise it). */
export const LIBRARY_MAX_VIDEOS = data.libraryMaxVideos;
/** FapTap (single-axis only): a video needs at least FAPTAP_FOCUS_MIN of these themes, by tag or title (VR also by FapTap's flag); favourites need none. */
export const FAPTAP_FOCUS = [
  { name: 'PMV / HMV', tags: [...PMV_TAGS], title: PMV_TITLE },
  { name: 'creampie', tags: ['creampie', 'anal-creampie', 'oral-creampie'], title: /(?<![\p{L}\p{N}])creampies?(?![\p{L}\p{N}])/iu },
  { name: 'reverse cowgirl', tags: ['reverse-cowgirl'], title: /(?<![\p{L}\p{N}])reverse[ -]cowgirl(?![\p{L}\p{N}])/iu },
  { name: 'VR', tags: ['vr'], title: /(?<![\p{L}\p{N}])vr(?![\p{L}\p{N}])/iu },
];
export const FAPTAP_FOCUS_MIN = 2;

// ── Discovery: favourites and live action ───────────────────────────────

/**
 * Starting favourite creators and actresses: they skip the live-not-POV and
 * FapTap focus rules, they rank RANKING.favouriteArtist higher, and the app
 * gives each an artist playlist from their first video plus the
 * FAVOURITES_PLAYLIST. Matched in titles, tags, uploaders and performers
 * (scripts/scout-live.mjs favouriteIn). The user adds and removes favourites
 * in the web remote's Artists view; `library.favouriteArtists` holds only
 * those changes (renderer/js/favourite-artists.js).
 */
export const FAVOURITES = data.favourites;
/**
 * Flat live action is turned down only when something shows it isn't POV
 * (user, 2026-11-03: favourites exempt; "let's not bother if we don't have
 * any indication a flat real post is not POV or a PMV or HMV or similar music
 * compilation"): a NON_POV_TAGS tag, or a music compilation (PMV_TAGS, the
 * `compilation` tag, PMV_TITLE or COMPILATION_TITLE). POV (POV_TAGS or POV as
 * a title word) always passes, and so do VR, favourites and posts the user
 * picks by id (`--add`). No indication either way passes.
 */
export const POV_TAGS = new Set(groupTags('POV'));
export const POV_TITLE = /(?<![\p{L}\p{N}])pov(?![\p{L}\p{N}])/iu;
export const NON_POV_TAGS = new Set(['non-pov', 'no-pov', 'not-pov']);
export const COMPILATION_TITLE = /(?<![\p{L}\p{N}])compilations?(?![\p{L}\p{N}])/iu;

// ── Discovery: ranking ──────────────────────────────────────────────────

/** Music tags (the Music tag-score group). */
export const MUSIC_TAGS = new Set(groupTags('Music'));
/**
 * Candidate score for what isn't a single tag; higher downloads first. Likes
 * add likes / likesPer, up to likesMax; 2D hentai only loses hentaiOnly.
 * multiAxis applies to EroScripts only (FapTap has no multi-axis scripts).
 * A favourite artist's post counts as artistInLibrary and also gets
 * favouriteArtist on top (the user changes it in the web remote's Artists
 * view: `library.favouriteArtistScore`, same range and steps as a tag score).
 * Tag preferences are TAG_SCORE_GROUPS. The app scores every library scene
 * the same way (preference-score.js, library-scores.js).
 */
export const RANKING = data.ranking;
/**
 * Tag scores the preference score starts from: every tag of a group starts at
 * the group's score, and a scene counts each group once (its highest positive
 * and its lowest negative tag score), so synonyms don't add up; groups do.
 * The user changes any tag's score in the web remote's Tags view
 * (`library.tagScores` holds only the changes). `named`: the group also
 * counts, at its first tag's score, when the title (EroScripts: or first
 * post) names it without a tag (lessPreferred in preference-score.js).
 * A tag belongs to one group at most.
 */
export const TAG_SCORE_GROUPS = data.tagScoreGroups;
/** 2D hentai only: animation tags among these, no THREE_D_TAGS and no THREE_D_TITLE. */
export const FLAT_HENTAI_TAGS = new Set(['hentai', '2d', 'anime']);
export const THREE_D_TAGS = new Set(['3d', 'sfm', 'vam', 'mmd', 'cgi', 'blender', 'koikatsu', 'honey-select']);
export const THREE_D_TITLE = /(?<![\p{L}\p{N}])(3d|sfm|vam|mmd)(?![\p{L}\p{N}])/iu;

// ── Video quality ───────────────────────────────────────────────────────

/** VR playback caps (heavier files stutter in the headset): ≤ 8K, ≤ 60 fps, the low bitrate when offered. */
export const VR_MAX_HEIGHT = data.videoCaps.vrMaxHeight;
export const VR_MAX_FPS = data.videoCaps.vrMaxFps;
/** Flat: ≤ 4K, otherwise the highest quality. */
export const FLAT_MAX_HEIGHT = data.videoCaps.flatMaxHeight;
/** Lowest FapTap stream worth downloading (lower ones are preview uploads). */
export const MIN_STREAM_HEIGHT = { flat: 720, vr: 1080 };

// ── Duplicates: which copy stays ────────────────────────────────────────

/**
 * One copy per video (user, 2026-11-04: "always keep biggest video and
 * fastest script"). The video that stays: within the VR caps, then the
 * biggest file. The scripts it keeps: the copy with hand-made axes (they
 * always win), then the fastest main script; when that's another copy's, its
 * script set moves next to the kept video. The first difference decides
 * (names are the reasons scripts/scene-dedupe-lib.mjs prints).
 */
export const DEDUPE_KEEP_ORDER = ['within VR caps', 'bigger video'];
export const DEDUPE_SCRIPT_ORDER = ['hand-made axes', 'faster script'];

// ── Names: words that don't tell videos apart ───────────────────────────

/**
 * Words in a video, script or post name that describe the encode, the cut or
 * a script variant, not what the video shows. Dropped before names are
 * compared: duplicate scenes (scripts/scene-dedupe-lib.mjs titleWords), the
 * scouts' library checks and their video / script pairing
 * (scripts/eroscripts-scout-lib.mjs coreName, sameVideo). Projection words
 * (`vr`, `sbs`, `180` …) are dropped too (VR_MARKER in scene-dedupe-lib).
 */
export const NAME_NOISE_WORDS = new Set([
  'hq', 'lq', 'sd', 'hd', 'fhd', 'qhd', 'uhd', 'upscale', 'upscaled', 'remaster', 'remastered', 'passthrough', 'pt', 'source',
  'hmv', 'pmv', 'high', 'low', 'bitrate', 'lowbitrate', 'highbitrate', 'version', 'ver', 'final', 'fixed', 'full',
  'hevc', 'h264', 'h265', 'x264', 'x265', 'av1', 'mp4', 'mkv', 'compressed', 'uncompressed', 'edit',
  'easy', 'easier', 'normal', 'medium', 'hard', 'harder', 'extreme', 'intense', 'soft', 'beginner', 'advanced', 'alt', 'alternative',
]);
/**
 * … and encode tokens, also run together: `1080p`, `4k`, `60fps`, `4k120fps`,
 * `1080p60`, `4k60`, a bare `1080` / `720` (`Title (1080)`), `v2`, `10bit`, `75mbps`.
 */
export const NAME_NOISE_TOKEN = /^((\d{3,4}p|\d{1,2}k)(\d{2,3}(fps)?)?|\d{2,3}fps|fps\d{2,3}|v\d+(\.\d+)?|\d+bit|\d+mbps|480|540|720|1080|1440|2160|2880|4320)$/;
/**
 * Genre words: they stay in a title but never tell two videos apart, so a
 * title needs another word to match anything (`newnew - VAM`, `Sulbas VR
 * Compilation`, an EroScripts post called `Fap Hero`). `fap hero` and `cock
 * hero` count as one word (`faphero`, `cockhero`).
 */
export const GENRE_WORDS = new Set(['vam', 'sfm', 'mmd', 'compilation', 'compilations', 'faphero', 'cockhero', 'pmv', 'hmv', 'pov', 'vr']);

// ── Ratings and triage ──────────────────────────────────────────────────

/** Rated this or lower (and not a favourite): deleted from disk, scripts and axes too, and the scouts skip the post. Shared (content-preferences.json): the age screener skips these videos. */
export const DELETE_AT_OR_BELOW = data.ratings.deleteAtOrBelow;
/** "Top rated" filter and the headset's Top Rated section (mirrored in backend/services/vr_sections.py). */
export const TOP_RATED_MIN = 4;

// ── Sorting and score lists ─────────────────────────────────────────────

/**
 * The library's sort when nothing else was chosen (desktop library and its collections,
 * web remote): preference score, highest first, unscored last (user, 2026-11-10; 10 → 15 the same day). Watched
 * videos still go last, each group by score. Mirrored in backend/web-remote/sort-default.js.
 */
export const DEFAULT_SORT = 'score:desc';

/**
 * Score lists (user, 2026-11-10): preference score bands kept as automatic collections on
 * the desktop (renderer/js/score-bands.js; the phone's Collections tab shows them), each
 * video in the band of its current score: `min <= score < below` (either end open);
 * unscored videos are in none. Cut at the library's spread (2026-11-10: median 11, top 20 %
 * from 14, top 10 % from 16.5, 9 videos at 20+): 20+ the very best, 15–19.5 the next ~15 %,
 * 10 the picking threshold for new downloads. `formerNames`: a collection made under an
 * older name is renamed (not duplicated). The headsets list the first three as sections
 * "Score 20+", "Score 15-19.5" and "Score 10-14.5"; those thresholds are mirrored in
 * backend/services/vr_sections.py (SCORE_ELITE_MIN, SCORE_TOP_MIN, SCORE_MID_MIN).
 */
export const SCORE_BANDS = [
  { key: 'score-20', name: 'Score 20+', min: 20 },
  { key: 'score-15', name: 'Score 15–19.5', min: 15, below: 20, formerNames: ['Score 15+'] },
  { key: 'score-10', name: 'Score 10–14.5', min: 10, below: 15 },
  { key: 'score-low', name: 'Score below 10', below: 10 },
];

// ── Smart playlists ─────────────────────────────────────────────────────

/**
 * Smart playlists (user, 2026-11-10; renderer/js/smart-playlists.js): app-made playlists that
 * refill themselves after each scan and whenever ratings, scores or durations change. Each
 * holds exactly its current members, only unwatched (unrated) videos, in its own order, so the
 * phone and the headsets (`Playlist: <name>`) show them like any playlist.
 * • `vr: true` — the backend's VR decision (as in the scores and the VR category).
 * • `minScore` — preference score at least this (`library.preferenceScores`).
 * • `maxSeconds` — known duration (`library.durationCache`) below this.
 * • `addedWithinDays` — added (the scan's `dateAdded`, the Date added sort and the headsets'
 *   Recently Added) at most this many days ago.
 * `order`: 'score' (highest first, title breaking ties) or 'added' (newest first).
 */
export const SMART_PLAYLISTS = [
  { key: 'vr-picks', name: 'VR picks', vr: true, minScore: 15, order: 'score' },
  { key: 'quick-ones', name: 'Quick ones', maxSeconds: 5 * 60, minScore: 15, order: 'score' },
  { key: 'fresh-downloads', name: 'Fresh downloads', addedWithinDays: 14, order: 'added' },
];

// ── Artist playlists ────────────────────────────────────────────────────

/** An artist folder with at least this many videos counts as a known creator for the preference score (library-scores.js). */
export const MIN_ARTIST_VIDEOS = 5;
/**
 * An artist folder with at least this many videos gets a playlist (user, 2026-11-10: keeps the
 * artist playlists to ~8; was MIN_ARTIST_VIDEOS). A favourite needs it too for its own playlist;
 * "Favourite artists" still takes a favourite's videos from one.
 */
export const ARTIST_PLAYLIST_MIN = 30;
export const FAVOURITES_PLAYLIST = 'Favourite artists';
/** Folder names that hold videos of many artists (lower case). */
export const NOT_AN_ARTIST = new Set(['unknown', 'misc', 'various', 'other', 'others', 'new folder', 'cock hero', 'fap hero', 'pmv', 'hmv', 'compilation', 'compilations']);
/**
 * Other spellings of an artist folder → the artist's own name (user, 2026-11-10). Compared like
 * artist folders (NFC, trimmed, any case; artist-playlists.js canonicalArtist): the app counts an
 * alias folder's videos under the artist (Artists view, Overview, artist playlists, scores,
 * favourites), and the scouts file new downloads under the artist's folder. Folders on disk keep
 * their names. `S022` / `S069` are miffn's series codes an older scout took for artist folders.
 */
export const ARTIST_ALIASES = {
  'Rondoudou Media': 'Rondoudou',
  RondoudouMedia: 'Rondoudou',
  'Horny Herring Studios': 'Horny Herring',
  shantiaoxiaozhi: 'shantianxiaozhi',
  S022: 'miffn',
  S069: 'miffn',
};

// ── Play queue ──────────────────────────────────────────────────────────

/**
 * The shared play queue's auto-fill (renderer/js/play-queue.js): videos are
 * appended after the user's own queue until it holds `size` in total, and a
 * video played or skipped from it comes back only after `replayAfterDays`.
 */
export const QUEUE_AUTOFILL = { size: 10, replayAfterDays: 30 };

// ── Auto categories ─────────────────────────────────────────────────────

/** Categories filled from post tags after each scan (VR and the axis kinds from the backend's decisions instead). */
export const AUTO_CATEGORIES = [
  { key: 'creampie', name: 'Creampie', color: '#e94560', tags: ['creampie', 'anal-creampie', 'oral-creampie'] },
  { key: 'riding', name: 'Riding', color: '#f39c12', tags: ['riding', 'cowgirl', 'reverse-cowgirl'] },
  { key: 'blowjob', name: 'Blowjob', color: '#ff6b81', tags: ['blowjob', 'bj', 'deepthroat', 'facefuck', 'throatpie', 'cum-in-mouth'] },
  { key: 'handjob', name: 'Handjob', color: '#fd79a8', tags: ['handjob', 'milking'] },
  { key: 'titjob', name: 'Titjob', color: '#e74c3c', tags: ['titfuck', 'titjob', 'paizuri', 'boobjob'] },
  { key: 'footjob', name: 'Footjob', color: '#1abc9c', tags: ['footjob'] },
  { key: 'anal', name: 'Anal', color: '#9b59b6', tags: ['anal', 'anal-creampie'] },
  { key: 'doggy', name: 'Doggy', color: '#3498db', tags: ['doggy', 'doggy-style', 'doggystyle', 'standing-doggy', 'pronebone'] },
  { key: 'missionary', name: 'Missionary', color: '#00cec9', tags: ['missionary'] },
  { key: 'group', name: 'Group', color: '#2ecc71', tags: ['gangbang', 'threesome', 'foursome', 'fivesome', 'orgy', 'double-penetration', 'harem'] },
  { key: 'monster', name: 'Monster', color: '#9b59b6', tags: ['monster', 'monstergirls', 'tentacle', 'werewolf'] },
  { key: 'pmv', name: 'PMV / HMV', color: '#f39c12', tags: [...PMV_TAGS] },
  { key: 'vr', name: 'VR', color: '#3498db', tags: [] },
  { key: 'pov', name: 'POV', color: '#00cec9', tags: [...POV_TAGS] },
  // One of these per scripted video, by its axes (backend `GET /api/media/axis-flags`).
  { key: 'axes-handmade', name: 'Hand-made axes', color: '#2ecc71', tags: [], axisKind: 'handmade' },
  { key: 'axes-generated', name: 'Generated axes', color: '#95a5a6', tags: [], axisKind: 'generated' },
  { key: 'axes-single', name: 'Single-axis', color: '#636e72', tags: [], axisKind: 'single' },
];

// ── Video tags ──────────────────────────────────────────────────────────

/**
 * The vocabulary the video tagger may add to a scene (`<userData>/video-tags.json`,
 * written by the `video-tags` CLI, merged into the scene's tags after each scan;
 * renderer/js/video-tags.js). Tags in the file outside this list are ignored.
 * Not a screening value: changing it needs no RULES_VERSION bump.
 */
export const VIDEO_TAGGER_TAGS = ['vr', 'pov', 'creampie', 'riding'];

// ── Underage screener ───────────────────────────────────────────────────

/**
 * The weekly underage screener (`npm run age:screen`, crates/age-screen;
 * values in content-preferences.json). A judge rates each video's chance of
 * depicting someone under 18 (`p_minor`, 0–1): at `purgeAt` or above it's
 * rated 1★ (deleted; favourites go to the playlist instead), from `reviewAt`
 * it goes to the `playlist` for the user to decide, below that it's clear.
 * Runs every `intervalDays` with `--weekly`.
 */
export const AGE_SCREEN = data.ageScreen;

// ── Auto collections ────────────────────────────────────────────────────

/**
 * Collections kept up to date after each scan, in display order. A video
 * matches by any of `tags`, by `title` (regex alternatives, whole words, on
 * the lower-cased file name), by `length` in seconds (`under` / `atLeast`;
 * the post's `lengthTags` when the duration isn't known yet), or by coming
 * from FapTap (`fapTap`).
 */
export const AUTO_COLLECTIONS = [
  // Medium
  { key: 'hentai', name: 'Hentai', tags: ['hentai', '2d'] },
  { key: 'sfm', name: 'SFM', tags: ['sfm'] },
  { key: 'vam', name: 'VaM', tags: ['vam'] },
  { key: 'mmd', name: 'MMD', tags: ['mmd'] },
  { key: 'cgi', name: 'CGI / 3D', tags: ['cgi', '3d', 'blender', 'koikatsu', 'honey-select'] },
  { key: 'ai', name: 'AI video', tags: ['ai-video', 'ai-assisted', 'ai', 'ai-generated'] },
  // Session length
  { key: 'quick', name: 'Quick (under 5 min)', length: { under: 300 }, lengthTags: ['len-0-2', 'len-2-5'] },
  { key: 'sessions', name: 'Sessions (15 min+)', length: { atLeast: 900 }, lengthTags: ['len-25-60', 'len-60-plus'] },
  { key: 'marathons', name: 'Marathons (1 hr+)', length: { atLeast: 3600 }, lengthTags: ['len-60-plus'] },
  // Franchise
  { key: 'overwatch', name: 'Overwatch', tags: ['overwatch'], title: 'overwatch|widowmaker|tracer|mercy|d\\.? ?va|kiriko|sombra|brigitte|pharah|symmetra|juno' },
  { key: 'genshin', name: 'Genshin Impact', tags: ['genshin-impact', 'genshin'], title: 'genshin|raiden shogun|ganyu|yae miko|furina|hu tao|keqing|lumine|nilou|eula|yelan|shenhe|arlecchino|navia|ayaka|fischl' },
  { key: 'zzz', name: 'Zenless Zone Zero', tags: ['zenless-zone-zero'], title: 'zenless|zzz|ellen joe|yixuan|qingyi|zhu yuan|burnice|miyabi|jane doe|nicole demara|anby|astra yao' },
  { key: 'final-fantasy', name: 'Final Fantasy', tags: ['final-fantasy', 'ff7', 'final-fantasy-7'], title: 'final fantasy|tifa|aerith|yuna|ff ?(?:7|vii|10|x|14|xiv)' },
  { key: 'nier', name: 'NieR (2B)', tags: ['nier', 'nier-automata', '2b'], title: 'nier|2b|yorha|a2' },
  { key: 'hsr', name: 'Honkai: Star Rail', tags: ['honkai-star-rail', 'honkai'], title: 'honkai|star rail|silver wolf|kafka|black swan|firefly|acheron|march 7th|stelle' },
  { key: 'lol', name: 'League of Legends', tags: ['league-of-legends', 'lol'], title: 'league of legends|ahri|akali|k[/-]?da|evelynn|miss fortune|caitlyn|seraphine' },
  { key: 'street-fighter', name: 'Street Fighter', tags: ['street-fighter'], title: 'street fighter|chun[- ]?li|cammy|juri|kimberly jackson' },
  { key: 'marvel-rivals', name: 'Marvel Rivals', tags: ['marvel-rivals'], title: 'marvel rivals|psylocke|black widow|scarlet witch|invisible woman|emma frost|luna snow' },
  // Script style
  { key: 'music', name: 'Music / beat', tags: ['music', 'beat-based', 'music-based', 'music-video', 'hybrid-based', 'pmv', 'hmv', 'cock-hero', 'fap-hero'] },
  { key: 'action', name: 'Action', tags: ['action-based', 'hybrid-based'] },
  { key: 'cock-hero', name: 'Cock Hero / Fap Hero', tags: ['cock-hero', 'fap-hero'], title: 'cock ?hero|fap ?hero' },
  // Source
  { key: 'faptap', name: 'FapTap', fapTap: true },
];
