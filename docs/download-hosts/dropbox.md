# Dropbox
Verdict: auto-with-limits

Seen on EroScripts: Rare. Ran a forum full-text search on 2026-11-03: `dropbox.com` matched 40 posts across the whole forum, with no further result pages. `dropbox` matched 59 posts, including chat, and `dropboxusercontent` matched 3. About 15 script topics carry a Dropbox file/folder link in post #1. Examples: 41856, 41911, 42853, 58921, 58931, 68710 (zip), 68797 (zip), 99599 (zip), 151674, 203111 (folder), 210430 (gif preview), 298264, 331838, 333354, 337363. Reply-only links: 2951#12, 68697#3, 124381#14, 178448#3, 216965#8 (Transfer).
None of these are in the scout's queue. The candidate list has 0 Dropbox hosts. Of these topics only 298264 is in the scout state (`excluded`). In a 20-post evenly spaced sample of the 1,038 `no-video` posts, 0 had Dropbox or OneDrive links; those posts mostly link patreon/rule34video/iwara/hanime1. The other topics have probably not been reached by the scout's feeds yet, since 331838/333354/337363 are from Aug–Sep 2026.
Link shapes seen:
- legacy file `https://www.dropbox.com/s/<15 char id>/<name>?dl=0` (2021–2024)
- current file `https://www.dropbox.com/scl/fi/<21 char id>/<name>?rlkey=<25 char>&st=<8>&e=<n>&dl=0`
- folder `https://www.dropbox.com/scl/fo/<id>/<secure_hash>/<sub path>?preview=<file>&rlkey=<k>&subfolder_nav_tracking=1&st=..&dl=0`
- Transfer `https://www.dropbox.com/t/<id>` → 301 `/transfer/<token>`
- `https://dl.dropboxusercontent.com/...` (3 old replies)
- Not seen but exists: legacy folder `/sh/<id>/<hash>`.

How downloads work:
- **Single file** (`/s/` or `/scl/fi/`): set `dl=1` (Dropbox's documented "force download"). The redirect chain is: `/s/<id>/<name>?dl=1` → 302 to `/scl/fi/<id>/<name>?rlkey=…&dl=1` → 302 to `https://uc<hex>.dl.dropboxusercontent.com/cd/0/get/<token>/file?dl=1`. That last host serves `200`/`206` with `Content-Type: application/binary`, `Accept-Ranges: bytes`, `Content-Disposition: attachment; filename="…"; filename*=UTF-8''…` and a full size in `Content-Range`. Rewriting the host to `dl.dropboxusercontent.com/scl/fi/…?rlkey=…` (no `dl`) or using `raw=1` also gives 206 `video/mp4`. No cookies or tokens are needed, and the `st=` / `e=` / `preview=` params are optional.
- **Dead or disabled link**: the `dl=1` request ends in **HTTP 200 `text/html`** instead of a 302. The page `<title>` is "Dropbox - Error" (disabled or taken down) or "Dropbox - File Deleted". Treat `text/html` as failure.
- **Folder** (`/scl/fo/`), two ways:
  1. `…/scl/fo/<id>/<hash>/<sub>?rlkey=<k>&dl=1` → 302 to `uc….dl.dropboxusercontent.com/zip_download_get/<token>`. This answers `200 application/zip` with a Content-Length (2,912,557,494 bytes for 203111) and `filename="<folder>.zip"`. The ZIP is "store" (uncompressed). It ignores `Range` (200, not 206), so it **cannot be resumed**.
  2. **Better: list the folder the way the web page does.** First `GET https://www.dropbox.com/scl/fo/<id>/<hash>?rlkey=<k>&dl=0` to collect the `Set-Cookie` values `t=` (CSRF) and `gvc=`. Then `POST https://www.dropbox.com/list_shared_link_folder_entries` with form body `is_xhr=true&t=<t>&link_key=<id>&link_type=s&secure_hash=<hash>&sub_path=/<decoded sub path>&rlkey=<k>`, sending the cookies and `X-Requested-With: XMLHttpRequest`. It returns JSON `{entries:[{filename, bytes, is_dir, href}], has_more_entries, next_request_voucher, …}`. Each `href` is a per-file `…/scl/fo/<id>/<file hash>/<path>?rlkey=<k>&dl=0`; set `dl=1` to get a resumable 206 file download, as above.
- **Size and name without downloading**: `GET` with `Range: bytes=0-0` → 206 `Content-Range: bytes 0-0/<size>` plus `Content-Disposition`. Node `fetch` `HEAD` returned no Content-Length, so don't rely on HEAD.
- **Official API** (`sharing/get_shared_link_metadata`, `files/list_folder` with `shared_link`): needs an app access token, i.e. a Dropbox account and app key. That's "possible with a Dropbox app", but not needed.
- **Transfer links** (`/t/…`, `/transfer/…`): the server returns only a JS app shell (HTML, 200). No download URL is visible without running the page.

Limits:
- The owner's account pays for the bandwidth. Per Dropbox Help: Basic/trials get 20 GB and ≤100,000 downloads/day; Plus/Professional/Business 1 TB/day; Advanced/Enterprise 4 TB/day.
- Going over the limit pauses the owner's links for 24 h, and longer for repeat offenders. "Link temporarily disabled" means one link was downloaded too often in 24 h.
- Links also die from DMCA, malware flags or deletion. 2 of the 7 links probed were dead.
- Folder ZIP download only works for folders under 250 GB (uncompressed) with fewer than 10,000 files, and the ZIP is not resumable.
- The resolved `cd/0/get/<token>` URLs are temporary. Re-resolve from the shared link on every (re)try [INFERENCE: lifetime not measured].
- No CAPTCHA, no IP quota seen, no region block seen. Probed from macOS with no cookies.

Probe results (2026-11-03; each ≤1–2 MB, temp files deleted):
- `/s/vsts8nh0qx6o6o0/stepmom-handy.zip?dl=1` (68710): 302→302→**206**, `bytes 0-1048575/108764796`, valid Zip header.
- `/s/2odenwsjme3f68r/Watch%20the%20lights.mp4?dl=1` (151674): 302→302→**206**, `/306062008`, ISO MP4.
- `/scl/fi/v8bx…/RebeccaQuickie-4kA.mp4?rlkey=…&dl=1` (337363): 302→**206**, `/158395984`. A mid-file Range `bytes=100000000-100999999` also returned **206**, so resume works.
- `/scl/fi/ghct…/RebeccaClubSpecials-4kA.mp4?rlkey=…&dl=1` (333354): **206**, `/188014387`.
- `curl -IL` on that file gives 200 `content-length: 188014387`, and `dl.dropboxusercontent.com/scl/fi/…` and `raw=1` each give **206** `video/mp4`.
- `/s/7b4f…/Hot teen….mp4?dl=1` (41856): **200 text/html** "Dropbox - Error" (link disabled).
- `/scl/fi/2t01…/Poke_Gardenia_Calprpesten.mp4` (298264): **200 text/html** "Dropbox - File Deleted".
- Folder 203111 with `dl=1`, sub path `/Anime Stuff`: 302 → **200 `application/zip`**, 2,912,557,494 bytes, `Range` ignored. The same folder without the sub path: 200 HTML "No Access".
- `list_shared_link_folder_entries` with `sub_path=/Anime Stuff`: **200 JSON**, 19 files with names, sizes and hrefs, `has_more_entries:false`. With `sub_path=""`: 404. Always pass the path segment after `<hash>`.
- Per-file href `…/AAKvfCthvrGAz6KJpRhSnow/Anime%20Stuff/ChoCho%20PATREON.mp4?rlkey=…&dl=1`: 302→**206** `/154884704`, `filename="ChoCho PATREON.mp4"`. Note that the post's `preview=ChoCho%2BPATREON.mp4` means "ChoCho PATREON.mp4": the `+` is a space.
- Trying the file path inside the folder URL directly (`…/<folder hash>/Anime%20Stuff/ChoCho%2BPATREON.mp4?dl=1`): 200 HTML. Use the listing's hrefs instead.
- Transfer `https://www.dropbox.com/t/0vUDyMVnlbsxusZ6` (216965): 301 → `/transfer/AAAA…` 200 HTML app shell. Whether it is still live is unknown.
- Tools: `yt-dlp`, `rclone`, `aria2c` and `gallery-dl` are not installed (`which` checked). Node `fetch` and curl are enough.

Workarounds considered:
- `dl=1` on file links: **works** with no auth; resumable.
- Folder `dl=1` ZIP: **works** but is not resumable and is limited to 250 GB / 10k files. It also forces downloading the whole folder (2.9 GB for one wanted 155 MB video). Use only as a fallback.
- Web listing endpoint (`list_shared_link_folder_entries`): **works**, and lets you pick only the wanted file. It is undocumented and may change, so fall back to the manual list if it returns non-JSON.
- Official Dropbox API: works only with an app token. That needs an account and is out of scope.
- Transfer links: **not feasible** headless without running Dropbox's JS. Hand off to the browser (rare: 1 seen, and probably expired).
- Bandwidth-banned links: nothing to do but wait 24 h or go manual. A browser does not help, because the same owner quota applies.

Recommendation for the scout:
- Detection regex over `cooked` hrefs (after `&amp;`→`&`): `/https?:\/\/(?:www\.|dl\.)?dropbox(?:usercontent)?\.com\/(s|scl\/fi|scl\/fo|sh|t|transfer)\/[^\s"<]+/gi`.
- Also add `dropboxusercontent.com` to `EXTERNAL_HOSTS`. Today `dropbox.com` matches `www.`/`dl.dropbox.com` but not `dl.dropboxusercontent.com`.
- In `topicFiles()`, add a `dropboxFiles(url)` next to `pixeldrainFiles`. It returns `{name,size,host:'dropbox',url}`. In `downloadFile`, a `file.host === 'dropbox'` branch fetches `url` (already `dl=1`); if the response is `text/html` it throws `dropbox: link dead/disabled (manual list)`.
- To resume: re-request the shared `dl=1` URL with `Range: bytes=<have>-`; Dropbox answers 206. Retry later on 429/5xx (none were seen).
- Put `/t/` and `/transfer/` links straight into the manual list (`MANUAL_HOSTS`, `manualPageUrl` = the link itself).
- Put a file on the manual list when it gets HTML twice, or when folder listing fails.

```js
const DBX_RE = /https?:\/\/(?:www\.|dl\.)?dropbox(?:usercontent)?\.com\/(?:s|scl\/fi|scl\/fo)\/[^\s"<]+/gi;
const UA = { 'User-Agent': 'Mozilla/5.0' };
const dbxDl = (u) => { const x = new URL(u); x.hostname = 'www.dropbox.com';
  for (const k of ['st', 'e', 'preview', 'subfolder_nav_tracking']) x.searchParams.delete(k);
  x.searchParams.set('dl', '1'); return x.href; };
const cdName = (cd) => decodeURIComponent((cd || '').match(/filename\*=UTF-8''([^;]+)/i)?.[1] ?? '');

async function dropboxFiles(link) {
  const u = new URL(link);
  const [, , kind, key, hash, ...sub] = u.pathname.split('/');      // '', 'scl', 'fo'|'fi', …
  if (u.pathname.startsWith('/s/') || kind === 'fi') {
    const r = await fetch(dbxDl(link), { headers: { ...UA, Range: 'bytes=0-0' } });
    await r.body?.cancel();
    if (r.status !== 206) throw new Error('dropbox: dead or disabled link');
    return [{ name: cdName(r.headers.get('content-disposition')),
      size: Number(r.headers.get('content-range').split('/')[1]), host: 'dropbox', url: dbxDl(link) }];
  }
  const rlkey = u.searchParams.get('rlkey');                          // folder: list like the web page
  const page = await fetch(`https://www.dropbox.com/scl/fo/${key}/${hash}?rlkey=${rlkey}&dl=0`, { headers: UA });
  const cookies = page.headers.getSetCookie().map((c) => c.split(';')[0]);
  const t = cookies.find((c) => c.startsWith('t='))?.slice(2);
  const body = new URLSearchParams({ is_xhr: 'true', t, link_key: key, link_type: 's', secure_hash: hash, rlkey,
    sub_path: sub.length ? '/' + sub.map(decodeURIComponent).join('/') : '' });
  const r = await fetch('https://www.dropbox.com/list_shared_link_folder_entries', { method: 'POST', body,
    headers: { ...UA, Cookie: cookies.join('; '), 'X-Requested-With': 'XMLHttpRequest' } });
  if (!/json/.test(r.headers.get('content-type'))) throw new Error(`dropbox: folder listing HTTP ${r.status}`);
  const j = await r.json();                                           // has_more_entries paging: not seen, not probed
  return j.entries.filter((e) => !e.is_dir).map((e) => ({ name: e.filename, size: e.bytes, host: 'dropbox', url: dbxDl(e.href) }));
}
// downloadFile branch: GET file.url with Range: bytes=<have>- → expect 206; 'text/html' ⇒ manual list.
```

Manual (browser) instructions if needed:
- **Single file**: open the post's Dropbox link and click **Download** (top right; on `/scl/fi/` pages choose "Download" → "Download" if asked to sign in, or close the sign-in box). Keep the filename Dropbox gives (the original upload name) and save it to `~/Downloads/for-fs`.
- **Folder**: open the link and tick only the wanted video (the post's `preview=` value names it; `+` = space). Click **Download** and save the single file to `~/Downloads/for-fs`. Avoid "Download all" (a whole-folder ZIP).
- **Transfer** (`/t/…`): open it and click **Download**. If it says the transfer expired, the file is gone.
- **"Link temporarily disabled" / "sharing activity interrupted"**: retry after 24 h. "Error"/"File deleted" means the file is gone.

Sources:
- https://help.dropbox.com/share/force-download (`dl=1`, `raw=1`, `/scl/fi/…?rlkey=…&dl=1` form)
- https://help.dropbox.com/share/banned-links (20 GB / 1 TB / 4 TB per day, 100,000 downloads/day, 24 h pause, "Link temporarily disabled")
- https://help.dropbox.com/sync/download-entire-folders (250 GB / 10,000-file folder download limit)
- https://help.dropbox.com/share/dropbox-transfer (Transfer links)
- Live probes listed above (2026-11-03); forum searches via `https://discuss.eroscripts.com/search.json?q=…` and `/t/<id>.json`.
