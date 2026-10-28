// Store — main process data store wrapping electron-conf
const { randomUUID } = require('crypto');

let Conf = null;
let conf = null;

const DEFAULTS = {
  settings: {
    handy: {
      connectionKey: '',
      defaultOffset: 0,
      slideMin: 0,
      slideMax: 100,
      scriptHostMode: 'local',
      syncRounds: 30,
    },
    player: {
      volume: 80,
      theme: 'dark',
      // Interface style — 'classic' (current look) or 'modern' (same palette,
      // modernised depth/spacing/motion). Orthogonal to `theme`. See
      // notes/features/SCOPE-modern-theme.md.
      uiStyle: 'classic',
      recentFiles: [],
      gapSkip: {
        mode: 'off',
        threshold: 10000,
      },
      upNext: {
        mode: 'auto',
        countdownSec: 10,
      },
      preferMultiAxis: 'single',
      smoothing: 'linear',
      speedLimit: 0,
      // Orgasm Switch (hold-X) behaviour:
      //   'hold'   — hold to ride the looping script, release returns to main
      //   'toggle' — press to start the finisher looping, press again to stop
      //              the device(s) (no return to the main script)
      orgasmSwitchMode: 'hold',
      // Pick a random script variation each time a multi-variant video
      // loads (zaikechi #209/#221). Beats pinned defaults while on.
      randomVariantOnPlay: false,
      // Carry the playback rate across videos instead of resetting to 1x
      // on each load. Off by default — resetting matches YouTube / VLC and
      // avoids "why is this at 2x?" the next morning — but a community
      // report (2026-08-05) uses non-1x constantly and re-picked it every
      // time. Read live by VideoPlayer at each load.
      rememberPlaybackSpeed: false,
      // Inline visualization overlays during normal playback (zaikechi
      // #209): TL = windowed timeline graph, HM = full-duration heatmap
      // strip. Independent toggles via the player's TL/HM buttons.
      inlineTimeline: false,
      inlineHeatmap: false,
      // Keep the video playing in a docked corner overlay while browsing
      // the library, instead of stopping on leave (mini-player). Default on.
      miniPlayer: true,
      // Linux only: VA-API hardware video decode. Default on (prior
      // behaviour). Turn off if video won't play — a broken VA-API driver
      // (esp. nvidia-vaapi) errors instead of falling back to software.
      // Read directly in main.js before app.whenReady (Chromium flags).
      hwVideoDecode: true,
      // i18n — locked decisions in notes/features/IMPL-multi-language.md
      // #2: default is English, not 'auto'.
      // #3: _localeOfferedFor tracks the locale that's already been
      //     surfaced via the first-launch toast so we don't re-offer.
      language: 'en',
      _localeOfferedFor: null,
    },
    backend: {
      port: 5123,
      localIp: 'auto',
    },
    library: {
      directory: '',
      sources: [],
      associations: {},
      manualVariants: {},
      // Per-path manual VR-classification override. Value is `'vr'` or
      // `'flat'`; absent means use the auto-detection heuristic.
      // Tri-state covers both flipping a missed VR file → VR and
      // clearing a false-positive on a flat file with a VR-like token.
      manualVRType: {},
      // Per-path VR-as-flat playback override. Legacy two-string shape
      // — value was `'left'` or `'right'`. Kept indefinitely as a read
      // fallback for downgrade safety (a user reverting to <0.7.x still
      // sees their saved eye preference). New writes go to vrFormat
      // below; this key is not updated post-migration.
      vrFlatten: {},
      // Phase 1 of VR-flatten expansion (2026-05-21). Per-path richer
      // schema:
      //   { projection: 'sbs-half'|'sbs-full'|'tb-half'|'tb-full'|'flat',
      //     eye: 'left'|'right'|null,
      //     zoom: 1.0,
      //     source: 'auto'|'manual' }
      // Old vrFlatten entries lazy-migrate on read; no batch migration.
      // Non-planar projections (fisheye/equirect/MKX/RF52/EAC) reserved
      // for Phase 2a/2b — they appear disabled in the panel dropdown.
      vrFormat: {},
      // Per-path custom thumbnail override. Two shapes:
      //   `{ seekPct: 0..1 }` — pinned frame (legacy shape, no `type` key)
      //   `{ type: 'image', imagePath }` — user-uploaded poster, imported
      //     into userData/custom-thumbs by main.js (community #217)
      // EroScripts previews and FapTap thumbnails add `source`
      // ('eroscripts' | 'faptap'), see renderer/js/eroscripts-preview.js.
      // Absent means auto (10%-mark frame).
      customThumbnails: {},
      // Settled FapTap thumbnail imports per video path, so a thumbnail
      // that doesn't match isn't fetched again on every scan:
      //   `{ url, status: 'imported'|'no-match', distance? }`
      faptapThumbnails: {},
      // Per-path resume position — "continue where you left off"
      // (community request 2026-08-05). Shape:
      //   `{ position: seconds, duration: seconds, updatedAt: epochMs }`
      // Keyed by VIDEO PATH, not playlist, so the same video shares one
      // position everywhere it appears and library cards get progress for
      // free. Entries are written only between the thresholds in
      // renderer/js/resume-position.js and cleared on natural end.
      // `updatedAt` doubles as the last-played stamp.
      resumePositions: {},
      // Per-playlist "where was I" marker, so reopening a playlist shows
      // which video you were last on and can offer to continue from it.
      // Shape: `{ [playlistId]: { lastVideoPath, updatedAt } }`. Separate
      // from resumePositions because that map is keyed by video path and
      // one video can sit in several playlists.
      playlistProgress: {},
      // Per-path rating + favourite, keyed by VIDEO PATH like resumePositions.
      // Shape: `{ rating: 0..5 in 0.5 steps, favorite: bool, updatedAt: epochMs }`.
      // Written from the desktop, and from HereSphere / the web remote via the
      // backend's library-edit channel (renderer/js/library-edits.js). An entry
      // with rating 0 and no favourite is deleted rather than stored.
      ratings: {},
      // Show the resume progress bar on cards / list rows. On by default —
      // the feature is pointless invisible — but it IS a visible watch
      // record, so it stays switchable.
      showResumeProgress: true,
      // Collapse videos sharing an identical filename to a single grid
      // entry (the same file living in two source folders). Off by default.
      // Never hides a video outright — one copy always survives; see
      // dedupeByName in renderer/js/library-search.js.
      hideDuplicateNames: false,
      // EroScripts tags of the post each video's script came from, keyed by
      // VIDEO PATH. Recorded on download from any route (standalone panel or
      // the Associate-modal search), from `eroscripts:sync` links and from
      // Discovery sidecars. Shape:
      //   `{ tags: string[], topicId, topicUrl, source, savedAt }`
      // FapTap scenes (`source: 'faptap-scout'`) have `topicId: null` and
      // add `faptapId`, `faptapUrl` and FapTap's thumbnail as `previewUrl`.
      // Last write wins — the record describes the CURRENT script's post.
      scriptTags: {},
      // Preference score per VIDEO PATH (renderer/js/library-scores.js): the
      // scouts' ranking applied to each scene, recomputed after each scan and
      // when the backend's VR / axis decisions arrive. Derived; kept so the
      // badges and the score sort work right after a start.
      preferenceScores: {},
      // The user's score per post tag (lower case) → -10 … +10 in 0.5 steps,
      // added to every scene's preference score and to the scouts' ranking.
      // Absent = 0. Set from the web remote's Tags view (library edits).
      tagScores: {},
      collections: [],
      activeCollectionId: null,
    },
    security: {
      // Hide media file NAMES in main.log; directory and extension stay.
      // Off by default. See electron/log-redact.js for the reasoning.
      hideLogFileNames: false,
    },
    editor: {
      defaultCreator: '',
      patternPresets: [],
      // Pop-out window — size only, not position. Multi-monitor users
      // move setups around; restoring at last-saved coords on a setup
      // that no longer has that monitor lands the window off-screen.
      popoutBounds: null,
    },
    knownDevices: [],
    buttplug: {
      port: 12345,
    },
    tcode: {
      transport: 'serial',  // 'serial' | 'udp' | 'websocket'
      port: '',             // serial port path (legacy field, still in use)
      baudRate: 115200,
      udpHost: '',          // 2.4 GHz wireless ESP-NOW bridge etc.
      udpPort: 0,
      wsUrl: '',            // ws://device.local:81  (also restim/MFP-consumer URLs)
      precision: 3,         // 3 = TCode-0.2 (L0500), 4 = TCode-0.3 (L05000)
      updateRateHz: 60,     // keyframe poll/detect rate; higher = fast vibration survives on OSR2+/SR6 (advanced)
      axisRanges: {},
      axisEnabled: {},
    },
    autoblow: {
      token: '',
      offset: 0,
    },
  },
  playlists: [],
  categories: [],
  videoCategories: {},
  _migrated: false,
};

// electron-conf is ESM-only → dynamic import. The import itself is the
// expensive half of initStore (~1s cold on the startup trace), and it
// touches nothing app-specific (no userData read — only `new Conf` does
// that), so main.js kicks it off at module load via preloadModule() and
// it resolves in parallel with Chromium init + the recovery sweep.
let _confModulePromise = null;

function preloadModule() {
  if (!_confModulePromise) _confModulePromise = import('electron-conf');
  return _confModulePromise;
}

async function initStore() {
  const mod = await preloadModule();
  Conf = mod.default || mod.Conf;
  conf = new Conf({ defaults: DEFAULTS });
  return conf;
}

/**
 * Subscribe to every config write. Used by data-backup to schedule a
 * debounced snapshot after significant mutations. Returns the
 * unsubscribe function from electron-conf.
 *
 * Called by main.js right after initStore() so the very first write
 * (which is often the migration) is observed too.
 */
function subscribe(callback) {
  if (!conf || typeof conf.onDidAnyChange !== 'function') {
    return () => {};
  }
  return conf.onDidAnyChange(callback);
}

function getAll() {
  return JSON.parse(JSON.stringify({
    settings: conf.get('settings'),
    playlists: conf.get('playlists'),
    categories: conf.get('categories'),
    videoCategories: conf.get('videoCategories'),
    _migrated: conf.get('_migrated'),
  }));
}

function getSetting(path) {
  return conf.get(`settings.${path}`);
}

function setSetting(path, value) {
  conf.set(`settings.${path}`, value);
}

function addRecentFile(filePath) {
  const recent = conf.get('settings.player.recentFiles') || [];
  const filtered = recent.filter((f) => f !== filePath);
  filtered.unshift(filePath);
  conf.set('settings.player.recentFiles', filtered.slice(0, 20));
}

// --- Playlists ---

function getPlaylists() {
  return conf.get('playlists') || [];
}

function getPlaylist(id) {
  const playlists = getPlaylists();
  return playlists.find((p) => p.id === id) || null;
}

function addPlaylist(name) {
  const playlists = getPlaylists();
  const playlist = {
    id: randomUUID(),
    name,
    createdAt: Date.now(),
    videoPaths: [],
  };
  playlists.push(playlist);
  conf.set('playlists', playlists);
  return playlist;
}

function deletePlaylist(id) {
  const playlists = getPlaylists().filter((p) => p.id !== id);
  conf.set('playlists', playlists);
}

function renamePlaylist(id, name) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (playlist) {
    playlist.name = name;
    conf.set('playlists', playlists);
  }
}

function setPlaylistLoop(id, loop) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (playlist) {
    playlist.loop = !!loop;
    conf.set('playlists', playlists);
  }
}

function setPlaylistShuffle(id, shuffle) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (playlist) {
    playlist.shuffle = !!shuffle;
    conf.set('playlists', playlists);
  }
}

// "Balance shuffle by script" (zaikechi #221): videos sharing an
// associated funscript occupy one slot in this playlist's shuffle.
function setPlaylistBalance(id, balance) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (playlist) {
    playlist.balanceByScript = !!balance;
    conf.set('playlists', playlists);
  }
}

// "Prefer unwatched" — shuffle draws videos you haven't finished before
// ones you have. Watched items still play, just last (see
// renderer/js/playlist-progress.js partitionByWatched).
function setPlaylistPreferUnwatched(id, prefer) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (playlist) {
    playlist.preferUnwatched = !!prefer;
    conf.set('playlists', playlists);
  }
}

function addVideoToPlaylist(id, videoPath) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (playlist && !playlist.videoPaths.includes(videoPath)) {
    playlist.videoPaths.push(videoPath);
    conf.set('playlists', playlists);
  }
}

/** Append `videoPaths` the playlist doesn't hold yet, in order, with one write. */
function addVideosToPlaylist(id, videoPaths) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (!playlist || !Array.isArray(videoPaths)) return;
  const have = new Set(playlist.videoPaths);
  const fresh = videoPaths.filter((p) => typeof p === 'string' && p && !have.has(p) && have.add(p));
  if (!fresh.length) return;
  playlist.videoPaths.push(...fresh);
  conf.set('playlists', playlists);
}

/**
 * Replace a playlist's videoPaths with a new ordering. Used by drag/Move
 * reorder. Validates that `newVideoPaths` is the same SET as the current
 * paths (a reorder, not an add/remove) — defends against a stale renderer
 * array silently dropping or duplicating entries.
 */
function setPlaylistVideoPaths(id, newVideoPaths) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (!playlist || !Array.isArray(newVideoPaths)) return;
  const before = [...playlist.videoPaths].sort();
  const after = [...newVideoPaths].sort();
  if (before.length !== after.length || before.some((p, i) => p !== after[i])) {
    return; // not a pure reorder — refuse rather than corrupt the playlist
  }
  playlist.videoPaths = [...newVideoPaths];
  conf.set('playlists', playlists);
}

function removeVideoFromPlaylist(id, videoPath) {
  const playlists = getPlaylists();
  const playlist = playlists.find((p) => p.id === id);
  if (playlist) {
    playlist.videoPaths = playlist.videoPaths.filter((p) => p !== videoPath);
    conf.set('playlists', playlists);
  }
}

// --- Categories ---

function getCategories() {
  return conf.get('categories') || [];
}

function addCategory(name, color, icon) {
  const categories = getCategories();
  const category = {
    id: randomUUID(),
    name,
    color,
  };
  // Only written when actually chosen. Absent means "plain colour dot",
  // which is what every pre-existing category has, so the stored shape
  // stays identical for anyone who never picks an icon.
  const clean = _cleanIcon(icon);
  if (clean) category.icon = clean;
  categories.push(category);
  conf.set('categories', categories);
  return category;
}

/**
 * Validate an icon before it reaches config.json.
 *
 * The renderer already normalises, but this is the last gate before user
 * config is written and a malformed value here would render as a blank
 * category on every surface until hand-edited out.
 */
function _cleanIcon(icon) {
  if (!icon || typeof icon !== 'object') return null;
  if (icon.type === 'shape') {
    return typeof icon.value === 'string' && icon.value ? { type: 'shape', value: icon.value } : null;
  }
  if (icon.type === 'emoji') {
    const v = String(icon.value || '').trim();
    // Cap the stored length. Graphemes are multi-code-unit so this is a
    // sanity bound, not a character count — the renderer does the precise
    // single-grapheme truncation.
    return v && v.length <= 16 ? { type: 'emoji', value: v } : null;
  }
  return null;
}

/**
 * Set or clear a category's icon. `null` clears it back to a colour dot.
 * Editing existing categories is the case that actually matters — an icon
 * you can only set at creation time is useless to anyone who already has
 * categories.
 */
function setCategoryIcon(id, icon) {
  const categories = getCategories();
  const category = categories.find((c) => c.id === id);
  if (!category) return;
  const clean = _cleanIcon(icon);
  if (clean) category.icon = clean;
  else delete category.icon;
  conf.set('categories', categories);
}

/** Set a category's colour. */
function setCategoryColor(id, color) {
  const categories = getCategories();
  const category = categories.find((c) => c.id === id);
  if (!category || typeof color !== 'string' || !color) return;
  category.color = color;
  conf.set('categories', categories);
}

function deleteCategory(id) {
  const categories = getCategories().filter((c) => c.id !== id);
  conf.set('categories', categories);
  // Clean up videoCategories references
  const mappings = conf.get('videoCategories') || {};
  for (const path of Object.keys(mappings)) {
    mappings[path] = mappings[path].filter((cid) => cid !== id);
    if (mappings[path].length === 0) delete mappings[path];
  }
  conf.set('videoCategories', mappings);
}

function renameCategory(id, name) {
  const categories = getCategories();
  const category = categories.find((c) => c.id === id);
  if (category) {
    category.name = name;
    conf.set('categories', categories);
  }
}

// --- Category Mappings ---

function assignCategory(videoPath, catId) {
  const mappings = conf.get('videoCategories') || {};
  if (!mappings[videoPath]) mappings[videoPath] = [];
  if (!mappings[videoPath].includes(catId)) {
    mappings[videoPath].push(catId);
    conf.set('videoCategories', mappings);
  }
}

/** Assign many `[videoPath, catId]` pairs in one write. */
function assignCategories(pairs) {
  const mappings = conf.get('videoCategories') || {};
  let changed = false;
  for (const [videoPath, catId] of Array.isArray(pairs) ? pairs : []) {
    if (typeof videoPath !== 'string' || !videoPath || typeof catId !== 'string' || !catId) continue;
    if (!mappings[videoPath]) mappings[videoPath] = [];
    if (mappings[videoPath].includes(catId)) continue;
    mappings[videoPath].push(catId);
    changed = true;
  }
  if (changed) conf.set('videoCategories', mappings);
}

function unassignCategory(videoPath, catId) {
  const mappings = conf.get('videoCategories') || {};
  if (mappings[videoPath]) {
    mappings[videoPath] = mappings[videoPath].filter((id) => id !== catId);
    if (mappings[videoPath].length === 0) delete mappings[videoPath];
    conf.set('videoCategories', mappings);
  }
}

function getVideoCategories(videoPath) {
  const mappings = conf.get('videoCategories') || {};
  return mappings[videoPath] || [];
}

function getVideosByCategory(catId) {
  const mappings = conf.get('videoCategories') || {};
  const paths = [];
  for (const [path, catIds] of Object.entries(mappings)) {
    if (catIds.includes(catId)) paths.push(path);
  }
  return paths;
}

// --- Migration helper (direct conf access for root-level keys) ---

function isMigrated() {
  return conf.get('_migrated') === true;
}

function migrateFromLegacy(legacyData) {
  if (!legacyData || typeof legacyData !== 'object') return;

  // Settings
  const settingKeys = ['handy', 'player', 'backend', 'library'];
  for (const key of settingKeys) {
    if (legacyData[key] && typeof legacyData[key] === 'object') {
      for (const [subKey, value] of Object.entries(legacyData[key])) {
        conf.set(`settings.${key}.${subKey}`, value);
      }
    }
  }

  // Playlists
  if (Array.isArray(legacyData.playlists)) {
    const valid = legacyData.playlists
      .filter((p) => p && p.id && p.name)
      .map((p) => ({
        id: p.id,
        name: p.name,
        createdAt: p.createdAt || Date.now(),
        videoPaths: Array.isArray(p.videoPaths) ? p.videoPaths : [],
      }));
    conf.set('playlists', valid);
  }

  // Categories
  if (Array.isArray(legacyData.categories)) {
    const valid = legacyData.categories
      .filter((c) => c && c.id && c.name)
      .map((c) => ({
        id: c.id,
        name: c.name,
        color: c.color || '#3498db',
      }));
    conf.set('categories', valid);
  }

  // Video category mappings
  if (legacyData.videoCategories && typeof legacyData.videoCategories === 'object') {
    const clean = {};
    for (const [path, catIds] of Object.entries(legacyData.videoCategories)) {
      if (Array.isArray(catIds) && catIds.length > 0) {
        clean[path] = catIds;
      }
    }
    conf.set('videoCategories', clean);
  }

  conf.set('_migrated', true);
}

module.exports = {
  DEFAULTS,
  initStore,
  preloadModule,
  subscribe,
  getAll,
  getSetting,
  setSetting,
  addRecentFile,
  getPlaylists,
  getPlaylist,
  addPlaylist,
  deletePlaylist,
  renamePlaylist,
  setPlaylistLoop,
  setPlaylistShuffle,
  setPlaylistBalance,
  setPlaylistPreferUnwatched,
  addVideoToPlaylist,
  addVideosToPlaylist,
  removeVideoFromPlaylist,
  setPlaylistVideoPaths,
  getCategories,
  addCategory,
  deleteCategory,
  renameCategory,
  setCategoryIcon,
  setCategoryColor,
  assignCategory,
  assignCategories,
  unassignCategory,
  getVideoCategories,
  getVideosByCategory,
  isMigrated,
  migrateFromLegacy,
};
