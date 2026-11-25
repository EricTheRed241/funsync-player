# FunSync in Rust: parity with the Electron app

The Rust app (`crates/funsync-server`, http://127.0.0.1:5300/) is built beside the Electron app (port 5123) until it can replace it. One row per user-visible feature of `README.md`; the README section is the reference. Status: **done**, **partial** (what's there / what's missing), **not started**, **not planned** (why).

Ground rules while both run: the Rust app has its own data folder (`~/Library/Application Support/funsync-rust/`, seeded by `funsync-server import`, which only reads the old `config.json`) and treats the library as read-only (no deletes, renames, rating deletes or sidecar writes). Downloads, scouts and jobs stay with the Electron app.

The **parity dashboard** at http://127.0.0.1:5300/parity (nav bar: Parity) shows the counts, a progress bar and these tables (the server reads this file at build time), plus the server's own state.

## Server and data

| Feature | Status | Notes |
|---|---|---|
| Own data folder, `library.json` | done | `--data-dir`, `$FUNSYNC_RUST_DATA` |
| Import from the old app (read-only) | done | sources, playlists, categories, `videoCategories`, the old `settings.library` (ratings, associations, scores, tag scores …) kept verbatim; the EroScripts session is not copied. `eroscripts-links.json`, `video-tags.json` and `pornstar-tags.json` are read after each scan from the folder imported from (Side files) |
| Health / status endpoint | done | `GET /health`, `GET /api/status` (sources present, videos, scan time, unreadable sources) |
| Status API with devices, issues (`/api/status` of the old backend) | done | `/api/status` adds `issues` (TCode link waiting / reconnecting with attempts and last failure, no device connected), `devices` (TCode link: state, URL, since, attempts, retry, last error, commands and bytes; Buttplug devices), `session`, `phone`, `backend`, `desktop` (the server's engine, always connected); no VR bridge / headset parts |
| Device dashboard (`/device`) | done | `backend/device-dashboard/` served unchanged (this computer only; 403 otherwise), fed by `GET /api/device/telemetry` from the server's engine: 1 s samples for 15 minutes, the last ~100 s of commands, 500 events (link connects, drops, failures, scripts loaded, phone take-overs); the web remote's newest phone is the playback source |
| Access PIN, LAN mode | done | `--lan` listens on every interface; the swipe deck's gate (`scout_report::gate`) with the old app's `access-pin.json`, so one login covers both apps (login page, cookie / `?k=`, lockout). HereSphere / DeoVR log in from their body (any user, password = PIN; wrong PINs count toward the lockout; `access: 0 / -1` answers) and get media URLs with a day-rounded `?k=` and thumbnails with a thumbnail-only `t1.` key; a refused WebSocket is closed with 4401 |
| One port + nav bar for the Rust sites (swipe deck, Downloads, creators, status) | done | `src/sites.rs`: `deploy/sites.json` relayed under their prefixes (base href + nav injected, `Location` mapped, bare page paths redirect), nav from the same file plus Parity |
| Log file | done | `~/Library/Logs/funsync-rust/server.log` (`$FUNSYNC_RUST_LOG`), rotated to `server.old.log` at 1 GB; every request (client, method, path, status, time) except a successful loopback `/health`, plus the server's own lines |

## Video Player

| Feature | Status | Notes |
|---|---|---|
| Playback with range streaming | done | `GET /api/media/stream/{id}` (Range, 206), played by the web remote's player |
| Custom controls, keyboard shortcuts | done | the web remote's controls plus `web/remote/extras.js` (added to its page by the server): Space/K, ←/→, J/L, ↑/↓, M, F, S, P (picture-in-picture), I (info), A/B/Escape (loop), R, V/Shift+V, G/Shift+G, H (Devices page), Z (Edge hold) |
| Drag-and-drop / O (open a file outside the library) | not planned | a browser page gets a dropped file's bytes but not its path, so it can't find the scripts beside it or drive devices from them; add the folder as a library source instead |
| Seek bar thumbnail preview | done | `extras.js`: hovering the player's seek bar (mouse) shows the frame at that time from a hidden second `<video>` of the stream, with the time |
| Subtitles (.srt / .vtt, SRT→VTT) | done | `GET /api/media/subtitle/{id}`, shown by the web remote |
| Screenshot, picture-in-picture, aspect ratio cycling | done | `extras.js`: S saves a PNG of the frame, P picture-in-picture, R cycles contain / cover / fill |
| A-B loop | done | `extras.js`: A / B set the points, Escape clears (the web remote's phone player has no loop of its own) |
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
| Generated thumbnails (10 % / ≥ 10 s, bad-frame retries, shared cache) | partial | rule ported, `thumb-cache/` with the old names, VR one-eye / centre crops (`_sbs`, `_sbsc`, `_tb`, `_tbc`) from the VR decision; versioned URLs (`?v=`, immutable caching) in the web remote listing; the cache is pruned after a scan that read every source (`prune_thumb_cache`: other videos, older file versions, crops or markers, videos with a poster; an unreadable file keeps its thumbnails); no pinned frames; headset URLs not versioned yet |
| Posters and custom thumbnails (EroScripts, FapTap, rule34video) | partial | posters the old app saved (`customThumbnails`, files in its `custom-thumbs/`, read-only) are served, and EroScripts post pictures picked by hand on the EroScripts page (saved in the Rust data dir's `custom-thumbs/`). After each scan FapTap thumbnails (only when their pHash is within 12 bits of a sampled frame) and rule34video covers are imported as the old app does (`posters`: same slot rules, `faptapThumbnails` / `rule34videoThumbnails` attempts, 1 s apart, JPEG ≤ 640 wide). Real library after an import: 0 of 50 FapTap scenes / 179 covers to fetch (the old app tried them all). Automatic EroScripts post previews (forum topic candidates) are not ported; animated images are matched by their first frame |
| Grid / list / folder views, hover preview | done | the web remote's grid, list and folder views; `extras.js` plays a muted preview over a grid card after 0.4 s of hover (mouse), from 10 % in |
| Sources: reorder, enable/disable, overlap, hot-plug | done | Settings page (`/remote/settings.html`, `src/settings.rs`): add, remove, enable, reorder, overlap warning; a source folder appearing or disappearing triggers a rescan (checked every 10 s) |
| Manual script association | done | `library.associations` (`single` / `multi` with axes, the old app's shape, imported ones included) override the scan's pairing; Associate page (`/remote/associate.html`): videos without a script first, the main script and each axis picked from fuzzy-ranked choices (same folder first), or back to automatic pairing. After each complete scan (`src/assoc.rs`) entries (and ratings) of moved videos follow them on an unambiguous file-name match, and entries whose video and folder are provably gone from a drive that answers are dropped (never for an offline / disabled source); a missing single / multi / custom-main script is re-pointed at the file of the same name next to the video; on the real library nothing changed (1054 kept) |
| Fuzzy search with exact-title precedence | done | the web remote's search (`library-search.js`) |
| Sort (preference score default, name, date, duration, speed, rating) | done | the web remote's sorts, on the scores and speed stats the server computes |
| Playlists | done | listed and played; add / remove from the phone (`/api/remote/playlists/add-video`, `remove-video`); create, rename, delete, reorder and bulk add / remove on the Lists page (`/remote/manage.html`, `POST/DELETE /api/groupings/<kind>/<id>[/videos]`) |
| Categories from EroScripts tags, VR, axis categories | done | `prefs::lists::categories` after each complete scan, with `library.autoCategories` (made once, added once, `vr` / `vr-manual`); axis kind from companions + embedded axes (`GET /api/media/axis-flags`, `GET /api/media/vr-flags` as the old backend). On the real library every category, VR included (154 / 154 with the frame check), matches the Electron app |
| Artist playlists, aliases, playlist cleanup | done | ARTIST_PLAYLIST_MIN, ARTIST_ALIASES, Favourite artists (FAVOURITES + `library.favouriteArtists`), `library.artistPlaylists`; cleanup only after a scan that read every source. Real library: every artist playlist and Favourite artists identical |
| Auto collections, score lists | done | AUTO_COLLECTIONS (tags, title words, length / `len-*` tags, FapTap) with `library.autoCollections`; SCORE_BANDS rewritten from the scores. Real library: every auto collection and score list identical |
| Smart playlists (VR picks, Quick ones, Favorites) | done | rebuilt after each scan, new durations, frame checks and every library edit (`library.smartPlaylists`). Real library: all three identical, members and order |
| Preference score and tag scores | done | computed after each scan (`prefs::score`: RANKING, tag-score groups counted once, named groups from the title, favourite-artist bonus, known artists) and after tag-score and favourite-artist edits. Real library: 999 / 999 equal |
| Multi-select bulk assignment | done | Lists page: tick videos (search, "only in this list"), then add to / remove from any playlist, category (with colour) or collection |
| Ratings and favourites | done | `POST /api/remote/rating` (half stars, heart) saved to `library.ratings`; ≤ 2★ deletes only when the Rust app owns the library (Triage deletes) |
| Watched videos last, Watched collection | done | from the ratings (`watched`, `watchedLast`, the `:watched` collection) |
| Overview (capacity, ratings, artists, tag scores) | done | `/remote/overview.html` (nav: Overview): videos vs `libraryMaxVideos` with bar and GB per source, favourites / Keep / Unrated, star histogram, every rated or hearted video (rate it there, click to play); artists and tag scores open the web remote's Artists and Tags views |
| Triage deletes (≤ 2★), duplicate deletes | done | only while the Rust app owns the library (`FUNSYNC_OWNS_LIBRARY=1`; off by default: ratings just save and `/api/remote/delete-duplicate` answers 409): `src/delete.rs`, port of `electron/rejected-videos.js` (companions, emptied scene / creator folders, `eroscripts-scout-rejected.json`) |

## Side files

| Feature | Status | Notes |
|---|---|---|
| Video tags merge (`video-tags.json`) | done | with the scan sidecars and `eroscripts-links.json` into `library.scriptTags` (`addedTags`, VIDEO_TAGGER_TAGS), read-only; real library: all 1470 records identical |
| Pornstar tags merge (`pornstar-tags.json`) | done | applied performers' tags, merged with the video tags |
| Underage screener integration (review playlist, 1★) | done | `age-screen --backend http://127.0.0.1:<port>` runs against this server unchanged (`/api/remote/videos`, `/api/media/vr-flags`, `/api/remote/rating`, `/api/remote/playlists/add-video`): checked with `--dry-run --limit 2` and `--run --limit 2` (fake judge, temp data and userData copy), which filed a review and a favourite into "Underage review". A 1★ verdict deletes only while this server owns the library |

## Remote access / web UI

| Feature | Status | Notes |
|---|---|---|
| One web UI everywhere (phone first, desktop layout) | done | the web remote (`backend/web-remote/`, unchanged; phone layout and its sidebar desktop layout) at `/remote/`, plus this app's desktop pages in `crates/funsync-server/web/remote/` (Devices, Lists, Associate, Overview, EroScripts, Settings) and `extras.js` for the desktop player's keys and previews |
| Triage from the phone (rating, heart, add to playlist) | done | `src/edits.rs`, applied to the library at once (no desktop edit queue) |
| Review on end | done | the phone's flow (add to *Needs review* by name; a rating takes the video out) |
| Tags / Artists views, tag scores, favourite artists | done | the web remote's views; tag scores, favourite artists and the bonus are stored and every change (and Recalculate) reruns the library rules (`library_changed` → `prefs::refresh`), so scores, score lists and smart playlists follow at once |
| Player controls: own or browser | done | web remote |
| PWA install | done | the web remote's manifest and service worker |
| Local-only mode | done | loopback is the default; `--lan` opts in |

## VR content server

| Feature | Status | Notes |
|---|---|---|
| HereSphere API (sections, scene tags, scripts) | done | `/heresphere` (`?filter=` vr, flat, scripted) and `/heresphere/<id>`: the old sections (Needs review, Queue with auto-fill, score lists, VR/Flat, speed, length, Hand-made, Recently Added, triage, playlists; watched last), scene media/scripts/subtitles/tags/chapters, URLs from the Host header; writable rating / heart / tags and `eventServer`. Hand-made from hand-made companion files or axes embedded in the script; versioned thumbnail URLs (`?v=`); access PIN login from the body. Checked against the old backend on the real library: every section's count matches but Quick ones (+1, a video added after the old app's last refresh) and one speed bucket (the old registry holds a stale speed), and 11 sampled scene details (layout, tags, scripts, rating) are identical |
| DeoVR API | done | `/deovr` and `/deovr/<id>` with the same sections and tags; encoding name from the probed codec (`h265`, `h264`, else ffprobe's name; `video_info.py`), kept in `media-cache.json`; access PIN login from the body |
| Rate / favourite / playlist / queue edits from the headset | done | HereSphere scene POST: rating, `isFavorite`, `Playlist:<name>` and `Queue:Up next` tags (only tags changed since the scene was served count); DeoVR is read-only, as before |
| Review on end from the headset | done | `POST /heresphere/events/<id>`: a play takes the video out of the queue (once per video); a pause / close at ≥ 90 % of an unrated video files it under Needs review |
| VR decision (projection, filename, frames, shape) | done | VR Format projection, Mark as VR / Flat, filename rules (patterns, studio codes, `XXVR-123`, `VR180`, bare `VR`), then the frame check (`stereo`: four frames, half against half; `vr-stereo-cache.json`), then the ~2:1 ≥ 3200 wide fallback from probed dimensions (`media-cache.json`, so both survive restarts). Real library: the frame check agrees with the old backend on 906 / 906 videos it had looked at |
| PCVR companion bridge (HereSphere timestamp API) | done | the server's TCP client to the timestamp server (`settings.vr.lastHost` / `lastPort`, 23554; 4-byte length + JSON, keep-alive every second, dropped after 8 s without packets) drives the same engine with `settings.vr.offset`; the headset's video is found by this server's stream URL or by file name (normalized as the old app does); Connect / Disconnect on the Devices page (reconnects at start when left connected); `vrBridge` in `/api/status` and the dashboard (link state, packet age, jitter, video) |
| Session status card, VR ↔ web remote mutex | done | last wins: a phone opening a video disconnects the VR bridge, the bridge's first packet kicks the phone; current session and the last 50 (`settings.sessions.history`, the old app's shape) in `/api/status` `sessions` and on the Devices page (Session card with history), not a docked card |

## Funscript support

| Feature | Status | Notes |
|---|---|---|
| Script loading (main, variant, axis) | done | `GET /api/media/script/{id}?variant=&axis=` serves the files (hand-made axis beats generated); the server plays the main script or the chosen variant with their axes on the devices |
| Heatmap and gap indicators on the seek bar | done | web remote |
| Script variants switch (V key) | done | the web remote player's variant menu (`switch-variant` → `variant-changed`): the server plays the variant with its own axis files, the video's hand-made axes fill in and beat generated ones; V / Shift+V cycle it (`extras.js`) |
| Multi-axis (TCode 10 axes, embedded axes) | done | server engine (`src/devices/`): companion axes L1/L2/R0/R1/R2/V0/V1/V2/A0 and axes embedded in the main script (`additional_axes`, `raw`, `axes`, `channels`, sibling keys, inline TCode) |
| Custom routing per device | done | the video's association (`active: "custom"`, `custom.routes`) gives TCode and each Buttplug device its script (the main route plays the main script); Buttplug devices by index first when the name there still matches, else by name, and the new index is stored; unrouted Buttplug devices get nothing; a route's script that moved is found by name in the video's folder and its new path stored. Editor on the Devices page (pick a video, a script per device; `GET` / `POST /api/devices/routing`, saved like `buildAssociationEntry('custom', …)` with `single` / `multi` kept) |
| Orgasm Switch from the phone | done | `orgasm-hold` (hold / toggle per `player.orgasmSwitchMode`) loops `player.orgasmConfig` (single, multi main + axes, custom → single or main route) on every device, video paused or not; toggle's second press stops the devices until playback starts again; `orgasm-state` keeps the phone's button honest |
| Edge hold (Z) | done | Z in the web remote (`extras.js` → `POST /api/remote/edge`): the devices idle while held (`player.edgeHoldMode` `toggle`: each press flips it); ignored while the Orgasm Switch plays, which ends it |
| Speed stats | done | `speedStatsCache` filled for new scripts after each scan (the old backend's rule: held positions left out, rounded half to even) |

## Script smoothing

| Feature | Status | Notes |
|---|---|---|
| PCHIP / Makima interpolation | done | `settings.player.smoothing` (linear / pchip / makima), server engine |
| Speed limit | done | `settings.player.speedLimit` (units/s), every axis |

## Devices

| Feature | Status | Notes |
|---|---|---|
| Buttplug / Intiface from the page | done | decision change: **the server drives every device**, from the playback state the web remote streams over `/api/remote/sync`. Buttplug v3 client to Intiface (`ws://127.0.0.1:<buttplug.port>`) with the renderer's per-device output: strokers `LinearCmd`; vibrators, oscillating machines (Oscillate silences Vibrate on the same device) and constrict / inflate `ScalarCmd`; rotators `RotateCmd`; modes speed / position / intensity per output (`buttplug.deviceSettings["<index>:<name>"]`: `vibeMode`, `oscillateMode`, `scalarMode`, `rotateMode`), invert, range, floor / ceiling, max intensity (70 %) and 2 s ramp-up for oscillate / scalar devices; idle outputs at pause; a multi association's `buttplugVib` plays its vib script on the vibrators; devices kept in `settings.knownDevices`; Devices page section per device. Tested against a fake Intiface (Intiface Central wasn't running for a smoke). Not ported: per-device axis assignment outside custom routing, heat / spray controls, range extender |
| TCode WebSocket (MiraBot, 60 Hz, 4 digits) from the page | done | from the server (decision change above): 60 Hz `L09999I17` frames, per-axis enable / min / max / invert from `settings.tcode.axes`, 3 or 4 digits (`precision`); Devices page `/remote/devices.html` |
| TCode USB serial from the server | done | raw tty set up with `stty` (no serial crate), `settings.tcode.port` / `baudRate`; a missing port is looked for every 5 s (plug in later or replug) |
| TCode UDP from the server | done | `udpHost` / `udpPort` |
| Browser serial relay | not planned | the server drives USB serial itself (decision 3); revisit if a device sits on another computer |
| TCode device profiles (MiraBot, FUNSR MINI, Custom) | done | `POST /api/devices {kind:"tcode", profile}` and the Devices page: saves the current profile under `tcode.profiles.saved`, loads the next one's (or its defaults) and connects it unless it was disconnected; FUNSR MINI stroke length / speed / start position (`DAG` / `DKP` / `DSP`) sent at once and 2.5 s after each connect; the device pill names the profile. No floor / ceiling, generated motion or browser relay |
| Auto-connect, WebSocket reconnect | done | `autoConnect` set by Connect / Disconnect; at start the server reconnects what was connected; retries every 5 s for a minute, then every minute |
| Per-device offsets, Sync tab | done | `tcode.defaultOffset` / `buttplug.defaultOffset` from the Devices page and the player's device pill (`set-offset`); the Devices page's Sync section shows each device's total offset for the web remote and for the VR bridge (device + `vr.offset`) |
| Generated axes playback, hand-made axes win | done | generated companions play only when the script has no hand-made axis (companion or embedded) |

## EroScripts, scouts, jobs

| Feature | Status | Notes |
|---|---|---|
| EroScripts search / download in the app, login | done | `/remote/eroscripts.html` + `/api/eroscripts/*`: Discourse login with 2FA or a pasted `_t` cookie (session in the Rust data dir, 0600; checked at start, an expired one shows on the page and `/parity`), search with thumbnails and Paid badge, every script across the post and replies labelled by poster (forum uploads incl. CDN `original/` links), pixeldrain listed, other hosts named; saving a `.funscript` or the video's scripts from a `.zip` next to the video (axis suffix kept, replace or variant on conflict, rescan) only while `FUNSYNC_OWNS_LIBRARY=1`, else 409. Auto-match: the videos without a script are searched by name in the background (one search / 5 s, again after a week, `eroscripts-matches.json`) and listed first with their hit count in *Find scripts for…*, instead of a toast when one is opened. `eroscripts:sync` links / scout stay their CLIs |
| Post previews as thumbnails | partial | by hand: *Use the post picture* saves the topic's poster-size picture into the Rust data dir's `custom-thumbs/` and records `customThumbnails` (served as the video's poster). Not automatic after a download, no perceptual-hash match against the video's frames, no resizing, no library-wide fetch |
| Scouts, swipe deck, downloads, creators, job runner | done | the sites are relayed on 5300; `--supervise` builds and runs them and `job-runner serve` like the desktop app (a site whose port already answers is left alone). The scouts and jobs themselves stay their own crates |
| `eroscripts:sync`, dedupe, reorganize, `upgrade:axes` CLIs | not planned | CLIs, unchanged; they keep writing side files both apps can read |

## Data

| Feature | Status | Notes |
|---|---|---|
| Export / import backups | done | Settings page: `GET /api/backup` (zip with `config.json` + `metadata.json`, derived caches left out), `POST /api/backup?mode=merge\|overwrite` (`mergeConfig` of `electron/data-export.js`); reads the old app's backups |
| App autostart (LaunchAgent) | done | `funsync-server autostart install [serve flags…]` / `uninstall` / `status`: LaunchAgent `com.funsync.rust` runs `serve` with those flags at login and again whenever it stops (`KeepAlive`); stderr in `~/Library/Logs/funsync-rust/launchd.log`. Not installed while the Electron app is the daily app |
