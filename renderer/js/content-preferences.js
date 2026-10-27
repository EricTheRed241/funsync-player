// Content preferences: every choice about which videos and scripts the
// library takes, ranks, keeps and how it sorts them, in one place.
//
// Read by the scouts (scripts/eroscripts-scout-lib.mjs,
// scripts/faptap-scout-lib.mjs, scripts/scout-live.mjs), the duplicate
// cleanup (scripts/scene-dedupe-lib.mjs) and the app (renderer/js/
// reject-rule.js, ratings.js, artist-playlists.js, auto-categories.js,
// auto-collections.js). Those modules hold the logic; the values live here.
// Changing a screening rule here means bumping RULES_VERSION in
// scripts/eroscripts-scout-lib.mjs (and FAPTAP_RULES_VERSION in
// scripts/faptap-scout-lib.mjs) so judged posts are judged again.
//
// Tags are EroScripts slugs (FapTap tags are aliased to them first). Title
// and text patterns match whole words only: `(?<![\p{L}\p{N}])…(?![\p{L}\p{N}])`.
//
// One value has a copy outside JavaScript: TOP_RATED_MIN is mirrored in
// backend/services/vr_sections.py.

// ── Discovery: what never comes in ──────────────────────────────────────

/** Excluded content, by tag, title or first post / description. Hyphenated tags (`futa-on-male`) match, `Futaba` doesn't. */
export const EXCLUDED_TEXT = /(?<![\p{L}\p{N}])(loli|lolicon|shota|shotacon|furry|furries|anthro|yiff|futa|futas|futanari|dickgirls?)(?![\p{L}\p{N}])/iu;
/** FapTap tags that are furry without saying so. */
export const FURRY_TAGS = new Set([
  'scalie', 'knot', 'knotted', 'horse', 'dog', 'animal', 'my-little-pony', 'animal-crossing', 'krystal-star-fox',
  'renamon-digimon', 'five-nights-at-freddy-s', 'roxanne-wolf-five-nights-at-freddy-s',
]);
/** Monsters, by tag or title word (`X WW` = werewolf; EroScripts also by these words in the first post), favourites and picks included. Succubi, demon girls and elves aren't monsters. */
export const MONSTER_TAGS = new Set([
  'monster', 'monsters', 'monstergirls', 'monstergirl', 'monster-girl', 'monster-girls', 'tentacle', 'tentacles', 'tentacle-sex',
  'tentacle-monster', 'werewolf', 'werewolves', 'horse', 'horse-cock', 'horsecock', 'bestiality', 'animated-bestiality', 'zoophilia',
  'orc', 'orcs', 'goblin', 'goblins', 'minotaur', 'ogre', 'troll', 'alien', 'aliens', 'beast', 'slime-girl', 'knot', 'knotted', 'knotting',
]);
export const MONSTER_TITLE = /(?<![\p{L}\p{N}])(monsters?|monster ?girls?|tentacles?|w[ae]rewol(?:f|ves)|horse ?guys?|horse ?cocks?|orcs?|goblins?|minotaurs?|ogres?|trolls?|aliens?|beasts?|bestiality)(?![\p{L}\p{N}])|触手/iu;
/** `Mercy X WW`: Solace2K and Rekin3D's short for werewolf, a standalone capital token. */
export const WEREWOLF_TOKEN = /(?<![\p{L}\p{N}])WW(?![\p{L}\p{N}])/u;
/** Feet, by tag or title word (EroScripts also in the first post): never. */
export const FEET_TAGS = new Set(['footjob', 'feet', 'foot', 'foot-fetish', 'foot-worship', 'footsie', 'soles', 'toes']);
export const FEET_TITLE = /(?<![\p{L}\p{N}])(feet|foot|footjobs?|toes|soles|footsie)(?![\p{L}\p{N}])|足コキ/iu;
/** Tags that say a post has no porn in it: never taken. */
export const SFW_TAGS = new Set(['sfw', 'safe-for-work', 'non-porn', 'nonporn', 'non-nude']);
/** FapTap audio roleplays (erotic audio / ASMR over a still or a loop): never taken. */
export const AUDIO_TITLE = /(?<![\p{L}\p{N}])(erotic audio|asmr|audio role ?play|f4m)(?![\p{L}\p{N}])/iu;
/** Audio roleplay tags: `asmr-audio-roleplay`, `erotic-audio-for-men`, `lewdlexiaudio`. */
export const isAudioRoleplayTag = (t) => t.includes('asmr') || t.includes('audio-roleplay') || (t.endsWith('audio') && t !== 'audio');
/** Shortest video worth downloading, by its main script's last action (FapTap also by listed length). */
export const MIN_VIDEO_MS = 120_000;

// ── Discovery: animated, sex required ───────────────────────────────────

/** Animation tags: a post with one (and no `real` tag) is animated; anything else is live action. AI video counts. */
export const ANIMATION_TAGS = new Set(['animation', '3d', 'hentai', 'vam', 'sfm', 'mmd', 'cgi', 'cartoon', 'blender', 'animate', 'anime', 'koikatsu', 'honey-select']);
/** More animation tags, as FapTap uses them. */
export const FAPTAP_ANIMATION_TAGS = new Set(['2d', 'ai-video', 'iwara']);
/** FapTap tags that make a video live action even with an animation tag (also any `amateur…` tag, and FapTap models / pornstars as performers). */
export const LIVE_TAGS = new Set([
  'real', 'real-sex', 'real-couple-homemade', 'real-orgasm', 'reality', 'homemade', 'pornstar', 'onlyfans', 'brazzers',
  'blacked', 'chaturbate', 'webcam', 'verified-amateurs', 'jav', 'celebrity', 'tiktok', 'cosplay', 'anime-cosplay', 'kbj',
  'egirl', 'sislovesme', 'dadcrush',
]);
/** Penetrative sex tags: a post with one is sex. Blowjob, handjob and titjob alone don't count. Required of every post, favourites and picks too. */
export const SEX_TAGS = new Set([
  'sex', 'vaginal', 'vaginal-sex', 'anal', 'anal-sex', 'riding', 'cowgirl', 'reverse-cowgirl', 'doggy', 'doggystyle', 'doggy-style',
  'standing-doggy', 'missionary', 'mating-press', 'pronebone', 'prone-bone', 'spooning', 'creampie', 'anal-creampie', 'cum-inside',
  'cum-in-pussy', 'cum-in-ass', 'double-penetration', 'penetration', 'intercourse', 'threesome', 'foursome', 'fivesome', 'gangbang',
  'orgy', 'group-sex', 'breeding', 'nakadashi',
]);
/** Sex words in a title (EroScripts: else in the first post, saved as `sexual: 'text'`), looked for after NON_SEX_PHRASES are taken out. */
export const SEX_WORDS = /(?<![\p{L}\p{N}])(sex|segs|fuck|fucks|fucked|fucking|riding|rides?|cowgirl|doggy\w*|missionary|creampies?|anal|vaginal|mating ?press|prone ?bone|intercourse|penetration|gangbang\w*|threesome|foursome|orgy|orgies|nakadashi|breeding|cum ?inside)(?![\p{L}\p{N}])/iu;
/** Sex words in scripts without spaces between words. */
export const SEX_WORDS_CJK = /セックス|エッチ|中出し|騎乗|正常位|做爱|性爱|骑乘|后入|中出|内射|섹스|내사/u;
/** Acts that aren't sex (`Face Fuck PMV`, `Titjob HMV`), removed from a title before sex words are looked for. */
export const NON_SEX_PHRASES = /(?<![\p{L}\p{N}])(?:(?:face|throat|mouth|skull|tit|titty|boob|thigh|armpit|hand|foot|feet)[ -]?fuck\w*|oral[ -]?sex)/giu;
/** A music video (PMV_TAGS / PMV_TITLE) counts as sex unless it's a dance or its title names one of these non-sex acts. */
export const NON_SEX_ACT_TITLE = /(?<![\p{L}\p{N}])(blowjobs?|blow ?job|bj|handjobs?|hand ?job|hj|titjobs?|tit ?job|titfuck\w*|tittyfuck\w*|boobjobs?|paizuri|joi|deepthroat\w*|(?:face|throat|mouth|skull) ?fuck\w*|fellatio|oral)(?![\p{L}\p{N}])|フェラ|パイズリ|手コキ|口交|乳交/iu;
/** A title that says the video is a dance counts like the `dance` tag. */
export const DANCE_TITLE = /(?<![\p{L}\p{N}])danc(e|es|ing)(?![\p{L}\p{N}])|ダンス|踊り|舞蹈|跳舞|댄스/iu;

// ── Discovery: scripts and axes ─────────────────────────────────────────

/** EroScripts feeds the scout lists (Free Scripts tagged …). No hentai feed: hentai comes in through the others. */
export const FEED_TAGS = ['multi-axis', 'animation', '3d', 'vam', 'sfm', 'mmd'];
/** PMV / HMV / Cock Hero / Fap Hero, by tag … */
export const PMV_TAGS = new Set(['pmv', 'hmv', 'cock-hero', 'fap-hero', 'music-video']);
/** … or by title. */
export const PMV_TITLE = /(?<![\p{L}\p{N}])(pmv|hmv|cock ?hero|fap ?hero)(?![\p{L}\p{N}])/iu;
/**
 * EroScripts takes multi-axis scenes only. A single-axis scene comes in only
 * when the post is VR, has creampie (a `creampie` / `…-creampie` tag or this
 * title) and is by a creator already in the library or a PMV_TAGS / PMV_TITLE
 * video. Single-axis scenes that come in get generated twist / roll / pitch;
 * hand-made axes always win, and a partial multi-axis scene is left as made.
 */
export const SINGLE_AXIS_CREAMPIE_TITLE = /(?<![\p{L}\p{N}])creampies?(?![\p{L}\p{N}])|中出し|内射/iu;
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
 * Favourite creators and actresses: their live action is always taken,
 * they skip the FapTap focus rule, and the app gives each an artist
 * playlist from their first video plus the FAVOURITES_PLAYLIST. Matched in
 * titles, tags, uploaders and performers (scripts/scout-live.mjs favouriteIn).
 */
export const FAVOURITES = ['ScyllaHMV', 'Agapilord', 'Eternal Enigma', 'Cuddle Mocap', 'Noodledude', 'NoFaceGirl', 'Candy Love', 'JJULEZ'];
/** Other live action: one in every LIVE_QUOTA_EVERY Discovery downloads, counted across both scouts. */
export const LIVE_QUOTA_EVERY = 20;

// ── Discovery: ranking ──────────────────────────────────────────────────

/** Music tags (+ RANKING.music; a library creator's music video also skips the live quota). */
export const MUSIC_TAGS = new Set(['music', 'pmv', 'hmv', 'beat-based', 'music-based', 'music-video', 'cock-hero', 'fap-hero', 'compilation']);
/** Preferred acts (+ RANKING.preferred, + RANKING.preferredExtra for each one more). */
export const PREFERRED_TAGS = new Set(['creampie', 'riding']);
/** Candidate score; higher downloads first. Likes add likes / likesPer, up to likesMax; 2D hentai only loses hentaiOnly. */
export const RANKING = { vr: 3, music: 2, artistInLibrary: 2, preferred: 2, preferredExtra: 0.5, multiAxis: 1, likesPer: 20, likesMax: 3, hentaiOnly: 3 };
/** 2D hentai only: animation tags among these, no THREE_D_TAGS and no THREE_D_TITLE. */
export const FLAT_HENTAI_TAGS = new Set(['hentai', '2d', 'anime']);
export const THREE_D_TAGS = new Set(['3d', 'sfm', 'vam', 'mmd', 'cgi', 'blender', 'koikatsu', 'honey-select']);
export const THREE_D_TITLE = /(?<![\p{L}\p{N}])(3d|sfm|vam|mmd)(?![\p{L}\p{N}])/iu;

// ── Video quality ───────────────────────────────────────────────────────

/** VR playback caps (heavier files stutter in the headset): ≤ 8K, ≤ 60 fps, the low bitrate when offered. */
export const VR_MAX_HEIGHT = 4320;
export const VR_MAX_FPS = 60;
/** Flat: ≤ 4K, otherwise the highest quality. */
export const FLAT_MAX_HEIGHT = 2160;
/** Lowest FapTap stream worth downloading (lower ones are preview uploads). */
export const MIN_STREAM_HEIGHT = { flat: 720, vr: 1080 };

// ── Duplicates: which copy stays ────────────────────────────────────────

/**
 * One copy per video. The first difference decides (names are the reasons
 * scripts/scene-dedupe-lib.mjs prints): EroScripts or a library scene over
 * FapTap, hand-made axes over generated, passthrough over none, fewer
 * breaches of the VR caps, more pixels, more fps, the faster main script,
 * the higher bitrate.
 */
export const DEDUPE_KEEP_ORDER = [
  'EroScripts over FapTap', 'hand-made axes', 'passthrough', 'within VR caps',
  'higher resolution', 'higher fps', 'faster script', 'higher bitrate',
];

// ── Ratings and triage ──────────────────────────────────────────────────

/** Rated this or lower (and not a favourite): deleted from disk, scripts and axes too, and the scouts skip the post. */
export const DELETE_AT_OR_BELOW = 2;
/** Rated this or higher: kept (out of Review). In between: the REVIEW_PLAYLIST. */
export const KEEP_FROM = 4;
export const REVIEW_PLAYLIST = 'Review';
/** "Top rated" filter and the headset's Top Rated section (mirrored in backend/services/vr_sections.py). */
export const TOP_RATED_MIN = 4;

// ── Artist playlists ────────────────────────────────────────────────────

/** An artist folder with at least this many videos gets a playlist ("more than six"); favourites from one. */
export const MIN_ARTIST_VIDEOS = 7;
export const FAVOURITES_PLAYLIST = 'Favourite artists';
/** Folder names that hold videos of many artists (lower case). */
export const NOT_AN_ARTIST = new Set(['unknown', 'misc', 'various', 'other', 'others', 'new folder', 'cock hero', 'fap hero', 'pmv', 'hmv', 'compilation', 'compilations']);

// ── Auto categories ─────────────────────────────────────────────────────

/** Categories filled from post tags after each scan (VR from the backend's VR decision instead). */
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
  { key: 'pov', name: 'POV', color: '#00cec9', tags: ['pov', 'male-pov'] },
];

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
