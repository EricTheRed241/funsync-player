# MediaFire
Verdict: auto-with-limits

Seen on EroScripts: rare in the scout's data, fairly common on the forum overall. A forum search for `mediafire.com` (`/search.json?q=mediafire.com`, 3 pages) returned **142 topics**. Only 2 of them have been judged by the scout: **152554 → `no-video`** and 341989 → `known`. That is 1 of the 1,038 `no-video` posts, and none are in `.candidates`. Most of the hits are older or less-liked posts the scout never ranked; some are only mentions in replies or guides (for example 831 "Handy sleeve alternatives" has no link).

I sampled 12 topics (2021-04 … 2026-04) and found 15 links. Link shapes:
- `https://www.mediafire.com/file/<quickkey>/<url-encoded name>/file`. This is the usual shape (topics 289614 with 4 files, 297618, 218348 split RAR part1/part2, 161667, 34209, 30220 in reply 12, 309002 in reply 7, 22005, 37730, 152554).
- `https://www.mediafire.com/folder/<folderkey>/<name>` (104906 "HMV Hero scripts": 2 videos and 2 funscripts).
- Seen in the API, not in posts: `https://www.mediafire.com/file_premium/<quickkey>/<name>/file`. Older shapes are `/download/<key>`, `/view/<key>` and `/?<key>`, which redirect to `/file/<key>/…`.
- The forum's cooked HTML escapes `&` in names as `&amp;` (for example `MM.B&amp;VC.part1.rar`). Names are often double-encoded (`%255B` for `[`). Always take the name from the API, never from the URL.

How downloads work:
1. **Metadata (anonymous, no session):** `GET https://www.mediafire.com/api/1.5/file/get_info.php?quick_key=<key>&response_format=json` returns `response.file_info.{filename, size, hash (sha256), password_protected, privacy, links.normal_download}`. `result:"Error"` means the key is invalid or deleted.
2. **Folder listing (anonymous):** `GET https://www.mediafire.com/api/1.5/folder/get_content.php?folder_key=<key>&content_type=files&chunk=<n>&chunk_size=1000&response_format=json` returns `folder_content.files[] {quickkey, filename, size, hash, password_protected}`. Repeat while `more_chunks=="yes"`. For subfolders, use `content_type=folders`, which gives `folders[].folderkey`, then recurse. `folder/get_info.php` gives `name`, `file_count` and `folder_count`.
3. **Direct link:** `file/get_links.php?link_type=direct_download` needs an account: it answers `direct_download_error 45 "Insufficient Permissions"`. Instead, fetch the file page `https://www.mediafire.com/file/<key>/file` with a browser User-Agent. The HTML has
   `<a … aria-label="Download file" href="https://download<NNNN>.mediafire.com/<dkey-token>/<key>/<name>" id="downloadButton">`.
   - There is no countdown, cookie or CAPTCHA for normal files.
   - The page sometimes obfuscates the extension in the href, e.g. `….funscri'+'pt`. Strip `'+'` before use (the server tolerated it anyway).
   - The download host answers without any cookie or Referer.
   - Range requests work: `206`, `accept-ranges: bytes`.
   - The server sends `Content-Disposition: attachment; filename="<exact API filename>"`.
   - A token was still valid more than 2 minutes later.
4. **The page redirects in three other cases:**
   - `302 → error.php?errno=386` means the file is blocked for a ToS violation. `get_info` still reports it as `public`/`Success`, so get_info alone does not prove a file can be downloaded.
   - `302 → /file/<key>/…/file?action=check` leads to a Cloudflare managed challenge: `403`, `cf-mitigated: challenge`, "Just a moment…". Plain HTTP can't pass it. A real Chromium engine passes it **without any interaction**, and the page then shows the normal `#downloadButton`. The resulting direct link again works from plain Node `fetch` with no cookies.
   - Password-protected files (`password_protected:"yes"`) show a password form instead.

Limits:
- MediaFire advertises "Absolutely no sharing or download limits. Fast and free downloads for everyone" (text on its own page). No quota header or bandwidth error appeared in any probe.
- **The Cloudflare check depends on file size (observed).** All 5 files of 900 MiB or more redirected to `?action=check`: 943,718,400 B (exactly 900 MiB), 2.16 GB, 3.15 GB, 3.97 GB and 5.29 GB (the `no-video` post 152554). All 11 files of 812 MB or less got a direct button: 9 KB … 812,866,300 B.
  - The threshold lies between 775 and 900 MiB. MediaFire does not document it.
  - Repeated fetches and an EroScripts Referer did not change the result, so it is per file and persistent, not random.
- The page CSS contains a `#form_captcha` template. [INFERENCE] MediaFire can switch to a reCAPTCHA form when one IP downloads heavily. It never appeared during these probes (about 30 page loads in a few minutes).
- Owner removals and ToS blocks: 1 of 15 sampled files was blocked (errno 386). There is no time-based expiry on free files.

Probe results (2026-11-03, from this Mac, no account):
- `get_info.php`: 14/14 keys `Success` (sizes and names above).
- `folder/get_info` + `get_content` for `wfaj40sobap27`: 4 files, `more_chunks:"no"`.
- File pages (`curl -A <Chrome UA>`):
  - 11 × `200` with `#downloadButton`.
  - 1 × `404 error.php?errno=386` (`ybzhuri5ltzslc7`, topic 297618).
  - 2 × `302 ?action=check` → `403 cf-mitigated: challenge` (`988wlb5z6m1e6a6` 943 MB RAR from topic 218348; `gt9xnbh2gdz1tca` 3.97 GB from topic 309002).
  - The 2 large mp4s in folder `wfaj40sobap27` and `qg9hq0mv6p3lzh3` (post 152554, 5.29 GB) also got `action=check`.
- Range test on the direct links, with no cookie, no UA and no Referer:
  - `TAW1.mp4`: `bytes=0-1048575` → `206`, `content-range: bytes 0-1048575/229965247`, valid MP4 `ftyp` box. A mid-file range at 200 MB → `206`.
  - `swsn….mp4`: 1 MB → `206`.
  - `.funscript` (629 KB): full file → `206`, JSON.
  - `Content-Disposition` names equal the API names, including spaces, `[ ]` and `.funscript`.
- `get_links.php?link_type=direct_download` → error 45 "Insufficient Permissions" (needs a session).
- Cloudflare check, solved with a real engine:
  - Managed headless Chromium on `/file/gt9xnbh2gdz1tca/x/file`: `#downloadButton` appeared on the `?action=check` URL with no interaction. Node `fetch` of that href with no cookies, `bytes=0-1048575` → `206 bytes 0-1048575/3969368774`, `ftyp`.
  - Hidden Electron 41.2.0 `BrowserWindow` (`show:false`, the repo's own `node_modules/.bin/electron`, userData in /tmp) on `/file/qg9hq0mv6p3lzh3/file` → resolved `download1529.mediafire.com/<dkey>/qg9hq0mv6p3lzh3/Fap+Hero+-+Kingdom+Come.mp4` in **7.3 s**.
- All partial files were deleted after the checks.

Workarounds considered:
- **Plain HTTP page scrape:** works for files under about 800 MB. This is the main path.
- **`get_links.php` direct_download:** doesn't work; needs an account session (error 45).
- **Hidden Electron `BrowserWindow` for `?action=check` files:** works. It passed Cloudflare non-interactively in about 7 s, and the link it yields is cookie-free, so the existing Node stream/size-check path does the download. Electron is already a dependency, so nothing new is installed.
- **Playwright:** possible, but no Playwright browsers are installed (`~/Library/Caches/ms-playwright` is missing). Playwright 1.59 is only a transitive dependency. `channel:'chrome'` with `/Applications/Google Chrome.app` could also work. It is not needed when Electron is available.
- **Copying `cf_clearance` cookies into Node `fetch`:** not tested and not needed. Cloudflare binds clearance to the browser's TLS fingerprint/UA, so this is fragile.
- **CAPTCHA-solving services:** not considered (out of scope).
- **Referer/UA tweaks to skip the check:** don't work; it is per file.

Recommendation for the scout:
- **Detect:** `/https?:\/\/(?:www\.)?mediafire\.com\/(?:file|file_premium|download|view)\/([a-z0-9]{10,20})/gi` for files and `/https?:\/\/(?:www\.)?mediafire\.com\/folder\/([a-z0-9]{10,20})/gi` for folders. Replace `&amp;` before matching. Add the branch in `topicFiles()` next to the MEGA and pixeldrain branches. Do not send the forum cookie.
- **List:** call `get_info` per file or `get_content` per folder (chunks, and recurse into folders). Emit `{name: filename, size: +size, host:'mediafire', url: 'https://www.mediafire.com/file/<key>/file', key, sha256: hash}`. If `password_protected === 'yes'`, add the host as `'MediaFire (password)'` instead of the file.
- **Download** (`download()` branch `file.host === 'mediafire'`):
  1. `fetch(pageUrl, {redirect:'manual', UA})`.
  2. If the redirect has `errno=` → throw `mediafire: blocked/removed (errno N)`. This is permanent; mark the file gone.
  3. If the redirect has `action=check` → resolve with a hidden Electron window: spawn `electron scripts/mediafire-resolve.cjs <pageUrl>`, which prints `#downloadButton.href` and exits. Timeout 45 s.
  4. Otherwise regex the href, `fetch` it with no headers and pipe it to `.part` exactly like pixeldrain.
  - The existing `size !== file.size` check covers truncation. Optionally verify the sha256 against `hash`.
- **Retry:** tokens are short-lived but valid for minutes, so a failed attempt simply re-resolves the page. Range resume (`Range: bytes=<partSize>-` → `206`) is possible if the scout ever keeps `.part` files.
- **Fall back to the manual list:**
  - If the Electron resolver fails or times out (for example a reCAPTCHA appears): add `'mediafire'` to `MANUAL_HOSTS`, with `manualPageUrl` returning `https://www.mediafire.com/file/<key>/file`.
  - Password-protected files.
```js
const UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36';
const api = async (p, q) => { const j = (await (await fetch(`https://www.mediafire.com/api/1.5/${p}?${new URLSearchParams({ ...q, response_format: 'json' })}`, { headers: { 'User-Agent': UA } })).json()).response;
  if (j.result !== 'Success') throw new Error(`mediafire ${p}: ${j.message}`); return j; };
const asFile = (k, f) => ({ name: f.filename, size: +f.size, host: 'mediafire', url: `https://www.mediafire.com/file/${k}/file`, sha256: f.hash, locked: f.password_protected === 'yes' });
async function mediafireFiles(kind, key) {
  if (kind === 'file') return [asFile(key, (await api('file/get_info.php', { quick_key: key })).file_info)];
  const out = [];
  for (const type of ['files', 'folders']) for (let chunk = 1; ; chunk++) {
    const c = (await api('folder/get_content.php', { folder_key: key, content_type: type, chunk, chunk_size: 1000 })).folder_content;
    if (type === 'files') out.push(...c.files.map((f) => asFile(f.quickkey, f)));
    else for (const d of c.folders) out.push(...await mediafireFiles('folder', d.folderkey));
    if (c.more_chunks !== 'yes') break;
  }
  return out;
}
/** Direct (cookie-free) URL of a MediaFire file page, via Electron when Cloudflare checks it. */
async function mediafireDirect(pageUrl) {
  const r = await fetch(pageUrl, { headers: { 'User-Agent': UA }, redirect: 'manual' });
  const loc = r.headers.get('location') || '';
  if (/errno=(\d+)/.test(loc)) throw new Error(`mediafire: removed/blocked (errno ${loc.match(/errno=(\d+)/)[1]})`);
  let href;
  if (/action=check/.test(loc)) href = await resolveInElectron(pageUrl); // spawn electron scripts/mediafire-resolve.cjs, read stdout, 45 s timeout
  else href = (await (r.status === 200 ? r : await fetch(new URL(loc, pageUrl), { headers: { 'User-Agent': UA } })).text())
    .match(/aria-label="Download file"\s+href="(https:\/\/download\d+\.mediafire\.com\/[^"]+)"/)?.[1];
  if (!href) throw new Error('mediafire: no download button (CAPTCHA or password?) – use --manual-list');
  return href.replace(/'\+'/g, '');
}
// download(): const resp = await fetch(await mediafireDirect(file.url)); if (resp.status !== 200) throw …; pipeline(Readable.fromWeb(resp.body), createWriteStream(tmp))
// scripts/mediafire-resolve.cjs (Electron main): new BrowserWindow({show:false}).loadURL(argv.at(-1)); poll
//   executeJavaScript("document.querySelector('#downloadButton')?.href") every 1 s up to 30 s; console.log(href); app.quit().
//   Set app.setPath('userData', <tmp or scout dir>) so it never touches the player's profile.
```

Manual (browser) instructions if needed:
1. Open the file page (`https://www.mediafire.com/file/<key>/file`). For large files, a "Just a moment…" Cloudflare page passes by itself in a few seconds. If a checkbox shows, tick it.
2. Click the blue **Download (size)** button. Ignore pop-up tabs and ads; close them.
3. Keep the filename the browser offers. It equals MediaFire's file name, which the scout matches by name and size. Save it into `~/Downloads/for-fs` (or leave it in `~/Downloads`).
4. For a folder link, open each file in the folder view and download each one. Download every part of split RAR archives (`.part1.rar`, `.part2.rar`).
5. A page that says "file … has been blocked for a violation of our Terms" (errno 386) is gone. Skip that post.

Sources:
- MediaFire Core API 1.5, file: https://www.mediafire.com/developers/core_api/1.5/file/
- MediaFire Core API 1.5, folder: https://www.mediafire.com/developers/core_api/1.5/folder/
- MediaFire Core API 1.5, getting started: https://www.mediafire.com/developers/core_api/1.5/getting_started/
- Live probes on 2026-11-03 against the file pages, `download*.mediafire.com` and `error.php?errno=386` (its text: "The file you requested has been blocked for a violation of our…" and "Absolutely no sharing or download limits").
- EroScripts forum search `https://discuss.eroscripts.com/search.json?q=mediafire.com`.
