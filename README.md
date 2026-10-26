# FunSync Player

A local desktop video player with device synchronization for funscript playback. Built with Electron. Windows and Linux.

**[Download Latest Release](https://github.com/DaveMakesWaves/funsync-player/releases/latest)**

---

## Features

### Video Player
- Full-featured player with custom controls, keyboard shortcuts, and drag-and-drop
- Seek bar thumbnail preview (live frame capture on hover)
- Subtitle support (.srt, .vtt) with automatic SRT to WebVTT conversion
- Screenshot capture, picture-in-picture, aspect ratio cycling
- A-B loop points for repeat sections
- Replay button when video ends
- Script gap auto-skip with configurable countdown or skip button

### Library
- Browse and organize your video collection from one or more source folders
- **Grid / list / folder-tree** view toggle with thumbnail cards and live hover preview
- Drag-to-reorder sources, per-source enable/disable, overlap detection, hot-plug drive detection
- Funscript auto-pairing by filename with auto/manual badges; subtitle auto-detection
- Fuzzy search with exact-title precedence, path + collection + category matching, diacritic folding
- Sort by name, duration, average speed, max speed (matched tab only)
- Playlists, categories, and collections with pin-folder-as-collection support
- Multi-select for bulk playlist/category assignment
- Script variant detection — `Title (Soft)`, `Title_hardcore` and `Title.intense` scripts next to `Title.mp4` become variants of that video
- Ratings and favourites — half-star rating and a favourite heart on every card, list row and in the player; sort by rating and filter to Favorites, Top rated (4+) or Unrated. Ratings follow a moved file the same way script associations do.

### Remote access
- **Local-only mode** — start with `--local-only` (e.g. `npx electron . --local-only`) or set `FUNSYNC_LOCAL_ONLY=1` to bind the backend to `127.0.0.1`. The desktop app works as usual, but the web remote, HereSphere and DeoVR can't reach it from any other device. Without either, it listens on every interface so the phone and headsets can connect.
- **Web Remote** — control FunSync from your phone on the same WiFi. Library, playback, device sync. Installable as a PWA from the phone's home screen.
  - Triage from the phone: the player shows half-star rating, a favourite heart and **Add to playlist…** (pick a playlist or name a new one); a playlist's view has a remove button per video. The library has a Rating filter (Favorites / Top rated 4+ / Unrated) and a Rating sort.
- **VR content server** — Quest standalone support via HereSphere (DeoVR works too). Your library appears in the headset; PC-connected devices sync automatically.
  - Library sections: VR, Flat, Slow / Medium / Fast (script average speed below 250, 250–349, 350 and up), Short (<5m) / Medium (5-15m) / Long (15m+), Hand-made (extra axes a scripter made: a companion file such as `Title.twist.funscript` that isn't a `.generated.` one, or non-empty axes embedded in the main script — `axes`, `channels`, `additional_axes`, `raw`, sibling axis keys or inline TCode values), and Recently Added (last 14 days). Then Favorites, Top Rated (4 stars and up), Unrated (every video without a rating — the triage queue; headset playback isn't tracked as "watched", so it can't be narrowed to watched videos) and one `Playlist: <name>` section per playlist, in your playlist order. Empty sections are hidden; videos whose speed or duration isn't known yet are left out of those sections, and embedded axes are picked up a moment after the library is registered.
  - Scene tags for filtering in the headset: `Format:VR` / `Format:Flat`, `Speed:Slow|Medium|Fast`, `Length:Short (<5m)|Medium (5-15m)|Long (15m+)`, `Feature:Hand-made multi-axis`, plus `Studio:<source folder>`, `Feature:Is scripted`, `Playlist:<name>` for each playlist the video is in, and 5-minute chapter markers.
  - **Rate, favourite and file into playlists from HereSphere.** Ratings (0–5 in half stars) and the favourite heart save back to the desktop library. Add a tag `Playlist:<name>` to put the video in that playlist (created if it doesn't exist); delete the tag to take it out. Other tag edits are ignored. DeoVR shows the same sections but is read-only.
  - Edits from the phone or the headset are applied by the desktop app, which owns the library. If it isn't running they wait in a queue (kept in `pending-library-edits.json` in the app's data folder, up to 1000 edits) and are applied in order when it next connects; the phone says so ("applies when the desktop app reconnects").
- **PCVR companion bridge** — HereSphere on PC (SteamVR, Virtual Desktop) driven via its timestamp API; all devices follow.
- **Session status card** docks bottom-right when an external source is driving playback, with a 50-entry history viewer and a VR ↔ Web Remote last-wins mutex.

### EroScripts Integration
- Search and download community funscripts from within the app
- Login with 2FA support (authenticator app), persistent session
- Thumbnails and metadata in search results; Paid Scripts posts are badged
- Finds scripts across the whole post: the author's replies (updated versions) and other users' versions, labelled by poster; funscripts on pixeldrain are listed and open in the browser, and posts hosted on MEGA, Google Drive, Gofile and similar say so instead of "no scripts"
- One-click download — scripts auto-renamed, auto-associated, and paired with the current video; axis files keep their axis suffix, and an existing script is never replaced without asking (Replace / Keep both as a variant / Cancel)
- Auto-match on video load — if no script is found locally, silently searches EroScripts and notifies
- **Post previews as thumbnails** — when a script is downloaded for a video (side panel or Associate → Find on EroScripts) and the video has no custom thumbnail, the post image that shows the video becomes its thumbnail everywhere: library, web remote, HereSphere and DeoVR. The image is downloaded once and stored locally (resized JPEG in the app's `custom-thumbs` folder), never hot-linked.
  - **Only images of the video.** Every image of the post's first post (and the topic thumbnail) is a candidate, except avatars and profile pictures, emoji, images in quotes, mentions and link previews, images linking to another site (Patreon, Ko-fi, store banners…), site and badge images, images under 320 px on the longest side and aspect ratios outside 1.2–3.2. Each candidate (several frames of an animated GIF/WebP) is compared with frames sampled across the video, skipping the first and last 5 % (every keyframe, or 24 seeks for files over 2.5 GB), by 64-bit perceptual hash (pHash); side-by-side VR frames are compared whole and by their left eye. A candidate is accepted when some frame is within **12 of 64 bits**; the closest accepted one wins and its distance is stored as `matchScore`, with its URL. No match: the generated frame stays. Samples are cached per video (path, size, modification time) in `preview-match-cache.json` in the data folder, two videos at a time.
  - Per video: kebab → Thumbnail → **Use EroScripts preview** (the post the script came from, or pick a post in a search) lists the post's images with their distance to the video, matching ones marked; the one picked becomes the thumbnail even if it doesn't match, and is then treated as the user's own. **Reset thumbnail** goes back to the generated frame. An uploaded poster or pinned frame is never replaced automatically.
  - Whole library: the header's **Fetch EroScripts previews** button first checks every EroScripts thumbnail FunSync set by itself against its video and removes those that don't show it (their image file is deleted; the generated frame comes back), then fills in a matching image for every video whose script came from a known post and that has no custom thumbnail, one post at a time, and reports what it set, removed and found no match for. Uploaded posters, pinned frames and previews picked by hand are never touched. Videos never linked to a post are left alone.
  - Dry run: `npm run eroscripts:preview-check -- [--pending] [--out <dir>]` reports, without writing anything in the data folder, which current EroScripts thumbnails the button would keep or remove (with distances), which post image would replace them, and contact sheets of both.
- **Reverse search: `npm run eroscripts:sync`** — links every library video that has a script to its EroScripts post, without the app running (log in once in the app; the command reuses that session). Run it every few days:
  - A link is **confirmed** when the post carries the local main script (same actions, metadata ignored) or one of the scene's hand-made axis files; **probable** when a post script has the same base name and the same length within 2 s. Anything weaker goes on a review list with its candidates and is never applied. Reposts of a confirmed script (e.g. a multi-axis re-upload) are kept as related posts.
  - Links go to `eroscripts-links.json` in the app's data folder; the command **never writes `config.json`**. The app merges the links on its next library scan, so **Fetch EroScripts previews** then covers those videos too.
  - Reports the hand-made axis files (twist / roll / pitch / surge / sway / vib …) the linked posts offer that a scene lacks, author's files first, replies with their poster. `--install-axes` downloads them as `<script stem>.<axis>.funscript` (never overwrites; only when the post's script is confirmed or its length matches the local one), then applies the folder rule below to each folder it installed into: sibling videos whose timing matches get a copy, and every `.generated.` file in the folder moves to the archive (`--archive <dir>`, default `<library parent>/Archive/_pruned-<date>`, with `MANIFEST.txt`).
  - Options: `--dry-run` (search and report only: no links file, nothing installed), `--limit N`, `--only <text>`, `--refresh` (re-check confirmed links too), `--retry` (re-check every video without a confirmed link now), `--stale-days N` (re-check unconfirmed results after N days, default 7), `--report <file>` (`.md` or `.json`; default `eroscripts-sync-report.md` in the data folder).
  - Read-only on the forum and paced at ≥ 700 ms between requests. Rate limited (429): stops with progress saved, exit code 2; the next run resumes. Expired session: exit code 3, log in again in the app.
  - Schedule (macOS): `npm run eroscripts:sync -- --install-schedule 3d` installs a launchd agent (`~/Library/LaunchAgents/com.funsync.eroscripts-sync.plist`) running every 3 days, logging to `logs/eroscripts-sync.log` in the data folder; `--uninstall-schedule` removes it.
- **Discovery scout: `npm run eroscripts:scout`** — finds free EroScripts posts worth adding to the library and downloads them for review, without the app running (it reuses the app's EroScripts login):
  - Lists the newest Free Scripts posts tagged multi-axis, animation, hentai, 3d, vam, sfm or mmd (`--pages N` for the multi-axis feed, default 6; `--anim-pages N` for each animation feed, default 3). Never takes loli, shota, furry / anthro / yiff or futa / futanari posts (tags, title or first post, as whole words: `futa-on-female` counts, `Futaba` doesn't); scenes already in the library are left alone. Takes animated posts only (no `real` tag; AI-generated video counts), plus music posts (PMV / HMV / Cock Hero) by a creator already in the library. Skips posts the library already links and scenes whose video it already has.
  - Each post's scripts are paired with its videos on the forum, MEGA or pixeldrain. Every script set gets its own video: names match without encode words (`1080p`, `8K60FPS`, `VR`…), numbers must agree (Part 1 isn't Part 2), and handy-only scripts are ignored. VR picks ≤ 8K, ≤ 60 fps and the low bitrate when offered; flat picks ≤ 4K. TCode suffixes (`.R0`, `.L1`…) are renamed to the names the library pairs.
  - Ranking: VR, then music, creator already in the library, creampie / riding, multi-axis, likes. The list and its plan go to `eroscripts-scout.json` in the data folder; the top `--top N` (default 40) print as a table with a Multi / Single axis column.
  - `--fetch` downloads the list, multi-axis posts first, into `<library parent>/_incoming/<artist>/<Artist - Title (metadata)>/` (files go to `_staging/` until complete), with an `eroscripts.json` sidecar per scene. **Single-axis scenes get generated twist / roll / pitch companions** (`npm run upgrade:axes`); hand-made axes are never touched. Every finished video joins the **Discovery** playlist, and `_incoming` becomes a library source named Discovery, so the scenes show in the library, the web remote, and HereSphere / DeoVR as `Playlist: Discovery`. `config.json` is written only while the app is closed (a backup is kept as `config.json.bak-discovery`); otherwise the videos wait in `eroscripts-scout-pending.json` until the next run. `--passes N --wait 1h` repeats the queue until nothing fails (MEGA and pixeldrain limit free downloads); finished scenes are skipped. `--ids 1,2` limits it to some posts, `--dry-run` lists the queue, `--incoming <dir>` changes the folder.
  - Scene names (the title the library, web remote and headset show) come from the post: **`Artist - Title (metadata)`**. The artist is the first leading `[Name]` / `(Name)` of the post title that names someone (labels such as `[VAM]`, `[Multi-axis]`, `[Fap Hero]` are skipped), else a trailing `by Name`, else the poster. Script labels (multi-axis, requested, suggested, commissioned, SR6, `… Script`) are dropped; other bracket parts and ` | ` segments become the metadata. A post with several videos names each by its own file (Iwara ids and `[Source]` dropped); a file name that says nothing (`7`) keeps the post title with the number as metadata. A VR projection suffix of the video name (`_180_3DH`, `_LRF_Full_SBS`) stays at the end so the headset still detects it.
  - `--retitle` renames scenes already in `_incoming` to that format, moving them to the artist's folder, and rewrites every path `config.json` keeps for them (playlists, script associations, caches; backup in `config.json.bak-discovery`). It needs the app closed (exit 2 otherwise); `--dry-run` lists the renames first. A second run changes nothing.
  - Exit codes: 0 done, 1 error, 2 rate limited, downloads still failing after the last pass, or the app running (`--retitle`), 3 not logged in / session expired.

### Funscript Support
- Heatmap overlay on the seek bar (speed-colored)
- Gap indicators on the seek bar showing idle sections
- Manual funscript association with fuzzy-ranked suggestions
- Script variations — switch between multiple script versions during playback (V key)
- Multi-axis funscript support (TCode convention, 10 axes); switching variants also switches to that variant's own axis files
- **Custom routing** — assign different scripts to different devices per video, stable across Intiface restarts (index-first matching with name auto-heal)
- Speed stats (average/max) displayed on library cards

### Script Smoothing
- PCHIP interpolation (shape-preserving, no overshoot) for smoother device motion
- Makima interpolation (less aggressive, good for oscillatory patterns)
- Configurable speed limit to prevent impossible device moves
- Per-setting persistence — configure once, applies to all playback

### Device Integration
- **The Handy** — HSSP cloud sync with automatic script upload, drift detection, 10-second cloud health check (catches BT-mode switches)
- **Buttplug.io** — Connect to 700+ devices via Intiface Central (strokers, vibrators, rotators, e-stim)
- **TCode (OSR2 / SR6 / MiraBot)** — USB serial, UDP (Wi-Fi) or WebSocket, with per-axis enable + min/max controls for all 10 axes (experimental). MiraBot's Wi-Fi firmware speaks UDP; its Bluetooth LE firmware is not supported
- **Autoblow Ultra / VacuGlide 2** — Cloud API (experimental)
- **Auto-connect on startup** — Handy and Buttplug.io connect automatically if previously configured
- Per-device offset controls + unified Sync tab showing total effective offset (VR + device stacking)
- Device simulator overlay showing real-time stroke position
- Works without any device connected — pure video playback is fully functional

### Script Editor (Experimental)
- OFS-style action graph with centered playhead and speed-colored lines
- Numpad placement (0-9 maps to position 0-100), frame-by-frame stepping, snap-to-frame toggle
- Selection, copy/paste, undo/redo (per-script history persistence)
- Multi-script selector for multi-axis / custom-routing editing
- Modifier tools: half/double speed, remap range, offset time, remove pauses, reverse
- Pattern generator: sine, sawtooth, square, triangle, escalating, random
- Gap detection and fill with configurable patterns
- Audio waveform display and beat detection for music-synced scripting
- Metadata editor (title, creator, tags, performers)
- Bookmarks with named markers on the graph
- Live device preview on numpad placement (when sync engine is idle)
- Autosave toggle (off by default, shows save timestamp when enabled)

### Multi-Axis Support (Experimental)
- **Target setup: 4 axes on a MiraBot Lite** — stroke (L0), twist (R0), roll (R1), pitch (R2). Every script in the library should drive all four.
- TCode convention detection (10 standard axes)
- Companion file detection by filename suffix, including generated `<name>.generated.<axis>.funscript` files
- Axes embedded in the main script are played too: `additional_axes`, `raw`, `axes` (TCode ids), `channels` (axis names or TCode ids), sibling axis keys and inline TCode values. An embedded axis needs at least 2 actions
- **Hand-made axis scripts always win, per folder.** A scene (a video and its stroke scripts, variants included) has hand-made axes when any of its scripts has a companion that isn't `.generated.` or axes embedded in it. A folder with any such scene keeps no `.generated.` axis files at all, including those of sibling videos (other versions of the scene in the same folder: `_4k` / `_6k`, `_hard`, VR / flat). A sibling gets a copy of the hand-made axes only when its stroke script's timing provably matches: identical actions, or both scripts end within 1 s of each other and ≥ 95 % of the source's actions have one in the sibling's script within ±50 ms. Otherwise it gets nothing and plays stroke only:
  - playback: once a script has any hand-made axes (companion or embedded), its generated companions are not played, whatever order the files are listed in, including ones saved in an older multi-axis association
  - `npm run upgrade:axes -- <folder>` fills the gap for stroke-only scripts: it writes twist, roll and pitch companions (never overwrites; `--dry-run` to preview). Surge / sway / valve only with `--surge` / `--sway` / `--valve`. It skips every script in a folder with hand-made axes and says so
  - `npm run prune:generated -- [--dry-run] --archive <dir> <library folder>...` cleans up a library: in each folder with hand-made axes it writes embedded axes out as `<stem>.<axis>.funscript` companions (never replacing a file; the main script is left alone), copies a hand-made axis set to each timing-compatible sibling as `<sibling stem>.<axis>.funscript` (never replacing a file; a sibling that doesn't match is reported as not timing-compatible) and moves every `.generated.` axis file in the folder into `<dir>/<path relative to the library folder's parent>`, listing each move as `original<TAB>archived` in `<dir>/MANIFEST.txt`. Nothing is deleted; `--dry-run` only prints the plan. macOS `._` files are ignored
  - after an EroScripts download (panel or the Associate dialog) saves a hand-made axis file, or a main script with embedded axes, every `.generated.` axis file in that script's folder (sibling videos' included, other folders untouched) is moved to the system trash and the videos are re-paired. Nothing is copied to siblings there; the toast says how many trashed files were siblings'. Run `prune:generated` to copy to matching siblings
- Switching a script variant switches to that variant's own axis files; a generated variant file never replaces the video's hand-made one
- Axis-to-device feature mapping
- Searchable dropdowns for axis assignment with fuzzy matching

### Data
- Settings, playlists, and categories stored locally (no cloud, no account)
- Export/import backups as .funsync-backup zip files
- Automatic updates — notified when a new version is available; **you control download AND install** (no silent install on app quit)
- All user data preserved across updates

---

## Install

1. Download the latest installer from the [Releases page](https://github.com/DaveMakesWaves/funsync-player/releases/latest)
   - **Windows**: `FunSync Player Setup X.Y.Z.exe`
   - **Linux**: `FunSync-Player-X.Y.Z.AppImage`
2. Run the installer / AppImage
   - Windows: no admin required, installs per-user
   - Linux: `chmod +x` the AppImage, then run
3. The app launches automatically

Updates are checked on startup. When a new version is available, a toast appears — click Download, then Restart Now when ready.

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Space / K | Play / Pause / Replay |
| Left / Right | Seek 5s |
| J / L | Seek 10s |
| Up / Down | Volume |
| M | Mute |
| F / F11 | Fullscreen |
| O | Open file |
| H | Device panel |
| S | Screenshot |
| I | Info overlay |
| A / B | Loop points |
| Escape | Clear loop / close panels |
| R | Cycle aspect ratio |
| E | Toggle script editor |
| D | Toggle device simulator |
| V / Shift+V | Cycle script variants |
| G / Shift+G | Skip to next/previous action |

### Script Editor (when canvas focused)

| Key / Mouse | Action |
|-------------|--------|
| 0-9 / Numpad 0-9 | Place action at current playhead (positions 0, 11, 22, ..., 100) |
| Alt+Click on empty canvas | Insert action at click position |
| Left / Right | Step one video frame |
| Ctrl+Left / Right | Fast frame step (default 6 — `editor.fastStepFrames`) |
| Shift+Left / Right | Move selected action(s) ±1 frame in time |
| Ctrl+Shift+Left / Right | Move selected action(s) ±N frames in time |
| Up / Down | Select previous / next action (with seek) |
| Ctrl+Up / Down | Select previous / next action across all loaded scripts |
| Shift+Up / Down | Nudge selected position ±5 (coarse) |
| Ctrl+Shift+Up / Down | Nudge selected position ±1 (fine) |
| Delete / Backspace | Delete selected actions |
| Ctrl+Z / Ctrl+Y | Undo / Redo |
| Ctrl+C / V / X | Copy / Paste / Cut |
| Ctrl+A | Select all |
| Ctrl+I | Invert positions |
| Ctrl+S | Save |
| W | Toggle audio waveform overlay |
| B | Add bookmark at current time |
| +/- | Zoom in / out |
| Escape | Clear selection / close editor |

When placing actions: press numpad without moving the playhead to refine the just-placed action's height; seek the playhead before pressing numpad to place a new action at the new time.

---

## Requirements

- Windows 10/11 (x64) or Linux (AppImage — Ubuntu 22+, Fedora 38+, SteamOS)
- [Intiface Central](https://intiface.com/central/) for Buttplug.io device support (optional)

---

## Building from Source

```bash
npm install
cd backend && python -m venv .venv && .venv/Scripts/pip install -r requirements.txt && cd ..
npm run build
```

Requires Node.js 18+, Python 3.11+, and ffmpeg/ffprobe binaries in the `ffmpeg/` directory (or `ffmpeg-linux/` for Linux).

---

## License

FunSync Player is free software, licensed under the **GNU General Public License v3.0 or later**. The full text is in [LICENSE](LICENSE).

You are free to use, study, share and modify it. If you distribute a modified version, it must also be released under the GPL with its source available, so improvements stay available to everyone.

```
Copyright (C) 2026 DaveMakesWaves

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

You should have received a copy of the GNU General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
```

### Packaging

Distribution packages are welcome. The GPL permits redistribution of both source and binaries, so no separate permission is needed to package FunSync for a distro repository.
