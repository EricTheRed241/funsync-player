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
| Status API with devices, issues (`/api/status` of the old backend) | not started | phase 5 (devices on the server) |
| Device dashboard (`/device`) | not started | |
| Access PIN, LAN mode | partial | `--lan` listens on every interface; the swipe deck's gate (`scout_report::gate`) with the old app's `access-pin.json`, so one login covers both apps (login page, cookie / `?k=`, lockout). Missing: headset body login and thumbnail-only keys (VR slice), WebSocket close 4401 (a refused socket gets 401) |
| One port + nav bar for the Rust sites (swipe deck, Downloads, creators, status) | done | `src/sites.rs`: `deploy/sites.json` relayed under their prefixes (base href + nav injected, `Location` mapped, bare page paths redirect), nav from the same file plus Parity |
| Log file | not started | stderr only |

## Video Player

| Feature | Status | Notes |
|---|---|---|
| Playback with range streaming | done | `GET /api/media/stream/{id}` (Range, 206), played by the web remote's player |
| Custom controls, keyboard shortcuts, drag-and-drop | partial | the web remote's controls (seek bar with chapters, double-tap ±10 s); no desktop keyboard shortcuts or drag-and-drop yet |
| Seek bar thumbnail preview | not started | |
| Subtitles (.srt / .vtt, SRT→VTT) | done | `GET /api/media/subtitle/{id}`, shown by the web remote |
| Screenshot, picture-in-picture, aspect ratio cycling | not started | |
| A-B loop | done | web remote player |
| Replay button at the end | not started | |
| Script gap auto-skip | done | web remote player |

## Play queue

| Feature | Status | Notes |
|---|---|---|
| One queue (user additions + auto-fill to 10) | not started | phase 2 |
| Queue on every client (page, headset section, `Queue:Up next` tag) | not started | phase 2 / 4 |
| Queue API (`/api/remote/queue*`) | not started | |

## Library

| Feature | Status | Notes |
|---|---|---|
| Library scan (pairing, variants, axis companions, subtitles, sidecars, fuzzy pass, AppleDouble skip, on-disk spelling) | done | port of `electron/library-scan.js`; on the real library (999 videos) every pairing, variant, subtitle, source and sidecar matches the JS scan |
| Generated thumbnails (10 % / ≥ 10 s, bad-frame retries, shared cache) | partial | rule ported, `thumb-cache/` with the old names, VR one-eye / centre crops (`_sbs`, `_sbsc`, `_tb`, `_tbc`) from the VR decision; no versioned URLs / immutable caching, no pinned frames, no cache pruning |
| Posters and custom thumbnails (EroScripts, FapTap, rule34video) | not started | |
| Grid / list / folder views, hover preview | partial | the web remote's grid, list and folder views; no hover preview |
| Sources: reorder, enable/disable, overlap, hot-plug | done | Settings page (`/remote/settings.html`, `src/settings.rs`): add, remove, enable, reorder, overlap warning; a source folder appearing or disappearing triggers a rescan (checked every 10 s) |
| Manual script association | not started | |
| Fuzzy search with exact-title precedence | done | the web remote's search (`library-search.js`) |
| Sort (preference score default, name, date, duration, speed, rating) | partial | the web remote's sorts; scores are the imported snapshot until the server computes them |
| Playlists | partial | listed and played (`/api/remote/playlists`); no edits yet |
| Categories from EroScripts tags, VR, axis categories | partial | the imported categories are listed (`/api/remote/categories`); not computed by the server yet |
| Artist playlists, aliases, playlist cleanup | not started | phase 3 |
| Auto collections, score lists | partial | the imported collections are listed; not computed by the server yet |
| Smart playlists (VR picks, Quick ones, Favorites) | not started | phase 3 |
| Preference score and tag scores | not started | phase 3 (Rust `candidateScore` exists in `crates/slr-scout/src/score.rs`) |
| Multi-select bulk assignment | not started | |
| Ratings and favourites | partial | imported and shown; no edits yet |
| Watched videos last, Watched collection | done | from the ratings (`watched`, `watchedLast`, the `:watched` collection) |
| Overview (capacity, ratings, artists, tag scores) | not started | |
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
| Triage from the phone (rating, heart, add to playlist) | not started | |
| Review on end | not started | |
| Tags / Artists views, tag scores, favourite artists | not started | |
| Player controls: own or browser | done | web remote |
| PWA install | done | the web remote's manifest and service worker |
| Local-only mode | done | loopback is the default; `--lan` opts in |

## VR content server

| Feature | Status | Notes |
|---|---|---|
| HereSphere API (sections, scene tags, scripts) | partial | `/heresphere` (`?filter=` vr, flat, scripted) and `/heresphere/<id>`: the old sections (Needs review, Queue, score lists, VR/Flat, speed, length, Hand-made, Recently Added, triage, playlists; watched last), scene media/scripts/subtitles/tags/chapters, URLs from the Host header. Read-only (`write*` false, no event server); Queue is `player.userQueue` only (no auto-fill); Hand-made from companion files only (no embedded axes); no access PIN; no `?v=` thumb versions |
| DeoVR API | partial | `/deovr` and `/deovr/<id>` with the same sections and tags; encoding name always `h264`; no access PIN |
| Rate / favourite / playlist / queue edits from the headset | not started | phase 4 |
| Review on end from the headset | not started | phase 4 |
| VR decision (projection, filename, frames, shape) | partial | VR Format projection, Mark as VR / Flat, filename rules (patterns, studio codes, `XXVR-123`, `VR180`, bare `VR`), ~2:1 ≥ 3200 wide fallback from probed dimensions (in memory, so a listing only sees videos probed since start); frame-based stereo detection (`stereo_detect.py`) not ported |
| PCVR companion bridge (HereSphere timestamp API) | not started | |
| Session status card, VR ↔ web remote mutex | not started | |

## Funscript support

| Feature | Status | Notes |
|---|---|---|
| Script loading (main, variant, axis) | partial | `GET /api/media/script/{id}?variant=&axis=` serves the files (hand-made axis beats generated); the server plays the main script and its axes on the devices; variants are not played yet |
| Heatmap and gap indicators on the seek bar | done | web remote |
| Script variants switch (V key) | not started | |
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
| TCode device profiles (MiraBot, FUNSR MINI, Custom) | not started | |
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
