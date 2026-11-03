# Download hosts for the scouts

Research on 2026-11-03: how each file host and video site that EroScripts posts link to can be downloaded, probed live (listing calls, ≤ 2 MB range reads). One report per host, same layout: verdict, how often it shows up on EroScripts, how downloads work, limits, probe results, workarounds, a recommendation for the scout (URL regex, listing, download, retry, manual-list fallback, Node sketch) and browser steps.

Today the scout (`scripts/eroscripts-scout.mjs`) fetches only forum uploads, MEGA and pixeldrain, and reads links from the **first post** only; everything else ends up `no-video` (1,038 judged posts) or stuck in the candidate list. Nothing here is implemented yet.

| Host | Verdict | Why it matters | Report |
|---|---|---|---|
| Video sites (rule34video, hanime1, iwara, hmvmania, spankbang …) | auto-with-limits; newgrounds / private iwara by hand; Patreon / Fanbox no | ~73 % of `no-video` posts link one (rule34video 39 %, hanime1 20 %, iwara 12 %): the biggest win. rule34video and hmvmania work with plain `fetch`; the rest via yt-dlp (not installed: `brew install yt-dlp`) | [video-sites.md](video-sites.md) |
| MEGA | auto-with-limits | 80 of the waiting candidates. Keep and resume `.part` files (`download({start})`, verify after), sleep exactly the 509 wait; fix `mega.co.nz/#!`, `www.mega.nz`, subfolder and in-folder file links; mark deleted links as gone | [mega.md](mega.md) |
| pixeldrain | auto-with-limits | 73 candidates. 132 of 175 files (141 of 141 GB) download directly; the 43 CAPTCHA files are almost all scripts (73 MB). Download a scene's video even when a script is protected; manual list only the protected files; ZIP routes avoid the CAPTCHA (your call) | [pixeldrain.md](pixeldrain.md) |
| Gofile | auto-with-limits | Guest token + website token from gofile's own `wt.obf.js` (salt rotates; fetch it each run), `accountToken` cookie; 100 GB / 30 days per IP. Links are often in replies | [gofile.md](gofile.md) |
| Google Drive | auto-with-limits | `drive.usercontent.google.com/download?id=…&export=download&confirm=t`, resumable; folders via `embeddedfolderview`. Rare in first posts, common in replies | [google-drive.md](google-drive.md) |
| Workupload | auto-with-limits | Self-solving SHA-256 puzzle (the site's own), no resume; ~88 % of its links are dead (30-day expiry) | [workupload.md](workupload.md) |
| MediaFire | auto-with-limits | Direct link from the page; files ≥ ~900 MiB hit a Cloudflare check that a hidden Electron window passes | [mediafire.md](mediafire.md) |
| Dropbox | auto-with-limits | `dl=1` for files, the web page's own listing for folders; rare (~15 script topics) | [dropbox.md](dropbox.md) |
| OneDrive | auto-with-limits | Anonymous badger token + shares endpoint (undocumented); rare (~7 topics) | [onedrive.md](onedrive.md) |
| files.fm | auto | Torrent file holds a plain HTTPS URL; almost every EroScripts link is dead | [files-fm.md](files-fm.md) |
| TeraBox | browser-handoff | Downloads need a logged-in `ndus` cookie; most posts also have a pixeldrain copy; `EXTERNAL_HOSTS` misses 1024terabox / 1024tera | [terabox.md](terabox.md) |

Cross-cutting findings:
- **Read every post of a topic, not only the first:** Gofile (8 of 11 candidates), Drive and Workupload links are mostly in replies.
- **`EXTERNAL_HOSTS`** in `electron/eroscripts-api.js` misses several domains (TeraBox mirrors, `dl.dropboxusercontent.com`, `photos.onedrive.com`, `*.sharepoint.com`, `mega.co.nz`).
- **Dead links** (404 / 403 / ENOENT) should mark a file gone instead of putting it on the manual list.
- The manual list (`--manual-list`, `npm run manual:open`) is the fallback for any host; its instructions are in the README under "Downloads by hand".
