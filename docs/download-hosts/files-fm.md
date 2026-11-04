# Files.fm
Verdict: auto (rare host, most links dead)

Seen on EroScripts: rare. A forum search for `files.fm` returned **14 topics**, all from 2021-11 … 2024-07. None of them have been judged by the scout: there are 0 in `no-video` and 0 in `.candidates`. Following the links in those topics gave **11 distinct links**:
- **8 folders** in the shape `https://files.fm/u/<hash>`: 124381 (reply 14), 45883, 45884, 44394, 46003, 46009, 126996 (reply 7), 176128, and 175593 (which re-posts bk67pgxbcw).
- **3 files** in the shape `https://files.fm/f/<hash>`: 78841, 76216, and 40059 (reply 4).
- Hashes are lowercase alphanumeric, 9–16 characters; the docs say to treat them as opaque.
- Mirrors of the same service: `xx.files.fm` language subdomains (`es.`, `ru.`) and the Latvian original `failiem.lv`.
- Raw links like `https://files.fm/down.php?i=<file hash>` are also possible.

**Only 2 of the 11 are still alive:** `f/pjv9kswhm` "RIDE 35 - PMV.mp4", 386 MB, from topic 78841, and `f/ymwhh2449`, from topic 76216. Both sit in the uploader's folder `wm6p44j45`, which is now "Private Access". The other 8 folders and `f/ca3ftrhvs` show "Files are deleted / File is deleted — Expiry date is over / Owner has deleted files".

How downloads work:
- **Plain `down.php` does not work for scripts.** The documented legacy link `https://api.files.fm/down.php?i=<file_hash>` (also on `files.fm` / `fvN-M.files.fm`) answers `302 → /f/<hash>` without a session. With the session cookie from visiting the page it answers `302 → /f/<hash>?start_p2p_download`. The web UI funnels every free download into its "Peer-Powered Download" (WebTorrent) form.
- **The torrent is fetchable anonymously.** That form's `.torrent` link is anonymous and needs no cookie:
  - **File:** `GET https://files.fm/torrent/get_torrent.php?file_hash=<hash>` returns `200 application/x-bittorrent`:
    - `info.name`: the exact file name.
    - `info.length`: the size in bytes.
    - `url-list`: a **plain HTTPS webseed** such as `https://fv9-1.failiem.lv/Files.fm_2022-07-11_file_pjv9kswhm/RIDE%2035%20-%20PMV.mp4`.
  - **Folder:** `GET …/get_torrent.php?folder_hash=<hash>` returns:
    - `info.name`: `Files.fm_<date>_<hash>`.
    - `info.files[] {length, path[]}`: the folder's files.
    - `url-list`: a base URL such as `https://fv5-6.failiem.lv/`. The file URL is `base + encodeURIComponent(info.name) + '/' + path.map(encodeURIComponent).join('/')`, the BitTorrent BEP-19 webseed convention.
  - Deleted, private, password-protected and probably paid items answer `403 {"Success":false,"ErrorCode":403,"ErrorMessage":"Torrent generation failed"}`.
- **The webseed URL is ordinary HTTP:**
  - No cookie, UA or Referer is needed.
  - Range works: `206`, `content-range` and `content-length` present.
  - Type is `application/octet-stream`, with no `Content-Disposition` header, so take the name from the torrent.
  - Only the `file_<hash>/<name>` part matters: the date prefix and `failiem.lv` vs `files.fm` both worked.
  - These are the same requests the page's own WebTorrent client makes to its webseeds.
- **Folder listing (documented alternative):** `GET https://api.files.fm/api/get_file_list_for_upload.php?hash=<folder_hash>&include_folders=1`. It works anonymously only for `LINK`/`PUBLIC` folders, and returns `[{hash, name, Size, type:"File"|"Folder", mime_type, …}]`. Each anonymous call is deliberately delayed by about 2 s and rate-limited per IP (`429 {"error":"rate_limited"}`). Private or deleted folders return `{"error":"not_authenticated_user"}`. The V2 `api_v2/get_folder_content` always needs authentication.
- **Page HTML (fallback for metadata):** `/f/<hash>` embeds `arrHashes = ["<hash>"]`, `arrFileHost = ["fv9-1.files.fm"]` and `arrDisplayNames = ["RIDE 35 - PMV.mp4"]`, plus item attributes `file_hash`, `display_name` and `size="386340367"`. A deleted item shows the text "File(s) is/are deleted"; a private folder shows `This folder is set to "Private Access"`.
- **ZIP of a folder:** `https://api.files.fm/server_scripts/zip/zip_streamer/upload_zip_streamer.php?uhash=<folder_hash>`. The docs say "Limits may apply". I did not probe it (unknown size; per-file webseeds are better).

Limits:
- **No download quota or CAPTCHA on the webseed path.** None was observed, and the docs list none for `LINK` downloads. The CAPTCHA widgets in the page are only for signing up or signing in.
- The docs say "API requests may be rate-limited or blocked by automated web firewall checks if request patterns look abusive". The anonymous listing is about 2 s per call and rate-limited per IP (exact numbers unpublished).
- Speed: a 2 MB range came at about 1.66 MB/s from fv9-1 (single sample, 0.41 s TTFB).
- **Expiry is the real limit.** Free or anonymous uploads expire: 9 of 11 EroScripts links (82 %) were "Expiry date is over / deleted", the oldest posts being from 2021–2022. The Pro page markets "Password and link expiration date" and "Recover deleted files for 30 days". Retention for free uploads is not published.
- Files that are password-protected, paid (`price` set) or have downloads disabled cannot be fetched.

Probe results (2026-11-03, no account, no cookies unless noted):
- `GET files.fm/u/<hash>` for the 8 folders: `200`, generic title, "Files are deleted". `GET files.fm/f/ca3ftrhvs`: "File is deleted". `GET /f/pjv9kswhm` and `/f/ymwhh2449`: `200`, titles "RIDE 35 - PMV" / "PUSSY RAIDERZ - PMV".
- `down.php?i=pjv9kswhm` on `api.files.fm`, `files.fm` and `fv9-1.files.fm`: `302 → /f/pjv9kswhm`, even with `&PHPSESSID=<page session>`. With the cookie jar from the page: `302 → ?start_p2p_download`.
- `get_torrent.php?file_hash=pjv9kswhm` and `…=ymwhh2449`: `200`, torrents of 15,380 B and 5,411 B.
- `get_torrent.php` for the 6 dead/private folder hashes and `ca3ftrhvs`: `403 "Torrent generation failed"`.
- The webseed from that torrent:
  - `bytes=0-1048575` → `206 bytes 0-1048575/386340367`, valid MP4 `ftyp`.
  - `bytes=300000000-300000099` with no UA → `206`.
  - `bytes=0-2097151` → `206` at about 1.66 MB/s.
- Public `LINK` folder found by web search (not from EroScripts), `a5qtn6gc4xf8g5hw`:
  - The anonymous listing returned 100 files in 2.6 s.
  - `get_torrent.php?folder_hash=…` returned a 100-file torrent with `url-list: https://fv5-6.failiem.lv/`.
  - Base, name and path → `206` for a 1 MB range. The single-file torrent's webseed `206`. A changed date prefix and the `files.fm` host instead of `failiem.lv` also gave `206`.
- Anonymous listing for `wm6p44j45` and the 5 dead folders: `{"error":"not_authenticated_user"}`, each about 2.5 s.
- All partial files were deleted.

Workarounds considered:
- **Torrent `url-list` webseed over plain HTTPS:** works. It needs one anonymous metadata call, gives exact names and sizes, and supports Range. This is the recommended path.
- **`down.php?i=` (documented V1 link):** doesn't work anonymously; it bounces to the page, then to the P2P form.
- **Real BitTorrent client (the `.torrent` via aria2c or WebTorrent):** possible, but nothing is installed (no `aria2c`, `yt-dlp`, `gallery-dl` or `rclone`). The webseed already gives the bytes without P2P.
- **Anonymous `get_file_list_for_upload.php`:** works for public folders only, about 2 s per call and IP-rate-limited. It is useful only to tell "private" from "deleted"; the torrent already gives the file list.
- **Folder ZIP streamer:** possible, but it is a single huge stream with "limits may apply", and the scout prefers per-file downloads.
- **Account or API token (`user_access_token`):** possible with a Files.fm account; out of scope. It is not needed for `LINK` items.

Recommendation for the scout:
- **Detect:** `/https?:\/\/(?:[a-z]{2}\.)?(?:files\.fm|failiem\.lv)\/(u|f)\/([a-z0-9]+)/gi`, and also `/(?:files\.fm|failiem\.lv)\/down\.php\?i=([a-z0-9]+)/`, treated as `f`. Add it in `topicFiles()` next to pixeldrain. Do not send the forum cookie.
- **List:** one call to `get_torrent.php` with `file_hash` (for `f`) or `folder_hash` (for `u`). Parse the bencode (about 15 lines, no dependency) and emit `{name, size, host:'filesfm', url: webseed}`.
  - `403` → the item is gone or private. Add the host as `'Files.fm (gone)'` so `no-video` is explained.
  - For folders, skip non-media files as the scout already does.
- **Download** (branch `file.host === 'filesfm'`): `fetch(file.url)` with no headers, check `200`, pipe it to `.part`. The existing size check applies.
- **Retry or resume:** if a webseed answers 404/403, call `get_torrent.php` again; the storage node in the URL can change. `Range: bytes=<have>-` → `206` if `.part` files are ever kept. Pace metadata calls at ≥1 s.
- **Manual list:** unlikely to be needed. Use it only if `get_torrent` fails with a non-403 status or the webseed keeps failing. `manualPageUrl` = `https://files.fm/f/<file hash>`.
```js
// Minimal bencode decoder (Buffer → objects; strings stay Buffers).
function bdecode(b) { let i = 0; const d = () => { const c = b[i];
  if (c === 0x69) { const e = b.indexOf(0x65, i); const n = Number(b.toString('latin1', i + 1, e)); i = e + 1; return n; }
  if (c === 0x6c) { i++; const a = []; while (b[i] !== 0x65) a.push(d()); i++; return a; }
  if (c === 0x64) { i++; const o = {}; while (b[i] !== 0x65) { const k = d().toString(); o[k] = d(); } i++; return o; }
  const col = b.indexOf(0x3a, i), len = Number(b.toString('latin1', i, col)); i = col + 1 + len; return b.subarray(col + 1, i); };
  return d(); }

// No forum cookie goes to files.fm. kind: 'f' (file) | 'u' (folder).
async function filesFmFiles(kind, hash) {
  const q = kind === 'f' ? `file_hash=${hash}` : `folder_hash=${hash}`;
  const r = await fetch(`https://files.fm/torrent/get_torrent.php?${q}`, { headers: { 'User-Agent': 'Mozilla/5.0' } });
  if (r.status === 403) throw new Error('files.fm: deleted, expired or private');
  if (!r.ok || !/bittorrent/.test(r.headers.get('content-type') || '')) throw new Error(`files.fm: HTTP ${r.status}`);
  const t = bdecode(Buffer.from(await r.arrayBuffer()));
  const seeds = [].concat(t['url-list'] || []).map(String);
  if (!seeds.length) throw new Error('files.fm: torrent has no webseed');
  const root = t.info.name.toString('utf8');
  if (!t.info.files) return [{ name: root, size: t.info.length, host: 'filesfm', url: seeds[0] }];
  const base = seeds[0].endsWith('/') ? seeds[0] : `${seeds[0]}/`;
  return t.info.files.map((f) => { const parts = f.path.map((p) => p.toString('utf8'));
    return { name: parts.at(-1), size: f.length, host: 'filesfm',
      url: base + [root, ...parts].map(encodeURIComponent).join('/') }; });
}
// topicFiles(): for (const m of cooked.matchAll(/https?:\/\/(?:[a-z]{2}\.)?(?:files\.fm|failiem\.lv)\/(u|f)\/([a-z0-9]+)/gi))
//   try { files.push(...await filesFmFiles(m[1], m[2])); } catch { hosts.add('Files.fm (unreadable)'); }
// download(): const resp = await fetch(file.url); if (!resp.ok) throw new Error(`files.fm: HTTP ${resp.status}`);
//   await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp));
```

Manual (browser) instructions if needed:
1. Open `https://files.fm/f/<hash>`, or the folder `https://files.fm/u/<hash>` and click a file.
2. Press **Download**. Files.fm starts its "Peer-Powered Download": keep the tab in the foreground until it finishes. The browser then asks where to save.
   - Or click the small **.torrent** link in that dialog and open it in any torrent client. Its webseed downloads at full speed even without peers.
3. Keep the file name exactly as Files.fm shows it (the scout matches by name and size). Save it into `~/Downloads/for-fs` (or leave it in `~/Downloads`).
4. "Files are deleted — Expiry date is over" or "This folder is set to Private Access" means the post's files are gone. Skip that post.

Sources:
- Files.fm REST API V2 and legacy V1 docs:
  - https://files.fm/api/docs/filesfm_api_v2.md (sections `download_file`, `get_file_list_for_upload.php` "Anonymous link access", "Download folder as ZIP", rate-limit notes)
  - https://files.fm/api/docs/filesfm_api_v2.html
  - https://files.fm/api/docs/openapi.yaml
- Files.fm page script `https://files.fm/js/webtorrent_download_form.js` (`/torrent/get_torrent.php?file_hash=…|folder_hash=…`, `/ajax/webtorrent_download_form.php` `init_client`).
- Files.fm Pro features: https://files.fm/pro
- BitTorrent webseeding, BEP 19: https://www.bittorrent.org/beps/bep_0019.html
- EroScripts forum search `https://discuss.eroscripts.com/search.json?q=files.fm`, and live probes on 2026-11-03.
