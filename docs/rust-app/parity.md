# FunSync in Rust: parity with the Electron app

The Rust app (`crates/funsync-server`, http://127.0.0.1:5300/) is built beside the Electron app (port 5123) until it can replace it. One row per user-visible feature of `README.md`; the README section is the reference. Status: **done**, **partial** (what's there / what's missing), **not started**, **not planned** (why).

Ground rules while both run: the Rust app has its own data folder (`~/Library/Application Support/funsync-rust/`, seeded by `funsync-server import`, which only reads the old `config.json`) and treats the library as read-only (no deletes, renames, rating deletes or sidecar writes). Downloads, scouts and jobs stay with the Electron app.

The **parity dashboard** at http://127.0.0.1:5300/parity (nav bar: Parity) shows the counts, a progress bar and these tables (the server reads this file at build time), plus the server's own state.

## Server and data

| Feature | Status | Notes |
|---|---|---|
| Own data folder, `library.json` | done | `--data-dir`, `$FUNSYNC_RUST_DATA` |
| Import from the old app (read-only) | done | sources, playlists, categories, `videoCategories`, the old `settings.library` (ratings, associations, scores, tag scores …) kept verbatim; the EroScripts session is not copied. `video-tags.json` / `pornstar-tags.json` are read live in phase 3 |
| Health / status endpoint | done | `GET /health`, `GET /api/status` (sources present, videos, scan time, unreadable sources) |
| Status API with devices, issues (`/api/status` of the old backend) | done | `/api/status` adds `issues` (TCode link waiting / reconnecting with attempts and last failure, no device connected), `devices` (TCode link: state, URL, since, attempts, retry, last error, commands and bytes; Buttplug devices), `session`, `phone`, `backend`, `desktop` (the server's engine, always connected); no VR bridge / headset parts |
| Device dashboard (`/device`) | done | `backend/device-dashboard/` served unchanged (this computer only; 403 otherwise), fed by `GET /api/device/telemetry` from the server's engine: 1 s samples for 15 minutes, the last ~100 s of commands, 500 events (link connects, drops, failures, scripts loaded, phone take-overs); the web remote's newest phone is the playback source |
| Access PIN, LAN mode | partial | `--lan` listens on every interface; the swipe deck's gate (`scout_report::gate`) with the old app's `access-pin.json`, so one login covers both apps (login page, cookie / `?k=`, lockout). Missing: headset body login and thumbnail-only keys (VR slice), WebSocket close 4401 (a refused socket gets 401) |
| One port + nav bar for the Rust sites (swipe deck, Downloads, creators, status) | done | `src/sites.rs`: `deploy/sites.json` relayed under their prefixes (base href + nav injected, `Location` mapped, bare page paths redirect), nav from the same file plus Parity |
| Log file | done | `~/Library/Logs/funsync-rust/server.log` (`$FUNSYNC_RUST_LOG`), rotated to `server.old.log` at 1 GB; every request (client, method, path, status, time) except a successful loopback `/health`, plus the server's own lines |

## Video Player

| Feature | Status | Notes |
|---|---|---|
| Playback with range streaming | done | `GET /api/media/stream/{id}` (Range, 206), played by the web remote's player |
| Custom controls, keyboard shortcuts, drag-and-drop | partial | the web remote's controls plus `web/remote/extras.js` (added to its page by the server): Space/K, ←/→, J/L, ↑/↓, M, F, S, P, R, G/Shift+G. Not ported: drag-and-drop / O (open a file outside the library), H, I, A/B and V keys (the player's own buttons do loops and variants) |
| Seek bar thumbnail preview | not started | |
| Subtitles (.srt / .vtt, SRT→VTT) | done | `GET /api/media/subtitle/{id}`, shown by the web remote |
| Screenshot, picture-in-picture, aspect ratio cycling | done | `extras.js`: S saves a PNG of the frame, P picture-in-picture, R cycles contain / cover / fill |
| A-B loop | done | web remote player |
| Replay button at the end | done | `extras.js`: a Replay button over the player when a video ends (and Space replays) |
| Script gap auto-skip | done | web remote player |

## Play queue

| Feature | Status | Notes |
|---|---|---|
| One queue (user additions + auto-fill to 10) | done | `src/queue.rs`, port of `play-queue.js` (`player.userQueue` / `queuePlayed` / `queueAutoFill`, auto-fill from `preferenceScores`, review playlists and the delete rating skipped) |
| Queue on every client (page, headset section, `Queue:Up next` tag) | done | the web remote's Queue tab and queue buttons; the HereSphere / DeoVR Queue section (your queue + auto-fill) and the `Queue:Up next` tag, editable in HereSphere |
| Queue API (`/api/remote/queue*`) | done | `GET /queue`, `add` / `remove` / `played` / `clear` / `auto-fill`, applied at once |

## Library

| Feature | Status | Notes |
|---|---|---|
| Library scan (pairing, variants, axis companions, subtitles, sidecars, fuzzy pass, AppleDouble skip, on-disk spelling) | done | port of `electron/library-scan.js`; on the real library (999 videos) every pairing, variant, subtitle, source and sidecar matches the JS scan |
| Generated thumbnails (10 % / ≥ 10 s, bad-frame retries, shared cache) | partial | rule ported, `thumb-cache/` with the old names, VR one-eye / centre crops (`_sbs`, `_sbsc`, `_tb`, `_tbc`) from the VR decision; versioned URLs (`?v=`, immutable caching) in the web remote listing; no pinned frames, no cache pruning; headset URLs not versioned yet |
| Posters and custom thumbnails (EroScripts, FapTap, rule34video) | partial | posters the old app saved (`customThumbnails`, files in its `custom-thumbs/`, read-only) are served; fetching new ones (EroScripts previews, FapTap, rule34video) is not ported |
| Grid / list / folder views, hover preview | partial | the web remote's grid, list and folder views; no hover preview |
| Sources: reorder, enable/disable, overlap, hot-plug | done | Settings page (`/remote/settings.html`, `src/settings.rs`): add, remove, enable, reorder, overlap warning; a source folder appearing or disappearing triggers a rescan (checked every 10 s) |
| Manual script association | not started | |
| Fuzzy search with exact-title precedence | done | the web remote's search (`library-search.js`) |
| Sort (preference score default, name, date, duration, speed, rating) | partial | the web remote's sorts; scores are the imported snapshot until the server computes them |
| Playlists | done | listed and played; add / remove from the phone (`/api/remote/playlists/add-video`, `remove-video`); create, rename, delete, reorder and bulk add / remove on the Lists page (`/remote/manage.html`, `POST/DELETE /api/groupings/<kind>/<id>[/videos]`) |
| Categories from EroScripts tags, VR, axis categories | partial | the imported categories are listed (`/api/remote/categories`); not computed by the server yet |
| Artist playlists, aliases, playlist cleanup | not started | phase 3 |
| Auto collections, score lists | partial | the imported collections are listed; not computed by the server yet |
| Smart playlists (VR picks, Quick ones, Favorites) | not started | phase 3 |
| Preference score and tag scores | not started | phase 3 (Rust `candidateScore` exists in `crates/slr-scout/src/score.rs`) |
| Multi-select bulk assignment | done | Lists page: tick videos (search, "only in this list"), then add to / remove from any playlist, category (with colour) or collection |
| Ratings and favourites | done | `POST /api/remote/rating` (half stars, heart) saved to `library.ratings`; ≤ 2★ deletes nothing (read-only library) |
| Watched videos last, Watched collection | done | from the ratings (`watched`, `watchedLast`, the `:watched` collection) |
| Overview (capacity, ratings, artists, tag scores) | done | `/remote/overview.html` (nav: Overview): videos vs `libraryMaxVideos` with bar and GB per source, favourites / Keep / Unrated, star histogram, every rated or hearted video (rate it there, click to play); artists and tag scores open the web remote's Artists and Tags views |
| Triage deletes (≤ 2★), duplicate deletes | not planned | read-only library until the user says the Rust app takes over |

## Side files

| Feature | Status | Notes |
|---|---|---|
| Video tags merge (`video-tags.json`) | not started | phase 3 |
| Pornstar tags merge (`pornstar-tags.json`) | not started | phase 3 |
| Underage screener integration (review playlist, 1★) | not started | screener talks to the old backend today |

## Remote access / web UI

| Feature | Status | Notes |
|---|---|---|
| One web UI everywhere (phone first, desktop layout) | partial | the web remote (`backend/web-remote/`, unchanged, with `crates/funsync-server/web/remote/` overriding files when needed) at `/remote/`; desktop-only views (Overview, settings, devices) missing |
| Triage from the phone (rating, heart, add to playlist) | done | `src/edits.rs`, applied to the library at once (no desktop edit queue) |
| Review on end | done | the phone's flow (add to *Needs review* by name; a rating takes the video out) |
| Tags / Artists views, tag scores, favourite artists | partial | views, tag scores, favourite artists and bonus are stored; Recalculate only saves until the server computes scores |
| Player controls: own or browser | done | web remote |
| PWA install | done | the web remote's manifest and service worker |
| Local-only mode | done | loopback is the default; `--lan` opts in |

## VR content server

| Feature | Status | Notes |
|---|---|---|
| HereSphere API (sections, scene tags, scripts) | partial | `/heresphere` (`?filter=` vr, flat, scripted) and `/heresphere/<id>`: the old sections (Needs review, Queue with auto-fill, score lists, VR/Flat, speed, length, Hand-made, Recently Added, triage, playlists; watched last), scene media/scripts/subtitles/tags/chapters, URLs from the Host header; writable rating / heart / tags and `eventServer`. Hand-made from companion files only (no embedded axes); versioned thumbnail URLs (`?v=`); no access PIN |
| DeoVR API | partial | `/deovr` and `/deovr/<id>` with the same sections and tags; encoding name always `h264`; no access PIN |
| Rate / favourite / playlist / queue edits from the headset | done | HereSphere scene POST: rating, `isFavorite`, `Playlist:<name>` and `Queue:Up next` tags (only tags changed since the scene was served count); DeoVR is read-only, as before |
| Review on end from the headset | done | `POST /heresphere/events/<id>`: a play takes the video out of the queue (once per video); a pause / close at ≥ 90 % of an unrated video files it under Needs review |
| VR decision (projection, filename, frames, shape) | partial | VR Format projection, Mark as VR / Flat, filename rules (patterns, studio codes, `XXVR-123`, `VR180`, bare `VR`), ~2:1 ≥ 3200 wide fallback from probed dimensions (in memory, so a listing only sees videos probed since start); frame-based stereo detection (`stereo_detect.py`) not ported |
| PCVR companion bridge (HereSphere timestamp API) | not started | |
| Session status card, VR ↔ web remote mutex | not started | |

## Funscript support

| Feature | Status | Notes |
|---|---|---|
| Script loading (main, variant, axis) | done | `GET /api/media/script/{id}?variant=&axis=` serves the files (hand-made axis beats generated); the server plays the main script or the chosen variant with their axes on the devices |
| Heatmap and gap indicators on the seek bar | done | web remote |
| Script variants switch (V key) | partial | from the web remote's player (`switch-variant` → `variant-changed`): the server plays the variant with its own axis files, the video's hand-made axes fill in and beat generated ones; no V key (no desktop player) |
| Multi-axis (TCode 10 axes, embedded axes) | done | server engine (`src/devices/`): companion axes L1/L2/R0/R1/R2/V0/V1/V2/A0 and axes embedded in the main script (`additional_axes`, `raw`, `axes`, `channels`, sibling keys, inline TCode) |
| Custom routing per device | not started | |
| Speed stats | partial | from the imported `speedStatsCache`; not computed for new scripts |

## Script smoothing

| Feature | Status | Notes |
|---|---|---|
| PCHIP / Makima interpolation | done | `settings.player.smoothing` (linear / pchip / makima), server engine |
| Speed limit | done | `settings.player.speedLimit` (units/s), every axis |

## Devices

| Feature | Status | Notes |
|---|---|---|
| Buttplug / Intiface from the page | partial | decision change: **the server drives every device**, from the playback state the web remote streams over `/api/remote/sync`. Minimal Buttplug v3 client to Intiface (`ws://127.0.0.1:<buttplug.port>`): strokers follow the stroke (`LinearCmd`), vibrators its speed (`ScalarCmd`); no per-device modes, rotate or oscillate yet; not tested against a real Intiface |
| TCode WebSocket (MiraBot, 60 Hz, 4 digits) from the page | done | from the server (decision change above): 60 Hz `L09999I17` frames, per-axis enable / min / max / invert from `settings.tcode.axes`, 3 or 4 digits (`precision`); Devices page `/remote/devices.html` |
| TCode USB serial from the server | done | raw tty set up with `stty` (no serial crate), `settings.tcode.port` / `baudRate` |
| TCode UDP from the server | done | `udpHost` / `udpPort` |
| Browser serial relay | not planned | the server drives USB serial itself (decision 3); revisit if a device sits on another computer |
| TCode device profiles (MiraBot, FUNSR MINI, Custom) | done | `POST /api/devices {kind:"tcode", profile}` and the Devices page: saves the current profile under `tcode.profiles.saved`, loads the next one's (or its defaults) and connects it unless it was disconnected; FUNSR MINI stroke length / speed / start position (`DAG` / `DKP` / `DSP`) sent at once and 2.5 s after each connect; the device pill names the profile. No floor / ceiling, generated motion or browser relay |
| Auto-connect, WebSocket reconnect | done | `autoConnect` set by Connect / Disconnect; at start the server reconnects what was connected; retries every 5 s for a minute, then every minute |
| Per-device offsets, Sync tab | partial | `tcode.defaultOffset` / `buttplug.defaultOffset` from the Devices page and the player's device pill (`set-offset`); no Sync tab |
| Generated axes playback, hand-made axes win | done | generated companions play only when the script has no hand-made axis (companion or embedded) |

## EroScripts, scouts, jobs

| Feature | Status | Notes |
|---|---|---|
| EroScripts search / download in the app, login | not started | |
| Post previews as thumbnails | not started | |
| Scouts, swipe deck, downloads, creators, job runner | done | the sites are relayed on 5300; `--supervise` builds and runs them and `job-runner serve` like the desktop app (a site whose port already answers is left alone). The scouts and jobs themselves stay their own crates |
| `eroscripts:sync`, dedupe, reorganize, `upgrade:axes` CLIs | not planned | CLIs, unchanged; they keep writing side files both apps can read |

## Data

| Feature | Status | Notes |
|---|---|---|
| Export / import backups | done | Settings page: `GET /api/backup` (zip with `config.json` + `metadata.json`, derived caches left out), `POST /api/backup?mode=merge\|overwrite` (`mergeConfig` of `electron/data-export.js`); reads the old app's backups |
| App autostart (LaunchAgent) | not planned | until the Rust app replaces the Electron one |
