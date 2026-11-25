# FunSync in Rust: parity with the Electron app

The Rust app (`crates/funsync-server`, http://127.0.0.1:5300/) is built beside the Electron app (port 5123) until it can replace it. One row per user-visible feature of `README.md`; the README section is the reference. Status: **done**, **partial** (what's there / what's missing), **not started**, **not planned** (why).

Ground rules while both run: the Rust app has its own data folder (`~/Library/Application Support/funsync-rust/`, seeded by `funsync-server import`, which only reads the old `config.json`) and treats the library as read-only (no deletes, renames, rating deletes or sidecar writes). Downloads, scouts and jobs stay with the Electron app.

The **status dashboard** at http://127.0.0.1:5300/status shows the counts, a progress bar and these tables (the server reads this file at build time), plus the server's own state.

## Server and data

| Feature | Status | Notes |
|---|---|---|
| Own data folder, `library.json` | done | `--data-dir`, `$FUNSYNC_RUST_DATA` |
| Import from the old app (read-only) | done | sources, playlists, categories, `videoCategories`, the old `settings.library` (ratings, associations, scores, tag scores …) kept verbatim; the EroScripts session is not copied. `video-tags.json` / `pornstar-tags.json` are read live in phase 3 |
| Health / status endpoint | done | `GET /health`, `GET /api/status` (sources present, videos, scan time, unreadable sources) |
| Status API with devices, issues (`/api/status` of the old backend) | not started | phase 5 (devices on the server) |
| Device dashboard (`/device`) | not started | |
| Access PIN, LAN mode | not started | loopback only until the PIN gate is ported (`--lan` then) |
| Nav bar | partial | `/site-nav.js` (the old nav script) links Remote and Status; the swipe deck, Downloads and Creators sites aren't proxied yet |
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
| Generated thumbnails (10 % / ≥ 10 s, bad-frame retries, shared cache) | partial | flat rule ported, `thumb-cache/` with the old names; VR one-eye crops wait for the VR decision; no versioned URLs / immutable caching, no pinned frames, no cache pruning |
| Posters and custom thumbnails (EroScripts, FapTap, rule34video) | not started | |
| Grid / list / folder views, hover preview | partial | the web remote's grid, list and folder views; no hover preview |
| Sources: reorder, enable/disable, overlap, hot-plug | not started | enabled flag honoured, overlap deduped; no UI |
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
| Local-only mode | done | loopback is the default (it's the only mode so far) |

## VR content server

| Feature | Status | Notes |
|---|---|---|
| HereSphere API (sections, scene tags, scripts) | not started | phase 4 |
| DeoVR API | not started | phase 4 |
| Rate / favourite / playlist / queue edits from the headset | not started | phase 4 |
| Review on end from the headset | not started | phase 4 |
| VR decision (projection, filename, frames, shape) | not started | phase 3 / 4 |
| PCVR companion bridge (HereSphere timestamp API) | not started | |
| Session status card, VR ↔ web remote mutex | not started | |

## Funscript support

| Feature | Status | Notes |
|---|---|---|
| Script loading (main, variant, axis) | partial | `GET /api/media/script/{id}?variant=&axis=` serves the files (hand-made axis beats generated); no playback yet |
| Heatmap and gap indicators on the seek bar | done | web remote |
| Script variants switch (V key) | not started | |
| Multi-axis (TCode 10 axes, embedded axes) | not started | phase 5 |
| Custom routing per device | not started | |
| Speed stats | partial | from the imported `speedStatsCache`; not computed for new scripts |

## Script smoothing

| Feature | Status | Notes |
|---|---|---|
| PCHIP / Makima interpolation | not started | phase 5 |
| Speed limit | not started | phase 5 |

## Devices

| Feature | Status | Notes |
|---|---|---|
| Buttplug / Intiface from the page | not started | phase 2 |
| TCode WebSocket (MiraBot, 60 Hz, 4 digits) from the page | not started | phase 2 |
| TCode USB serial from the server | not started | phase 5 |
| TCode UDP from the server | not started | phase 5 |
| Browser serial relay | not planned | the server drives USB serial itself (decision 3); revisit if a device sits on another computer |
| TCode device profiles (MiraBot, FUNSR MINI, Custom) | not started | |
| Auto-connect, WebSocket reconnect | not started | |
| Per-device offsets, Sync tab | not started | |
| Generated axes playback, hand-made axes win | not started | phase 5 |

## EroScripts, scouts, jobs

| Feature | Status | Notes |
|---|---|---|
| EroScripts search / download in the app, login | not started | |
| Post previews as thumbnails | not started | |
| Scouts, swipe deck, downloads, creators, job runner | not planned | stay with the current setup (Rust crates on their own ports); the Rust app links to them later |
| `eroscripts:sync`, dedupe, reorganize, `upgrade:axes` CLIs | not planned | CLIs, unchanged; they keep writing side files both apps can read |

## Data

| Feature | Status | Notes |
|---|---|---|
| Export / import backups | not started | |
| App autostart (LaunchAgent) | not planned | until the Rust app replaces the Electron one |
