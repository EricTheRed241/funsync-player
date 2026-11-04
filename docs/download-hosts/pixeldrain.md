# pixeldrain
Verdict: auto-with-limits

**Summary:**
- **Works without a CAPTCHA:**
  - Every file whose `availability` is `""`. That was 132/175 files and 141.08 of 141.15 GB in today's candidate list. These download directly with `GET /api/file/<id>` and support Range/resume.
  - Hotlink-protected files can also be fetched through pixeldrain's documented ZIP routes: the multi-file `/api/file/<id1>,<id2>?download` and the list `/api/list/<id>/zip`. The pixeldrain page's own "Download all" button uses the list route without a CAPTCHA.
- **Needs a person:** only a direct single-file request for a protected file (`file_rate_limited_captcha_required`). This also applies if the ZIP route is closed or ruled out.

**What the probes showed:**
- 41 of the 43 protected files are funscripts of a few KB. The scout downloads scripts before the video, so a scene fails as soon as its first protected script gets a 403.
- The current `--manual-list` then sends the person to every pixeldrain file of the post, about 141 GB in total. Only 73 MB of that actually needs a CAPTCHA.

## Seen on EroScripts
- **How common:** 101 of 357 waiting candidates name the host `pixeldrain`. Their scenes resolve to 175 pixeldrain files in 85 posts, 141 GB in total. The forum search `pixeldrain.com/l/` returns its 50-hit cap: list links are common, mostly one list per scene (video + scripts).
- **Candidate examples:**
  - 344305, 315364, 116171, 88213, 296363, 125426, 291669: single video files, all unprotected today.
  - Posts with protected files:
    - 101425: both videos, `umq9eGWk` and `wguPZuZD`.
    - 131260, 239583, 240139, 247151, 279360, 317321, 337992, 341216, 342011, 344414, 345187: the scripts are protected.
- **List examples found via search (not candidates):**
  - 337168: lists `MLzX2Jjg`, `N5apkX3x`, … with 7 protected scripts plus 1 unprotected video each.
  - 249422: lists `MMFHuFNA`, `FyZQTfDk`, `euf8KYP3`.
  - 235914: 5 lists that are all `not_found` (deleted).
- **Link shapes:**
  - `https://pixeldrain.com/u/<id>` (file viewer).
  - `https://pixeldrain.com/l/<id>` (list viewer).
  - The same on `pixeldrain.net`, which serves the same API.
  - `/u/<id1>,<id2>` is legal: the viewer splits on commas.
  - `/api/file/<id>` direct links are possible.
  - IDs are 8 chars `[A-Za-z0-9]`. The current regex in `scripts/eroscripts-scout.mjs:176` and `electron/eroscripts-api.js:44` misses comma lists and `/api/file/` links.

## How downloads work
- **Info:**
  - `GET /api/file/<id>/info` takes up to 100 comma-separated IDs per call. A single ID returns an object; several return an array.
  - It returns `name`, `size`, `hash_sha256`, `views`, `downloads`, `availability`, `availability_message`, `can_download` and `download_speed_limit`.
  - `GET /api/list/<id>` returns `files[]` with the same fields, `availability` included. So `pixeldrainFiles()` already receives `availability` and drops it.
  - No auth is needed. Rate-limit headers are `X-Ratelimit-Limit: 3000` per IP, resetting every 60 s.
- **Direct download:** `GET /api/file/<id>` (`?download` only adds `Content-Disposition: attachment`).
  - Supports Range (`206`, `Accept-Ranges: bytes`).
  - For a protected file it answers `403 {"value":"file_rate_limited_captcha_required"}`. Adding `?download`, leaving it out, using `.net` or sending `Referer: https://pixeldrain.com/u/<id>` all give the same 403.
  - A `Wget/` User-Agent on `/u/<id>` is redirected `303 → /api/file/<id>`, which is still protected.
- **Multi-file ZIP (documented):** `GET /api/file/<id1>,<id2>,…?download` returns `200 application/zip`, `filename="Multiple files.zip"`.
  - It is a streamed deflate ZIP with data descriptors and no Range support.
  - It contains protected files as their real bytes, verified by inflating a protected funscript. `<id>,<id>` works too, giving 2 copies of the same entry; a single `<id>,` gives 404.
- **List ZIP:** `GET /api/list/<id>/zip` returns `200 application/zip`, `filename="<list title>.zip"`, in the same streamed format, and includes protected files (verified).
  - The web viewer's "Download all" (`Downloader.svelte` `download_list`) calls it without a CAPTCHA.
  - `/l/<id>` with a `Wget/` User-Agent is redirected there by pixeldrain's own server (`webcontroller/file_viewer.go:185-190`).
- **CAPTCHA flow on the page** (`svelte/src/file_viewer/Downloader.svelte`):
  - If `availability` ends with `_captcha_required`, the page loads Google reCAPTCHA v2 (site key from `GET /api/misc/recaptcha`).
  - On success it downloads `download_href + "&recaptcha_response=" + token`.
  - No cookie or session is set. The token is a Google reCAPTCHA response: valid for 2 minutes and verifiable only once. The browser's own download consumes it, so the scout cannot reuse it.
  - A free account's login cookie changes nothing: "For free downloads it makes no difference if you're logged in". Only a paid account on the downloader's side (API key via Basic auth) lifts hotlink protection.
- **IP quota:**
  - WebSocket `wss://pixeldrain.com/api/file_stats`, send `{"type":"limits"}`. The reply is `{server_overload, speed_limit, download_limit:10000, download_limit_used, transfer_limit:6000000000, transfer_limit_used}`.
  - Node ≥22 has a global `WebSocket`; this repo runs v26.4.

## Limits
- **Free IP limits:**
  - 6 GB transferred and 10,000 file downloads per IP, over a sliding 24 h window (FAQ "How does the transfer limit work?"). The viewer text says "per 48 hours"; the FAQ and limits table say 24 h / per day.
  - Beyond 6 GB, downloads continue at 1 MiB/s (limits table: "Download speed after limit 1 MiB/s") and previews are disabled.
  - The API may also return `ip_download_limited_captcha_required` / `ip_transfer_limited_captcha_required` / `transfer_limit_exceeded`. A CAPTCHA lifts these for that download.
  - At probe time this IP had already used **4.04 GB of 6 GB**. After my probes it was 4.12 GB: ZIP and Range bytes count too.
- **Hotlink protection is per file, global for everyone, not per IP.**
  - It is triggered when a file is downloaded far more often than its viewer page is viewed. The API docs say "three times more downloads than views".
  - The FAQ says "When enough people complete the test the hotlink protection will be removed".
  - Observed: protected files had downloads/views ratios of 2.6–12, while unprotected files reach 6.3. So the state is sticky and not a live ratio; re-check `availability` at fetch time.
  - Funscripts are hit hardest (41 of 55 candidate funscripts are protected). Players and script tools fetch them directly without opening the page.
- **Other limits:**
  - `max_concurrent_downloads` limits parallel connections for free IPs, with no number published. Download one file at a time.
  - Free uploads expire 60 days after the last view; 451 means takedown; 404 means gone.
  - API request limit: 3000/min/IP (headers).

## Probe results
Run on 2026-11-03 from this Mac's IP, with Node fetch and curl, no cookies.
- **Batch info:** `/api/file/<id>/info` for all 175 candidate file IDs (300 ms apart) returned 175×200: 132 with `availability:""` and 43 with `file_rate_limited_captcha_required`. The 43 protected files are 72.9 MB: 41 funscripts plus the 2 videos of post 101425.
- **Range on unprotected files:** `bytes=0-1048575` on `aW8UjpMB` (2.47 GB), `1MCbPpJ2` (8.67 GB) and `SMPiQgsW` (486 MB) returned 206, 1 MiB of valid MP4 (`ftyp`), and the correct `Content-Disposition` filename.
- **Protected `umq9eGWk`:** these all returned **403** (225-byte JSON): `?download`, no parameter, `pixeldrain.net`, and `Referer: /u/<id>`. The thumbnail still returned 206.
- **Multi-file ZIP:**
  - `umq9eGWk,wguPZuZD?download` (both protected) returned **200 ZIP, 71.5 MB**. Range was ignored, so this went past the 2 MB probe cap. I deleted the file right away; it is counted in the IP's quota.
  - `x2kFQP8q,1y7LMHEB` (2 protected funscripts) returned 200, 2.6 KB. `unzip -l` lists both with their correct sizes (7108/825), and the content is real funscript JSON.
  - `x2kFQP8q,x2kFQP8q` returned 200 with 2 copies; `x2kFQP8q,` returned 404.
- **List API and ZIP:**
  - `/api/list/<id>` on 16 lists from forum posts 235914/337168/249422/175634: 11 returned `success`, 5 returned `not_found`. Per-file `availability` is present.
  - `/api/list/MMFHuFNA/zip` (protected funscript + 98 MB video) returned 200 `application/zip`, with no `Accept-Ranges` and Range ignored. I aborted after 2 MB. The first entry is `(Dorokuma) Shupogaki.funscript` (deflate, flag 8); inflated it is 19,420 bytes of real JSON, equal to the info size. The second entry is the `.mp4`.
- **Other checks:**
  - The WebSocket limits call reported download_limit_used 8→22 and transfer_limit_used 4,044,631,646→4,124,949,943.
  - `robots.txt`: "Go ahead robots, do your worst".
  - Forum use: 7 requests (1 search + 6 topics), ≥800 ms apart, read-only.
  - All `/tmp` probe files have been deleted.

## Workarounds considered
- **`?download` vs no parameter, `pixeldrain.net`, Referer, browser-like UA:** doesn't work; all get the same 403.
- **Wait it out:**
  - Partly works, slowly. Protection lifts after enough people solve the CAPTCHA (FAQ), so a later run may find the file free.
  - Cheap check: re-read `availability` with batch info before each pass (1 request per 100 files).
- **Reuse the browser CAPTCHA result:**
  - Doesn't work. No cookie is set; the token travels in the download URL, is single-use and lasts 2 min, and the browser's own download spends it.
  - Catching the token before it is spent would need a browser extension or userscript on pixeldrain.com that hands it to the scout (the reCAPTCHA site key only renders on pixeldrain's domains). That is possible but not worth it for KB-sized scripts, since the browser already saves the file.
- **Free account / browser session cookie:** doesn't work. Free limits are per IP; logging in changes nothing.
- **Paid plan:** possible. Pro / Prepaid at €1 per TB with a €1 minimum balance. Downloading with that account's API key via Basic auth lifts hotlinking and the 6 GB cap. Not used.
- **Multi-file ZIP `/api/file/a,b?download`:** works for protected files (verified). It is documented in the API docs ("You can also download multiple files at once…"), but has caveats:
  - No Range or resume.
  - Deflate-compressed.
  - Counts against the 6 GB.
  - A lone protected file needs `id,id`, which costs twice the transfer (irrelevant for KB scripts, bad for videos).
- **List ZIP `/api/list/<id>/zip`:** works for protected files (verified). It is the same request the list page's "Download all" button makes without a CAPTCHA, and pixeldrain's server redirects `Wget` there itself. Same caveats as the multi-file ZIP, and it only applies when the post linked a list.
- **Caveat for both ZIP routes:** they sidestep the protection that exists so downloaders see pixeldrain's page (FAQ: "The download page is our primary source of new customers"). Their ToS doesn't forbid it and robots.txt is permissive, but pixeldrain could close the route without notice. Decision for Main/the user: I recommend using the ZIPs for the small protected scripts only and keeping the manual list as the fallback.
- **Opening `/u/<id>` before downloading to raise the "views" count:** not tested and not recommended. It games their hotlink metric.

## Recommendation for the scout
1. **Detect** with `/https?:\/\/(?:www\.)?pixeldrain\.(?:com|net)\/(u|l|api\/file|api\/list)\/([A-Za-z0-9]+(?:,[A-Za-z0-9]+)*)/gi`. Split the IDs on `,`; the `l` / `api/list` forms are lists.
2. **List files:**
   - Keep `pixeldrainFiles()`, but copy `availability`, `hash_sha256` and `id` onto each file object. Lists already carry them; the single-file `info` does too.
   - Use batch info (`/api/file/<≤100 ids>/info`) instead of one request per `/u/` link.
   - Remember which list a file came from (`listId`).
3. **Fetch, before each pass:**
   - Re-check `availability` with batch info.
   - Read `limits` over the WebSocket. If `transfer_limit_used + size > transfer_limit`, either defer big videos to the next run or accept 1 MiB/s, and log the remaining quota.
4. **Per file:**
   - `availability === ""`: direct `GET /api/file/<id>` with `Range: bytes=<have>-` resume into the `.part` file. Download serially, never in parallel (`max_concurrent_downloads`).
   - `file_rate_limited_captcha_required`, and the user has allowed the ZIP route:
     - Fetch all of the post's protected files in one request: `/api/list/<listId>/zip` if they all come from one list and the list is small, else `/api/file/<ids>?download`. Use `<id>,<id>` only when a single file is protected and it is a script (under 1 MB).
     - Save the ZIP to staging, then `unzip -o -j` / `bsdtar -xf` the wanted names.
     - Verify the size and `sha256` against the info response.
     - Fetch protected scripts before their scene's video, so the scene doesn't fail on a KB script.
   - `ip_*_captcha_required`, `transfer_limit_exceeded`, `download_limit_exceeded`: stop pixeldrain for this run (sliding 24 h) and keep the post queued.
   - `max_concurrent_downloads` or 429: wait 30–60 s, then retry up to 3 times.
   - 404 / 451: mark the file gone and drop the scene.
   - Protected and the ZIP route disabled or failing: add the file to the manual list.
5. **Manual list:** `manualDownloads()` should list only files whose fresh `availability` is non-empty, not every pixeldrain file (today that is 43 files and 73 MB instead of 175 files and 141 GB). `MANUAL_HOSTS` should become "files that failed with a CAPTCHA-class error".

```js
const PD = 'https://pixeldrain.com/api';
async function pdInfo(ids) {                       // ≤100 ids per request
  const out = [];
  for (let i = 0; i < ids.length; i += 100) {
    const d = await (await fetch(`${PD}/file/${ids.slice(i, i + 100).join(',')}/info`)).json();
    out.push(...(Array.isArray(d) ? d : d.success ? [d] : []));
  }
  return new Map(out.map((f) => [f.id, f]));       // {name,size,availability,hash_sha256}
}
function pdLimits() {                              // IP quota, sliding 24 h
  return new Promise((ok, bad) => {
    const ws = new WebSocket('wss://pixeldrain.com/api/file_stats');
    const t = setTimeout(() => { ws.close(); bad(new Error('pixeldrain limits: timeout')); }, 10000);
    ws.onopen = () => ws.send('{"type":"limits"}');
    ws.onmessage = (m) => { const j = JSON.parse(m.data); if (j.type === 'limits') { clearTimeout(t); ws.close(); ok(j.limits); } };
    ws.onerror = () => { clearTimeout(t); bad(new Error('pixeldrain limits: socket error')); };
  });
}
async function pdDirect(id, tmp) {                 // unprotected file, resumable
  const have = fs.existsSync(tmp) ? fs.statSync(tmp).size : 0;
  const r = await fetch(`${PD}/file/${id}?download`, { headers: have ? { Range: `bytes=${have}-` } : {} });
  if (!r.ok) { const e = await r.json().catch(() => ({})); throw Object.assign(new Error(`pixeldrain: ${e.value || r.status}`), { code: e.value, status: r.status }); }
  await pipeline(Readable.fromWeb(r.body), fs.createWriteStream(tmp, { flags: r.status === 206 ? 'a' : 'w' }));
}
async function pdZip(ids, listId, zipPath, outDir, names) {   // protected files, one request per post
  const url = listId ? `${PD}/list/${listId}/zip` : `${PD}/file/${(ids.length > 1 ? ids : [ids[0], ids[0]]).join(',')}?download`;
  const r = await fetch(url);
  if (!r.ok || !/zip/.test(r.headers.get('content-type') || '')) throw new Error(`pixeldrain zip: HTTP ${r.status}`);
  await pipeline(Readable.fromWeb(r.body), fs.createWriteStream(zipPath));   // no Range: restart on failure
  const x = spawnSync('unzip', ['-o', '-j', zipPath, ...names, '-d', outDir], { encoding: 'utf8' });
  fs.rmSync(zipPath, { force: true });
  if (x.status !== 0) throw new Error(`pixeldrain zip: unzip ${x.status}`);
}
// caller: verify size + sha256 against pdInfo(); on CAPTCHA-class 403 or zip failure → manual list
```
- **Resume and retry:** resume direct downloads with Range. ZIPs cannot resume, so restart them, and keep ZIPs to script-sized payloads. A ZIP must not be the way protected *videos* are fetched unless the user accepts restarting multi-GB transfers.
- **Electron side:** `electron/eroscripts-api.js` `_resolvePixeldrain` could likewise fetch protected funscripts through the ZIP route instead of opening the page.

## Manual (browser) instructions if needed
Only for files the scout reports as protected (CAPTCHA-class 403) and could not fetch:
1. Run `npm run manual:open`. Each post opens next to its pixeldrain pages. Smoother option: open one tab per post with the comma form `https://pixeldrain.com/u/<id1>,<id2>,…`, which pixeldrain's viewer accepts, instead of one tab per file.
2. On each file, press **Download** and tick the reCAPTCHA. The browser saves the file under pixeldrain's own name (`Content-Disposition` = the name in the list). Keep that name exactly, without " (1)" suffixes, and save it to `~/Downloads/for-fs` (`~/Downloads` also works).
3. If the post links a **list** (`/l/<id>`), the page's **Download all** button saves `<list title>.zip` with no CAPTCHA. Keep the ZIP in `~/Downloads/for-fs`; the scout would need to learn to take files out of it (today it matches by file name only).
4. Watch the quota: about 6 GB per 24 h per IP (sliding), then 1 MiB/s. After that, a CAPTCHA appears on every file and each download still works.
5. Then run `npm run eroscripts:scout -- --fetch --ids <ids>`.

**Ideas to make the manual flow smoother:**
- (a) List only protected files, mostly KB-sized scripts, after a fresh batch-info check.
- (b) Have the scout download the videos itself even when a script is protected: fetch unprotected files first, and park the scene as "waiting for script" instead of failing it.
- (c) Use the comma `/u/` URL per post to cut tabs.
- (d) Show each file's size and the remaining IP quota (from `pdLimits()`) at the top of `download-for-fs.txt`.
- (e) Accept `Multiple files.zip` / `<list title>.zip` in Downloads.

## Sources
- pixeldrain API docs, `GET /file/{id}` (hotlink rule "three times more downloads than views", `recaptcha_response`, comma multi-file ZIP, error codes): https://pixeldrain.com/api
- Limits table (6 GB/day, 10,000 downloads/day, 1 MiB/s after the limit, hotlinking premium-only): https://github.com/Fornaxian/pixeldrain_web/blob/master/res/include/md/limits.md
- FAQ (sliding 24 h window per IP, login irrelevant for free, how hotlink protection works and lifts): https://github.com/Fornaxian/pixeldrain_web/blob/master/res/include/md/about.md and https://pixeldrain.com/about
- Viewer CAPTCHA flow (`recaptcha_response` appended to download URL; list "Download all" via `/api/list/<id>/zip` without CAPTCHA): https://github.com/Fornaxian/pixeldrain_web/blob/master/svelte/src/file_viewer/Downloader.svelte, https://github.com/Fornaxian/pixeldrain_web/blob/master/svelte/src/file_viewer/FileViewer.svelte
- Hotlink page text and the "per 48 hours" wording: https://github.com/Fornaxian/pixeldrain_web/blob/master/svelte/src/file_viewer/viewers/RateLimit.svelte
- IP limits WebSocket `/api/file_stats`: https://github.com/Fornaxian/pixeldrain_web/blob/master/svelte/src/lib/StatsSocket.ts
- Wget redirects (`/u/` → `/api/file/`, `/l/` → `/api/list/<id>/zip`): https://github.com/Fornaxian/pixeldrain_web/blob/master/webcontroller/file_viewer.go (commit 05c0e2e, 2026-10-09)
- reCAPTCHA token validity (2 minutes, verified once): https://developers.google.com/recaptcha/docs/verify
- robots.txt: https://pixeldrain.com/robots.txt
