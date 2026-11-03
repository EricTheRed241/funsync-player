# Google Drive
Verdict: auto-with-limits

Seen on EroScripts: Rare in the scout's own pool, but common on the forum overall.
- Random sample of 40 of the 1,038 `no-video` posts (first post `cooked`, seed 7): **0 of 40 link to Drive**. Their hosts were patreon 20, rule34video 17, hanime1 6, iwara 5, thehandy 3, pixeldrain/mega/gofile/hmvmania/fantia 2 each, and singles (yandex disk, spankbang, newgrounds, gumroad …).
- Forum search `drive.google.com in:first category:14` (plain and `order:latest`) gave 62 first-post hits from 2020-11 to 2026-10. Of those, **only 2 were judged in `eroscripts-scout-state.json`** (304654 and 44527, both `not-sexual`). So Drive posts mostly sit outside the scanned feeds (older posts, live action, JAV, PMVs), not in `no-video`.
- Drive also shows up in **replies** a lot: mirrors and re-uploads such as 111743#16, 60727#28, 50951#22, 52780#18, 191641#9, 104486#8, 124554#5, 54841#6, 275933#8, 113466#10, 23267#10. The scout reads only post 1, so it never sees these.
- Real first-post examples:
  - Files: 259784 (SupaDupaFunk PMV, `…/file/d/1tU7LKlQjrll_QRmcsoMYUkbsNdyncSki/view?usp=sharing`), 256559, 285545, 224162 (`?usp=drive_link`)
  - Folders: 303017 (Midna PMV, `…/drive/folders/1Pi16zh_WTd9oee9q5cafUFAALlqWHarE?usp=sharing`, holds the .mp4 **and the .funscript**), 341395, 225338 (JAV VR, "I may delete them if I run out of space"; the folder is now empty), 57106 (Happy Face, in a reply)
- Link shapes:
  - `https://drive.google.com/file/d/<id>/view?usp=sharing|drive_link`
  - `https://drive.google.com/open?id=<id>` (307 → `/file/d/<id>/view`)
  - `https://drive.google.com/uc?export=download&id=<id>` and `https://docs.google.com/uc?export=download&id=<id>` (both 303 → `drive.usercontent.google.com/download?id=<id>&export=download`)
  - `https://drive.google.com/drive/folders/<id>?usp=sharing` (also `/drive/u/0/folders/<id>`)
  - IDs are `[-\w]{25,}` (seen: 33 chars, starting `1`).

How downloads work: All of this is anonymous: no cookies, no API key, any or no User-Agent (Node `fetch` defaults work).
1. **File**: `GET https://drive.usercontent.google.com/download?id=<id>&export=download&confirm=t`
   - The response carries the file directly: `Content-Disposition: attachment; filename="<name>"` and the real `Content-Type` (video/mp4, application/octet-stream).
   - It honours `Range`: `206` with `Content-Range: bytes a-b/<total>` and `Accept-Ranges: bytes`.
   - **The `uuid` is not needed**: `confirm=t` alone skips the "can't scan for viruses" page for large files (tested on 696 MB).
   - Without `confirm=t`, files under about 100 MB come straight back. Larger ones get a 200 `text/html` page titled "Google Drive - Virus scan warning", holding `<form id="download-form" action="https://drive.usercontent.google.com/download">` with hidden `id`, `export=download`, `confirm=t` and `uuid=<uuid>`, plus `<span class="uc-name-size">…name… (696M)</span>`.
2. **Metadata (name, size)**: the same URL with `Range: bytes=0-0` returns `206`, `Content-Range: …/<total bytes>`, and the filename in `Content-Disposition` (for non-ASCII names, also handle `filename*=UTF-8''…`). The `/file/d/<id>/view` page title is `<name> - Google Drive`.
3. **Folder listing, no API key**:
   - `GET https://drive.google.com/embeddedfolderview?id=<folderId>` returns plain HTML. The `<title>` is the folder name. Each entry is `<div class="flip-entry" id="entry-<fileId>">` with `<a href="https://drive.google.com/file/d/<fileId>/view…">` (subfolders: `href="https://drive.google.com/drive/folders/<id>"`), `<div class="flip-entry-title">name</div>` and a last-modified date. **No sizes** in this page; get them via step 2. gdown uses the same page.
   - Alternative with sizes: `GET https://drive.google.com/drive/folders/<id>?hl=en` (≈280 KB) embeds `window['_DRIVE_ivd'] = '<JS-escaped JSON>'`. In `d[0][i]`: `[0]` = id, `[1]` = [parent], `[2]` = name, `[3]` = mime, **`[13]` = size in bytes** (e.g. Midna PMV.mp4 54525348, .funscript 14633). This format is undocumented, so treat it as fragile.
4. **With an API key (not anonymous)**: `GET https://www.googleapis.com/drive/v3/files?q='<folderId>'+in+parents&fields=files(id,name,size,mimeType)&key=<KEY>` and `files/<id>?alt=media&key=<KEY>`. This needs your own Google Cloud project key (an account), which the rules don't allow. The scout doesn't need it.

Limits:
- **Per-file download quota**: Google doesn't publish numbers. A file that "too many users have viewed or downloaded recently" is locked for everyone anonymous. The page (title "Google Drive - Quota exceeded", `<p class="uc-error-subcaption">`) says "Sorry, you can't view or download this file at this time. Too many users have viewed or downloaded this file recently. Please try accessing the file again later. If the file you are trying to access is particularly large or is shared with many people, it may take up to 24 hours…".
  - It comes back as `text/html` with no `Content-Disposition`. [INFERENCE] The status is probably 200 rather than 403, since Google indexed such pages (search results above). So detect by content type and caption, not by status.
  - gdown's FAQ: a **signed-in** browser session is let through when anonymous access is throttled.
- **Owner deletions / private links**: an anonymous request then gets **HTTP 404** "Page Not Found" from both `drive.usercontent…/download` and `/file/d/<id>/view`. There's no separate "private" signal for anonymous users. 6 of 17 probed file IDs were 404; 1 of 4 folders (225338) is now empty.
- No CAPTCHA, no bandwidth cap per user and no wait timer observed. Speed: 2 MB in about 1.06 s from Node (one sample).
- Folder listing size: gdown used to cap folders at 50 files. Whether `embeddedfolderview` pages beyond that is untested; every EroScripts folder seen had 1–2 files. Google-native docs (`docs.google.com/<kind>/d/<id>`) in a folder are not videos: skip them.

Probe results (2026-11-03, curl and Node 26 `fetch`, no cookies):
- `download?id=1tU7…&export=download` (696 MB mp4) → 200 text/html virus-scan interstitial (2,487 B) holding the confirm/uuid form.
- Same with `&confirm=t&uuid=…`, `Range: bytes=0-2097151` → **206 video/mp4, `content-range: bytes 0-2097151/729477774`**, filename "SuperDuperFunk 60fps_blend_ 4K_h256.mp4". The 2 MB saved to /tmp start with an `ftypisom` box; file deleted.
- `&confirm=t` without uuid, `Range: bytes=500000000-501048575` → 206, 1 MB (resume from the middle works); deleted.
- 54 MB mp4 (1hMy…) without confirm → 206 directly. A `.funscript` (1Cvc…, 14,633 B) → 200 application/octet-stream, valid JSON.
- `drive.google.com/uc?…` and `docs.google.com/uc?…` → 303 to usercontent; `/open?id=` → 307 to `/file/d/<id>/view?usp=drive_open`.
- File IDs from 12 posts/replies with `Range: bytes=0-0&confirm=t`: 11 live (206), 6 gone (404 text/html "Error 404 (Not Found)"). `/file/d/<id>/view` for the gone ones → 404 "Page Not Found".
- `embeddedfolderview` on 4 folders → 200: Midna (2 entries), AICOS (1), Happy Face (2), VRKM-1458 (title only, 0 entries).
- Folder page `_DRIVE_ivd` parsed: field 13 = size.
- Two "Quota exceeded" pages indexed by Google (11fx3n…, 1muyfp…) were serving again (206), so the quota response itself couldn't be captured live.
- Forum: 54 requests (≥ 800 ms apart). The cookie was never printed or written.

Workarounds considered:
- **Plain `fetch` with `confirm=t`**: works; recommended, no dependency.
- **Parse the interstitial form (id, export, confirm, uuid)**: works, but isn't needed today. Keep it as a fallback in case Google starts requiring the uuid again (gdown does exactly this).
- **gdown 6.5.1** (`brew info gdown`, not installed): handles files, folders (`embeddedfolderview`), resume (`--continue`) and browser cookies for quota. It adds a Python dependency, and Node does the same in about 40 lines. Not needed.
- **yt-dlp GoogleDrive extractor** (not installed): works for single files. Not needed.
- **rclone `backend copyid`**: needs a configured Drive remote with OAuth (a Google account). Not anonymous; not recommended.
- **Drive API v3 with an API key**: gives clean listings with sizes, but needs a Cloud project. Possible with your own key; not needed.
- **Quota exceeded → signed-in session**: possible with the user's own Google cookies (gdown `--cookies-from-browser`). The scout must not read browser cookies, so fall back to the manual list instead. A person signed into Google in the browser normally gets past the anonymous quota; "Add shortcut / Make a copy" to their own Drive is the documented trick when even that fails.

Recommendation for the scout:
- **Detect** in `topicFiles()` (post 1 `cooked`, like pixeldrain): `/https?:\/\/(?:drive|docs)\.google\.com\/(?:file\/d\/|(?:open|uc)\?(?:[^"'\s]*&)?id=|drive\/(?:u\/\d+\/)?folders\/)([-\w]{25,})/g`. Treat `folders/` matches as folders, everything else as files. Also catch `drive.usercontent.google.com/download?id=`. Add `drive.usercontent.google.com` and `docs.google.com` to `EXTERNAL_HOSTS` so leftovers are still recognised.
- **List files**: a file → one `Range: bytes=0-0` probe for name and size. A folder → `embeddedfolderview`, recurse into subfolders (depth ≤ 3), then probe each file for its size; skip Google-native docs. Return `{name, size, host: 'gdrive', url: dlUrl(id)}`. Scripts on Drive are plain files too, so `.funscript` files come through the same path, which lets the scout judge length and axes before the video.
- **Download**: `fetch(url)` and stream to `.part`. Check `Content-Disposition` is present and the content type isn't `text/html`; anything else → `driveError(html)`.
- **Resume**: `Range: bytes=<partSize>-` and append; expect 206 with a matching total (it works mid-file). Retry 5xx and network errors with backoff.
- **Errors**:
  - 404 → `gone` (deleted or private; drop the file, no retry).
  - "Quota exceeded" / `uc-error-subcaption` "Too many users" → `quota`: keep the candidate and retry in a later pass (the `--wait` loop is fine; Google says up to 24 h). If it's still blocked after ~24 h, put it on the `--manual-list` (signed-in browser).
  - An interstitial that still appears with `confirm=t` → re-request using the form's hidden inputs (uuid).
- Don't send the forum cookie to Google.

```js
const GD_RE = /https?:\/\/(?:drive|docs)\.google\.com\/(?:file\/d\/|(?:open|uc)\?(?:[^"'\s<]*&(?:amp;)?)?id=|drive\/(?:u\/\d+\/)?(folders)\/)([-\w]{25,})/g;
const dlUrl = (id) => `https://drive.usercontent.google.com/download?id=${id}&export=download&confirm=t`;
const H = { 'User-Agent': 'Mozilla/5.0' };
function driveError(html, status) {
  if (status === 404) return Object.assign(new Error('gdrive: gone (deleted or not shared)'), { gone: true });
  if (/Quota exceeded|Too many users/i.test(html)) return Object.assign(new Error('gdrive: download quota exceeded, retry later (≤24 h) or by hand'), { retry: true, manual: true });
  return new Error(`gdrive: HTTP ${status}, no file (${(html.match(/<title>([^<]*)/) || [])[1] || 'html'})`);
}
async function driveFile(id, name) {
  const r = await fetch(dlUrl(id), { headers: { ...H, Range: 'bytes=0-0' } });
  if (!r.headers.get('content-disposition')) throw driveError(await r.text(), r.status);
  await r.body?.cancel();
  const cd = r.headers.get('content-disposition');
  const fn = name || decodeURIComponent((cd.match(/filename\*=UTF-8''([^;]+)/) || [])[1] || '') || (cd.match(/filename="([^"]+)"/) || [])[1];
  return { name: fn, size: Number(r.headers.get('content-range')?.split('/')[1]), host: 'gdrive', url: dlUrl(id) };
}
async function driveFolder(id, depth = 0) {
  const html = await (await fetch(`https://drive.google.com/embeddedfolderview?id=${id}`, { headers: H })).text();
  const out = [];
  for (const m of html.matchAll(/href="https:\/\/drive\.google\.com\/(file\/d|drive\/folders)\/([-\w]{25,})[^"]*"[\s\S]*?flip-entry-title">([^<]*)/g)) {
    const name = m[3].replace(/&amp;/g, '&').replace(/&#39;/g, "'").replace(/&quot;/g, '"');
    if (m[1] === 'file/d') out.push(await driveFile(m[2], name));
    else if (depth < 3) out.push(...await driveFolder(m[2], depth + 1));
  }
  return out;
}
// topicFiles(): for (const m of cooked.matchAll(GD_RE)) try { files.push(...(m[1] ? await driveFolder(m[2]) : [await driveFile(m[2])])); } catch (e) { hosts.add(e.gone ? 'Google Drive (gone)' : 'Google Drive (unreadable)'); }
// downloadFile(): else if (file.host === 'gdrive') {
//   const have = fs.existsSync(tmp) ? fs.statSync(tmp).size : 0;
//   const resp = await fetch(file.url, { headers: { ...H, ...(have ? { Range: `bytes=${have}-` } : {}) } });
//   if (!resp.headers.get('content-disposition') || /text\/html/.test(resp.headers.get('content-type'))) throw driveError(await resp.text(), resp.status);
//   await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp, { flags: resp.status === 206 ? 'a' : 'w' })); }
```
(The current scout deletes `.part` files on failure, so the resume branch only helps if that policy changes; a plain restart works as well.)

Manual (browser) instructions if needed: Only for "Quota exceeded" files that stay blocked after a retry, or a folder the listing can't read.
1. Sign in to your Google account in the browser, then open the Drive link from the post (`https://drive.google.com/file/d/<id>/view`, or the folder).
2. Click the download icon (⤓) at the top right. For a large file, Drive shows "Google Drive can't scan this file for viruses": click **Download anyway**. For a folder, right-click the file → Download. Downloading a whole folder gives a ZIP; prefer the individual files.
3. Keep the **filename exactly as Drive gives it**: it's the name the scout lists, and it matches by name and size. Save into `~/Downloads/for-fs/`.
4. If it still says "Quota exceeded" while you're signed in: click "Add shortcut to Drive" (or Make a copy) and download that copy from your own Drive, or try again within 24 h.

Sources:
- Probes above (2026-11-03): https://drive.usercontent.google.com/download?id=1tU7LKlQjrll_QRmcsoMYUkbsNdyncSki&export=download&confirm=t , https://drive.google.com/embeddedfolderview?id=1Pi16zh_WTd9oee9q5cafUFAALlqWHarE
- gdown source, confirmation form and `uc-error-subcaption` handling: https://github.com/wkentaro/gdown/blob/main/gdown/download.py
- gdown folder listing via `embeddedfolderview`: https://github.com/wkentaro/gdown/blob/main/gdown/download_folder.py
- gdown FAQ (throttling, a signed-in session gets through, `--cookies-from-browser`): https://github.com/wkentaro/gdown#faq
- Quota-exceeded page text (indexed): https://drive.usercontent.google.com/download?confirm=t&export=download&id=11fx3nNjSYB0X8vKxLAbYOrS2Bu6avm9A
- Google community threads on "Download quota exceeded": https://support.google.com/drive/thread/258523607 , https://support.google.com/drive/thread/2035857
- Drive API v3 files.list / alt=media (key path, not used): https://developers.google.com/drive/api/reference/rest/v3/files/list
- Forum posts: https://discuss.eroscripts.com/t/259784 , /t/303017 , /t/341395 , /t/225338 , /t/256559 , /t/285545 , /t/224162 , /t/57106
