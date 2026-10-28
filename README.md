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
- **Categories from EroScripts tags.** After each library scan, videos whose script came from an EroScripts post (downloaded in the app, linked by `eroscripts:sync`, or fetched into Discovery by the scout, whose `eroscripts.json` the scan reads) or from FapTap (`faptap:scout`, its `faptap.json`) are put in Creampie, Riding, Blowjob, Handjob, Titjob, Footjob, Anal, Doggy, Missionary, Group, Monster, PMV / HMV and POV by the post's tags (`anal-creampie` counts as Creampie, `reverse-cowgirl` as Riding …). An existing category with the same name is reused. Each category is created once and each video put in it once, kept in `library.autoCategories`: delete a category or take a video out and it stays that way.
  - **VR comes from the video, not the post.** The VR category holds every video the backend decides is VR, tagged or not: the VR Format panel's projection first (Flat = not VR), then Mark as VR / Mark as Flat, then filename markers (`_sbs`, `VR180`, `mkx200` …), then the frame shape (about 2:1 and at least 3200 px wide, probed in the background; videos still being probed are added on a later check, every 30 s for up to 10 minutes after a scan). Flat videos that the old post-tag rule had put in VR (posts tagged "2D/VR") are taken out; videos you put there yourself are never taken out, and a VR video you take out stays out. The library's VR / Flat filter, the web remote and HereSphere / DeoVR use the same decision.
  - **Hand-made axes / Generated axes / Single-axis.** Every scripted video is in one of these three categories, by its axes as the backend sees them (`GET /api/media/axis-flags`, the same rule as the headset's Hand-made section): **Hand-made axes** when a scripter made at least one extra axis (a `<name>.<axis>.funscript` companion, or axes embedded in the script; a partial set counts), else **Generated axes** when it has only `<name>.generated.<axis>.funscript` companions, else **Single-axis**. A video moves when its axes change (axes generated for it, or hand-made ones added); taking it out by hand sticks. They show as `Category:Hand-made axes` (…) on the headset and in the web remote's category filter, so hand-made scenes are one tap away. Videos whose scripts the backend is still reading for embedded axes are sorted when it's done (asked every 30 s for up to 10 minutes after a scan).
- **Artist playlists.** After each library scan, every artist with five or more videos gets a playlist named after them, and their new videos join it. The artist is the first folder under a library source (`<source>/<artist>/<scene>/<video>`, the layout the Discovery scout uses too), counted across all sources and spellings; folders like `Unknown`, `Cock Hero` or `Fap Hero` don't count. A **favourite** artist (`FAVOURITES` in the content preferences, the scouts' list too; matched to the folder name whatever its case) gets a playlist from their first video, and all favourites' videos also go into one **Favourite artists** playlist, grouped by artist. An existing playlist of that name is reused. Each playlist is made once and each video added once, kept in `library.artistPlaylists` (the combined playlist under the key `:favourites`): delete the playlist or take a video out and it stays that way. They show in HereSphere as `Playlist: <artist>`.
  - **Playlist cleanup.** After each library scan, right after the artist playlists, every playlist with no video left in the library goes (yours too; Discovery and Review are made again when a video needs them), and so does an artist playlist the app made that holds fewer than five videos still in the library (a favourite's only when empty). Such an artist is forgotten in `library.artistPlaylists`, so its playlist comes back, with all its videos, once it reaches five again. Your own playlists with videos stay whatever their size. Paths of deleted videos (left in a playlist while the app was open) don't count. It never runs on a scan that couldn't read every library source, so a disconnected drive can't empty your playlists.
- **Auto collections.** After each library scan five themes are kept up to date from the EroScripts post tags, the video durations and the video titles (file names): medium (Hentai, SFM, VaM, MMD, CGI / 3D, AI video), session length (Quick under 5 min, Sessions 15 min+, Marathons 1 hr+; a known duration wins over the post's `len-*` tag), franchise (Overwatch, Genshin Impact, Zenless Zone Zero, Final Fantasy, NieR (2B), Honkai: Star Rail, League of Legends, Street Fighter, Marvel Rivals; by tag, or by the franchise or a character named in the title as a whole word, e.g. `Tifa`, `D.Va`, `Ellen Joe`, `2B`, `Ahri`), script style (Music / beat, Action, Cock Hero / Fap Hero by tag or title) and source (**FapTap**: every scene `faptap:scout` brought in). A collection is made only once something matches, and once only; an existing one with the same name is reused; each video is added once (`library.autoCollections`), so deleting a collection or taking a video out sticks.
- **Preference score.** Every scene gets the score the EroScripts and FapTap scouts rank posts by, with the same weights (`RANKING` in `renderer/js/content-preferences.js`, applied by one shared implementation, `renderer/js/preference-score.js`), so the library and the scouting agree: hand-made axes +4 (the backend's axis kind, as in the Hand-made axes category; generated or single-axis add nothing), VR +3 (the backend's VR decision), music +2 and creampie / riding +2 (+0.5 for both) from the post tags, +2 when the artist folder is a favourite or holds five or more videos (the artist-playlist rule), likes up to +3 when the post record holds them; 2D hentai only −3, a monster −0.5 and feet −0.5 from the tags and the title (the file name). Scenes without post tags are scored from VR, axes and the title. The scores are recomputed after each scan and whenever the backend's VR or axis decisions arrive, and kept in `library.preferenceScores` (by video path) so they show right after a start; a scripted video whose axes the backend is still reading keeps its previous score until then. Each card and list row shows the score as a small number, and the Sort menu has **Preference score** (highest first; pick it again for lowest first). The web remote shows it on each scene (◆ 7.5) with the same sort, and HereSphere and DeoVR get a read-only `Score:<n>` scene tag, rounded to the nearest 0.5 and written with one decimal (`Score:7.5`, `Score:7.0`, `Score:-3.0`). Star ratings are separate and never changed by it.
- Multi-select for bulk playlist/category assignment
- Script variant detection — `Title (Soft)`, `Title_hardcore` and `Title.intense` scripts next to `Title.mp4` become variants of that video
- Ratings and favourites — half-star rating and a favourite heart on every card, list row and in the player; sort by rating and filter to Favorites, Top rated (4+) or Unrated. Ratings follow a moved file the same way script associations do.

### Remote access
- **Local-only mode** — start with `--local-only` (e.g. `npx electron . --local-only`) or set `FUNSYNC_LOCAL_ONLY=1` to bind the backend to `127.0.0.1`. The desktop app works as usual, but the web remote, HereSphere and DeoVR can't reach it from any other device. Without either, it listens on every interface so the phone and headsets can connect.
- **Web Remote** — control FunSync from your phone on the same WiFi. Library, playback, device sync. Installable as a PWA from the phone's home screen.
  - Triage from the phone: the player shows half-star rating, a favourite heart and **Add to playlist…** (pick a playlist or name a new one); a playlist's view has a remove button per video. The library has a Rating filter (Favorites / Top rated 4+ / Unrated), a Rating sort and a Preference score sort; each scene shows its preference score (◆ 7.5, see Library).
  - **Tags and categories as chips.** The player shows the video's categories and tags as chips: tap a category to open it, a tag to show the library filtered to it (the filter shows as a removable `Tag: …` chip, next to the other active filters). Cards and list rows name them on one line. Tags are the EroScripts / FapTap post tags linked on the desktop, lower case, without the noise: length (`len-…`), device tags (`the-handy`, `osr2`, `sr6`, `multi-axis`, `twist` …), `vr` / `non-vr`, `no-free-links` and `straight`.
  - **Tags view** (bottom nav, next to Categories): every tag with its video count and running time, most used first; tap one for the library filtered to it.
  - The VR filter and the VR badge use the backend's VR decision (VR Format panel projection, then Mark as VR / Flat, then the filename, then the frame shape), the same one HereSphere and DeoVR get. The desktop and other tools can read it per video at `GET /api/media/vr-flags` (`{"flags": {path: true|false}, "pending": n}`; videos still waiting for their frame size are left out and counted in `pending`).
  - Tapping a category opens that category (it used to fall back to the library).
- **VR content server** — Quest standalone support via HereSphere (DeoVR works too). Your library appears in the headset; PC-connected devices sync automatically.
  - Library sections: VR, Flat, Slow / Medium / Fast (script average speed below 250, 250–349, 350 and up), Short (<5m) / Medium (5-15m) / Long (15m+), Hand-made (extra axes a scripter made: a companion file such as `Title.twist.funscript` that isn't a `.generated.` one, or non-empty axes embedded in the main script — `axes`, `channels`, `additional_axes`, `raw`, sibling axis keys or inline TCode values), and Recently Added (last 14 days). Then Favorites, Top Rated (4 stars and up), Unrated (every video without a rating — the triage queue; headset playback isn't tracked as "watched", so it can't be narrowed to watched videos) and one `Playlist: <name>` section per playlist, in your playlist order. Empty sections are hidden; videos whose speed or duration isn't known yet are left out of those sections, and embedded axes are picked up a moment after the library is registered.
  - Scene tags for filtering in the headset: `Format:VR` / `Format:Flat` (the backend's VR decision, as above), `Speed:Slow|Medium|Fast`, `Length:Short (<5m)|Medium (5-15m)|Long (15m+)`, `Feature:Hand-made multi-axis`, plus `Studio:<source folder>`, `Feature:Is scripted`, `Playlist:<name>` for each playlist the video is in, `Category:<name>` for each category it's in (the EroScripts ones included), `Tag:<tag>` for each of its tags (the web remote's chip tags), `Score:<n>` for its preference score (nearest 0.5, one decimal; see Library), and 5-minute chapter markers. `Category:`, `Tag:` and `Score:` are read-only in the headset; desktop changes show up within a second. DeoVR's scene categories carry the same tags.
  - **Rate, favourite and file into playlists from HereSphere.** Ratings (0–5 in half stars) and the favourite heart save back to the desktop library. Add a tag `Playlist:<name>` to put the video in that playlist (created if it doesn't exist); delete the tag to take it out. Other tag edits are ignored. DeoVR shows the same sections but is read-only.
  - Edits from the phone or the headset are applied by the desktop app, which owns the library. If it isn't running they wait in a queue (kept in `pending-library-edits.json` in the app's data folder, up to 1000 edits) and are applied in order when it next connects; the phone says so ("applies when the desktop app reconnects").
  - **Triage deletes from disk.** A video **rated 2 stars or lower** (desktop, web remote or HereSphere; a favourite never) is deleted permanently, with its scripts, axis companions (hand-made and generated) and subtitles; a scene folder left with only its `eroscripts.json` goes too, and so does an emptied creator folder. It leaves every playlist, loses its rating, and the library rescans. Only files inside a library source are touched. The desktop app does this as soon as it applies the edit, and at startup for anything already there. Discovery scenes are remembered in `eroscripts-scout-rejected.json` in the data folder, so the scout never downloads that post again (entries from the former `No thank you` playlist, reason `no-thank-you`, still count).
  - **2½–3½ stars go to Review.** A video rated between 2 and 4 stars joins the `Review` playlist (made when needed); rating it 4 or more takes it out. At startup Review picks up such ratings it hasn't seen before, so taking a video out by hand sticks until you rate it again.
  - **A restarted backend gets the library back.** The backend holds the library only in memory. When it restarts (the *Restart backend* button, the backend being killed and started again, or one started outside the app), the desktop app sends its last scan again as soon as it reconnects, within about 15 seconds, so the phone and the headset don't come up empty until the next rescan.
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
  - FapTap Discovery scenes get FapTap's own thumbnail the same way, automatically after each library scan (only when it matches the video and no custom thumbnail is set; one that doesn't match isn't fetched again).
  - Dry run: `npm run eroscripts:preview-check -- [--pending] [--out <dir>]` reports, without writing anything in the data folder, which current EroScripts thumbnails the button would keep or remove (with distances), which post image would replace them, and contact sheets of both.
- **Reverse search: `npm run eroscripts:sync`** — links every library video that has a script to its EroScripts post, without the app running (log in once in the app; the command reuses that session). Run it every few days:
  - A link is **confirmed** when the post carries the local main script (same actions, metadata ignored) or one of the scene's hand-made axis files; **probable** when a post script has the same base name and the same length within 2 s. Anything weaker goes on a review list with its candidates and is never applied. Reposts of a confirmed script (e.g. a multi-axis re-upload) are kept as related posts.
  - Links go to `eroscripts-links.json` in the app's data folder; the command **never writes `config.json`**. The app merges the links on its next library scan, so **Fetch EroScripts previews** then covers those videos too.
  - Reports the hand-made axis files (twist / roll / pitch / surge / sway / vib …) the linked posts offer that a scene lacks, author's files first, replies with their poster. `--install-axes` downloads them as `<script stem>.<axis>.funscript` (never overwrites; only when the post's script is confirmed or its length matches the local one), then applies the folder rule below to each folder it installed into: sibling videos whose timing matches get a copy, and every `.generated.` file in the folder moves to the archive (`--archive <dir>`, default `<library parent>/Archive/_pruned-<date>`, with `MANIFEST.txt`).
  - Options: `--dry-run` (search and report only: no links file, nothing installed), `--limit N`, `--only <text>`, `--refresh` (re-check confirmed links too), `--retry` (re-check every video without a confirmed link now), `--stale-days N` (re-check unconfirmed results after N days, default 7), `--report <file>` (`.md` or `.json`; default `eroscripts-sync-report.md` in the data folder).
  - Read-only on the forum and paced at ≥ 700 ms between requests. Rate limited (429): stops with progress saved, exit code 2; the next run resumes. Expired session: exit code 3, log in again in the app.
  - Schedule (macOS): `npm run eroscripts:sync -- --install-schedule 3d` installs a launchd agent (`~/Library/LaunchAgents/com.funsync.eroscripts-sync.plist`) running every 3 days, logging to `logs/eroscripts-sync.log` in the data folder; `--uninstall-schedule` removes it.
- **Discovery scout: `npm run eroscripts:scout`** — finds free EroScripts posts worth adding to the library and downloads them for review, without the app running (it reuses the app's EroScripts login):
  - Lists the newest Free Scripts posts tagged multi-axis, animation, 3d, vam or sfm (`--pages N` for the multi-axis feed, default 6; `--anim-pages N` for each animation feed, default 3). There's no hentai feed: hentai posts still come in through the others, and 2D hentai ranks lower (below); there's no mmd feed either, since MMD is blocked (below). Never takes loli, shota, furry / anthro / yiff or futa / futanari posts (tags, title or first post, as whole words: `futa-on-female` counts, `Futaba` doesn't); scenes already in the library are left alone. Takes animated posts (no `real` tag; AI-generated video counts), music posts (PMV / HMV / Cock Hero) by a creator already in the library, and live action as below. Skips posts the library already links, posts turned down in the app (2 stars or lower), and scenes whose video it already has.
  - **Live action (both scouts).** Animated is the default. Live action by a **favourite** creator or actress (named in the title, tags, poster / uploader or FapTap performers) is always taken; the list is `FAVOURITES` in `renderer/js/content-preferences.js` (ScyllaHMV, Agapilord, Eternal Enigma, Cuddle Mocap, Noodledude, NoFaceGirl, Candy Love, JJULEZ; it replaces the old `scout-favourites.json`, which nothing reads any more). Other live action is listed as "live (quota)" and downloaded only **one in every 20 Discovery downloads**, counted across both scouts in `scout-live-quota.json`, so new live creators still turn up; quota scenes go last in a `--fetch` so the run's other downloads can earn the slot, and the rest wait for later runs. Favourites rank like creators already in the library.
  - **Monsters and feet rank lower (both scouts, favourites and picks included); they aren't kept out.** A post loses 0.5 (`RANKING.monster`) when a tag names a monster (monster, monstergirls, tentacle, werewolf, horse / horse-cock, bestiality, zoophilia, orc, goblin, minotaur, ogre, troll, alien, beast, slime-girl, knot …) or its title does, as whole words in any case (monster girl, tentacles, 触手, werewolf, horse guy, horse cock, orcs, goblins, minotaur, ogre, troll, aliens, beast, bestiality, and `WW` on its own, as in Solace2K's and Rekin3D's "Mercy X WW"). Succubi, demon girls and elves aren't monsters. Feet lose 0.5 too (`RANKING.feet`): footjob, feet, foot, foot-fetish, foot-worship, footsie, soles or toes as a tag, or feet / foot / footjob / toes / soles / footsie / 足コキ in the title; a post with both loses 1.0. On EroScripts the same words in the first post count too (saved on the candidate as `monster` / `feet`). Posts an older `--recheck` marked `contentBreak: monster / feet` are back on the list, ranked lower.
  - **Blocked creators and tags (both scouts, favourites and picks included).** `BLOCKED_CREATORS` in `renderer/js/content-preferences.js` (now AidenHet) are never taken: a post or video whose title names one, posted / uploaded by one, or with one as artist or FapTap performer, is turned down as `blocked`. Names compare without case, spaces or punctuation (`[Aiden Het]`, `aidenhet`). `BLOCKED_TAGS` / `BLOCKED_TITLE` (now **MMD**: the `mmd` tag, or MMD as a word in the title) are turned down the same way. `--fetch` drops waiting candidates of either saved list that match. Library scenes aren't touched; delete them with 2 stars or lower.
  - **Sex required (both scouts, picks included).** A post needs penetrative sex, by tag (vaginal, anal, sex, riding, cowgirl, reverse-cowgirl, doggy, missionary, mating-press, pronebone, spooning, creampie, anal-creampie, double-penetration, threesome, foursome, gangbang, orgy, cum-in-pussy …) or by a word in its title (sex, fuck / fucked / fucking, riding, rides, cowgirl, doggy, missionary, creampie, anal, mating press, 中出し, セックス, 騎乗位, 做爱, 骑乘, 내사 …), or on EroScripts by such a word in the first post. Each EroScripts candidate keeps where the sex showed (`sexual`: `tags`, `title` or `text`), so a saved one is judged again without reading the post. A blowjob, handjob, titjob or paizuri alone doesn't count; with sex it's fine. Music videos (PMV, HMV, Cock Hero, Fap Hero, music-video) count unless they're a dance (`dance` tag, or "dance" / ダンス / 舞蹈 in the title) or their title is about a non-sex act without a sex word (`Blowjob PMV`, `Titjob HMV`, `JOI`). Posts tagged `sfw` or `non-porn` never qualify. `--fetch` and every scan drop waiting candidates of either saved list (picks too) that now break these rules, and `--fetch` prints how many.
  - **`--recheck` (one-off):** candidates saved before the evidence was kept that fail only the sex rule (and those an older `--recheck` turned down for monster / feet words) get their first post read (paced, read-only): sex there keeps them (`sexual: 'text'`, plus `monster` / `feet` when the post names one), excluded words there drop them for good (`contentBreak: 'excluded'`), the rest stay skipped. It prints the counts with examples and writes the list atomically; exit 2 when rate limited (progress kept), 3 when the session expired.
  - **Hand-made axes only.** A scene is taken only with at least one hand-made axis file (a partial set counts and is left as made); single-axis scenes come in only as picks (`--add`). A post left without scenes is turned down; `--fetch` applies the same rule to a saved list. FapTap, whose scripts are all single-axis, is paused while the library works towards about half hand-made scenes.
  - **Library cap.** A `--fetch` (either scout) downloads nothing more once the library holds `LIBRARY_MAX_VIDEOS` videos (500), counted over the enabled library sources when each pass starts; a pass that reaches it stops there (exit 2). Delete scenes (2 stars or lower) or raise the cap to go on.
  - Each post's scripts are paired with its videos on the forum, MEGA or pixeldrain. Every script set gets its own video: names match without encode words (`1080p`, `8K60FPS`, `VR`…), numbers must agree (Part 1 isn't Part 2), and handy-only scripts are ignored. VR picks ≤ 8K, ≤ 60 fps and the low bitrate when offered; flat picks ≤ 4K. TCode suffixes (`.R0`, `.L1`…) are renamed to the names the library pairs.
  - Ranking: multi-axis +4 (EroScripts only; FapTap has no multi-axis scripts), VR +3, music +2, creator already in the library +2, creampie / riding +2 (+0.5 for both), likes up to +3; **2D hentai ranks 3 lower** (animation tags only hentai / 2d / anime, no 3d / sfm / vam / mmd / cgi / blender / koikatsu tag and no 3D / SFM / VaM / MMD in the title; FapTap the same), a monster −0.5, feet −0.5. A saved list (either scout) is scored again under the current weights on every scan and `--fetch`. The app scores library scenes with the same code (`renderer/js/preference-score.js`; see Preference score under Library). The list and its plan go to `eroscripts-scout.json` in the data folder; the top `--top N` (default 40) print as a table with a Multi / Single axis column and a New mark for posts found by this scan.
  - **Only new posts each run.** Every judged post is remembered with its verdict in `eroscripts-scout-state.json` in the data folder, so a scan reads each feed from the top only until a page holds nothing new (`--pages` / `--anim-pages` cap how deep), and judges just the new posts. Each run also **backfills** `--backfill N` older pages per feed (default 2) from where the last run stopped, until the feed's end, so the archive gets covered over time; the summary shows how far each feed has got. Candidates found earlier stay in the list until all their scenes are downloaded or turned down. When the rules change the stored verdicts count as unjudged again; `--full` judges every listed post again anyway.
  - Posts listed but not judged yet are kept in the state too, so a scan stopped by a rate limit or an expired session goes on with them next run. A post turned down for **no video** or **single-axis only** is judged again once it has new activity (scripters often add the video or axes later); other verdicts stand.
  - **Backfill to a date:** `--feeds multi-axis --since 2025-01-01 --backfill 80` walks just the multi-axis feed back to posts last active on 1 January 2025 and judges them (`--feeds` takes any of `multi-axis, animation, 3d, vam, sfm`). A feed that reaches its date shows `done to <date>` and isn't backfilled further, by later scans either, until a run passes an earlier `--since`.
  - `--fetch` downloads the list, multi-axis posts first, into the library's **EroScripts folder**, `<library parent>/EroScripts/<artist>/<Artist - Title (metadata)>/` (files go to `_staging/` until complete; a failed or interrupted download deletes its partial file there, since the next attempt starts over), with an `eroscripts.json` sidecar per scene. Each scene's scripts come first: **a video whose main script ends before two minutes isn't downloaded** (it's recorded in `eroscripts-scout-rejected.json` and not tried again; scenes fetched under the earlier one-minute rule stay). **Single-axis scenes get generated twist / roll / pitch companions** (`npm run upgrade:axes`); hand-made axes are never touched. Every finished video joins the **Discovery** playlist, and the EroScripts folder is a library source named EroScripts (added if missing), so the scenes show in the library, the web remote, and HereSphere / DeoVR as `Playlist: Discovery`. Screening counts every library source as the library, the EroScripts folder included. `config.json` is written only while the app is closed (a backup is kept as `config.json.bak-discovery`); otherwise the videos wait in `eroscripts-scout-pending.json` until the next run. `--passes N --wait 1h` repeats the queue until nothing fails (MEGA and pixeldrain limit free downloads); finished scenes are skipped. `--ids 1,2` limits it to some posts, `--dry-run` lists the queue, `--incoming <dir>` changes the folder.
  - Scene names (the title the library, web remote and headset show) come from the post: **`Artist - Title (metadata)`**. The artist is the first leading `[Name]` / `(Name)` of the post title that names someone (labels such as `[VAM]`, `[Multi-axis]`, `[Fap Hero]` are skipped), else a trailing `by Name`, else the poster. Script labels (multi-axis, requested, suggested, commissioned, SR6, `… Script`) are dropped; other bracket parts and ` | ` segments become the metadata. A post with several videos names each by its own file (Iwara ids and `[Source]` dropped); a file name that says nothing (`7`) keeps the post title with the number as metadata. A VR projection suffix of the video name (`_180_3DH`, `_LRF_Full_SBS`) stays at the end so the headset still detects it.
  - `--retitle` renames scenes already in the EroScripts folder to that format, moving them to the artist's folder, and rewrites every path `config.json` keeps for them (playlists, script associations, caches; backup in `config.json.bak-discovery`). It needs the app closed (exit 2 otherwise); `--dry-run` lists the renames first. A second run changes nothing.
  - **Picking posts by id:** `npm run eroscripts:scout -- --add id1,id2,…` judges chosen Free Scripts posts (a favourite's back catalogue, say) and puts them on the list past the animation and multi-axis rules: the exclusions, the sex rule, posts the library already links or you turned down, and scenes already in the library still apply, single-axis scenes get generated twist / roll / pitch, and picks never use the live quota. Then `--fetch --ids id1,id2,…` downloads them. Only videos on the forum, MEGA or pixeldrain can be fetched; posts whose video lives elsewhere (spankbang, rule34video, Patreon) come back as `no-video`.
  - Exit codes: 0 done, 1 error, 2 rate limited, downloads still failing after the last pass, or the app running (`--retitle`), 3 not logged in / session expired.
- **FapTap scout: `npm run faptap:scout`** — the same Discovery flow for [faptap.net](https://faptap.net), through its JSON API (no account needed):
  - faptap.net serves a script only with the `track_token` cookie its pages set (valid an hour); without it the script endpoint answers HTTP 401 unless Cloudflare happens to have a copy cached. The scout loads the home page for a token and again whenever a script is refused.
  - Lists every FapTap video, newest first (`--pages N` of new videos, 42 each, default 5), and backfills `--backfill N` older pages per run (default 10) until the whole catalogue has been judged; verdicts are kept in `faptap-scout-state.json` in the data folder, `--full` judges again.
  - **Tags reuse the library's own.** FapTap's free-text tags are slugged like EroScripts tags (`Cum in Mouth` → `cum-in-mouth`), and FapTap spellings become the tags the library already uses, so FapTap scenes land in the existing categories and collections: `Cum Inside Pussy` / `Cowgirl Creampie` → `creampie`, `Reverse Cowgirl Position` → `reverse-cowgirl`, `Cock Riding` → `riding`, `PMV Compilation` → `pmv`, `Animated 3d` → `3d`, `Virtual Reality` → `vr`, and a character tag such as `Mercy (Overwatch)` adds its franchise (`overwatch`).
  - **Focus: at least two of PMV / HMV (Cock Hero and Fap Hero included), creampie, reverse cowgirl and VR**, each by tag or title (VR also by FapTap's VR flag); favourites don't need it. Anything else is turned down, and a saved list made under older rules is filtered the same way before `--fetch`. The scan table's Focus column shows which themes a candidate has, and marks favourites, picks and live action on the quota. **No audio roleplays**: erotic audio / ASMR tags (`asmr-audio-roleplay`, `erotic-audio-for-men` …) or titles (`[Erotic Audio Roleplay]`, ASMR, F4M) are turned down.
  - **Picking videos by id:** `npm run faptap:scout -- --add id1,id2,…` puts chosen videos on the list past the animation and focus rules (the exclusions, the sex rule, audio roleplays, the two-minute minimum and library duplicates still apply), then `--fetch --ids id1,id2,…` downloads them. Picks never use the live quota.
  - Same rules as the EroScripts scout except the multi-axis one: **every FapTap script is single-axis** (the site has no multi-axis scripts), so every scene is taken as single-axis and gets generated twist / roll / pitch. Never loli, shota, furry / anthro / yiff or futa / futanari (tags, title or description), nor furry tags that don't say so (scalie, knot, Krystal, Renamon, FNAF …). Animated means an animation tag (animated, animated 3d, hentai, cgi, sfm, mmd, ai-video …) and no live-action tag (amateur…, homemade, pornstar, onlyfans, jav …); because FapTap tags are free text, a video whose performer is a FapTap model / pornstar, or whose title names one, counts as live action, which follows the favourites / quota rule above. Music videos by a creator already in the library are taken too (a library folder named after a genre, such as `Cock Hero`, is no creator). Sex required (a blowjob / handjob / titjob alone isn't) and at least two minutes long, as on EroScripts; monsters and feet (tags or title) rank 0.5 lower each and 2D hentai only 3 lower. Videos the library (Discovery included) or the EroScripts scout list already has under the same title are skipped, and so are videos with no FapTap stream (only linked from another site).
  - Stream: the best FapTap (Bunny CDN) HLS stream up to 4K, 8K for VR, and none below 720p (1080p for VR, as some VR uploads only have a preview). FapTap encodes VR at 1080p at most, so VR from FapTap is soft in the headset. The VR projection becomes the file-name suffix the headset reads (`_180_LR`, `_180_TB`, `_MKX200`, `_360_LR` …).
  - `--fetch` downloads the list, best first, into the library's **FapTap folder**, `<library parent>/FapTap/<artist>/<Artist - Title (metadata)>/`, with a `faptap.json` sidecar (FapTap id, url, title, uploader, artist, performers, tags, thumbnail URL, VR projection); the folder is a library source named FapTap (added if missing). It needs `ffmpeg` and `ffprobe` on PATH: the stream is copied without re-encoding through `_staging/` and its length checked against FapTap's. The script (FapTap's `ms,pos` CSV, made a funscript) comes first; one that ends before two minutes isn't downloaded and is recorded in `eroscripts-scout-rejected.json`. Finished scenes join the **Discovery** playlist exactly like EroScripts ones, their tags feed the auto categories and collections, and 2 stars or lower deletes them and keeps the scout from taking them again. An EroScripts link found later by `eroscripts:sync` replaces the FapTap tags. `--ids a,b` limits the fetch, `--passes N --wait 1h` retries failures, `--dry-run` lists the queue.
  - **Scene names** follow the EroScripts convention, **`Artist - Title (metadata)`**, filed under the artist's folder. The artist is the person who made or stars in the video, not the scripter who uploaded it: a favourite named in the title, performers or tags (a one-word CamelCase favourite also matches its distinctive first part: `Scylla Compilation` → ScyllaHMV); else the title's own artist (leading `[Name]` / `(Name)`, `(by Name)`, trailing `by Name`; upload labels such as `[Script Overhaul]` don't count); else a library creator folder named in the title or listed as performer; else a performer FapTap calls a model or pornstar (its other performers are often characters); else the uploader. The artist's mentions leave the title, another leading `[Name]` becomes metadata, and a VR projection suffix ends the name. `--retitle` renames scenes fetched earlier to this format, filling in performers, thumbnail URL and projection from faptap.net for older sidecars, and rewrites the paths `config.json` keeps for them (app closed, exit 2 otherwise; `--dry-run` lists the renames first).
  - **One copy per video:** a FapTap video with the same title and length as a library scene (an EroScripts one always wins) is turned down as `duplicate`, and of several FapTap uploads of one video only the best stream (then the most liked) stays on the list.
  - Read-only and paced at ≥ 700 ms between requests. Exit codes: 0 done, 1 error, 2 rate limited or downloads still failing after the last pass.
- **Duplicate scenes: `npm run dedupe:scenes`** — finds videos the library holds more than once and keeps one. Both scouts run it after every pass that downloaded something (`--apply`).
  - The same video means: the same length (±1.5 s), the same projection and orientation (VR stays apart from flat, portrait from landscape), titles that match once quality words are dropped (`1080p`, `4k120fps`, `passthrough`, `(2)`, `PMV`…; the shorter title's words must all be in the longer one, so `Rina - …` never matches `Yixuan - …`), and either identical main scripts or the same picture (five small frames compared).
  - **Which copy stays**, first difference decides: EroScripts or a hand-made library scene over FapTap; hand-made axes over generated; passthrough over no passthrough; fewer breaches of the VR playback caps (≤ 8K, ≤ 60 fps); more pixels, then more fps (flat has no fps cap, so 4K 120 fps beats 1080p 60 fps); the faster main script (average speed); the higher bitrate.
  - Without `--apply` it only lists the groups and why each copy goes. With it, an extra copy is deleted the way triage deletes a scene (scripts, axes, sidecar, empty folders) and recorded in `eroscripts-scout-rejected.json` as `duplicate`, so neither scout fetches it again (the post's other videos still come). A second encode in the same folder as the kept one loses only its video file, since the two share their scripts. Playlists drop the deleted paths while the app is closed; with it open they stay, hidden as missing.
  - Before any download the EroScripts scan already makes one scene of a post's identical script files named per encode (`… 1080p60fps.funscript` / `… 4k120fps.funscript`, same sizes on every axis) and takes the best video for it; flat videos prefer more fps, VR keeps ≤ 8K, ≤ 60 fps and low bitrate.
  - Video facts are cached in `scene-probe-cache.json` in the data folder. Needs `ffmpeg` / `ffprobe`. Exit codes: 0 done, 1 error.
- **Library layout by source: `npm run library:reorganize`** — sorts every scene folder of the library into one folder per source, next to each other: `EroScripts/` (scenes with an `eroscripts.json`, and scenes without a sidecar whose video `eroscripts:sync` linked to a post), `FapTap/` (scenes with a `faptap.json`, even when later linked to the same script's EroScripts post) and `Other/` (the rest), each as `<source>/<artist>/<scene>/`. The scouts download into `EroScripts/` and `FapTap/` from then on.
  - Scene folders are renamed on the same drive (nothing is copied), each with its scripts, axis files and sidecar. Every path under a moved folder is rewritten in `config.json` (playlists, ratings, resume points, categories, collections, script pairings, VR settings, thumbnails …) and in the other JSON files in the data folder (EroScripts links, scout lists, caches), each backed up first as `<file>.bak-reorganize`; the library sources become `EroScripts`, `FapTap` and `Other`, and emptied old folders are removed. Rescan the library afterwards: auto categories, collections, artist playlists and thumbnails follow the new paths.
  - A scene whose target already exists, that two scenes would share, or that sits loose in a source root is left where it is and listed. `--dry-run` shows the counts, the moves and how many paths each file would get rewritten (the app may run for it); a real run needs the app closed (exit 2 otherwise). Each run is logged to `logs/library-reorganize-<time>.json` in the data folder, and `--undo <log>` puts it back. `--root <dir>` picks where the source folders go (default: next to the first library source).

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

### Content preferences
- **One file holds every video and script preference:** `renderer/js/content-preferences.js`. The scouts, the duplicate cleanup and the app read their values from it: excluded content, what counts as animated, live action and sex, the minimum length, the EroScripts feeds, the hand-made-axes rule, the library cap (`LIBRARY_MAX_VIDEOS`), the FapTap focus themes, favourite creators and the live quota, the ranking weights (2D hentai, monster and feet penalties included; the scouts' ranking and the library's preference score both use them), the VR and flat quality caps, which duplicate copy stays, the rating thresholds (delete / Review / keep / top rated), the artist-playlist threshold, and the auto category and collection definitions (the axis categories included). Change a value there (and bump the scouts' `RULES_VERSION` / `FAPTAP_RULES_VERSION` when a screening rule changes) instead of in the code that applies it.
- The headset's Top Rated threshold has a copy in `backend/services/vr_sections.py`; keep the two equal.

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
