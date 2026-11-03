# OneDrive
Verdict: auto-with-limits

Seen on EroScripts: Very rare. Forum full-text search on 2026-11-03: `1drv.ms` 8 posts, `onedrive` 15 posts (mostly chat about software), `sharepoint.com` 0 (one SharePoint link turned up via "onedrive").
- Script topics with a OneDrive link: 105930, 225360, 225365 (the latter two share one folder), 197570#4, 313748, 50543, 241933#4 (SharePoint). Non-script: 48731#2.
- None are in the scout's candidate list (0 OneDrive hosts) or in its state. The 20-post sample of `no-video` posts found 0 OneDrive/Dropbox links.

Link shapes seen:
- Legacy personal: `https://1drv.ms/{u|v|f}/s!<base64ish>?e=<6>`. `u` = file, `v` = video, `f` = folder.
- Post-2024 personal on SharePoint backend: `https://1drv.ms/v/c/<16 hex cid>/<Ec…|IQ…>?e=<6>`.
- Expanded redirect targets that some posts paste directly:
  - `https://onedrive.live.com/:u:/g/personal/<CID>/s!…?resid=<CID>!<n>&e=…&migratedtospo=true&redeem=<base64url of the 1drv.ms link>`
  - `https://photos.onedrive.com/share/<CID>!<n>?cid=…&resId=…&ithint=video&e=…&migratedtospo=true&redeem=<base64url>` (seen in 50543)
- Business: `https://<tenant>-my.sharepoint.com/personal/<user>/_layouts/15/onedrive.aspx?id=<path>&ga=1` (241933). Not seen but standard: `https://<tenant>-my.sharepoint.com/:v:/g/personal/<user>/<token>?e=…`.

How downloads work (consumer OneDrive):
- A 1drv.ms link only redirects to the JS web app (`onedrive.live.com/?…` or `photos.onedrive.com` → 200 HTML). There is no direct file URL in the HTML.
- The web app itself gets an anonymous "Badger" token, then calls the OneDrive v2 shares API on the SharePoint-hosted personal endpoint. Steps:
  1. `POST https://api-badgerp.svc.ms/v1.0/token` with body `{"appId":"5cbed6ac-a083-4e14-b191-b4ba07653de2"}` (`Content-Type: application/json`). Response: `{authScheme:"badger", token, expiryTimeUtc}`; the token is valid 7 days.
  2. Build the share id: `"u!" + base64url(shareUrl)` (strip `=`, `/`→`_`, `+`→`-`). Use the original `1drv.ms/...` URL. For `onedrive.live.com` / `photos.onedrive.com` URLs, base64url-decode the `redeem=` param to get it back.
  3. `GET https://my.microsoftpersonalcontent.com/_api/v2.0/shares/<shareId>/driveitem` with headers `Authorization: Badger <token>` and `Prefer: autoredeem`. Response JSON fields: `name`, `size`, `file.mimeType` or `folder.childCount`, `parentReference.driveId`, `id`, `@content.downloadUrl`, plus `createdBy` with the owner's email, so don't log the whole object.
  4. For a folder: `GET …/shares/<shareId>/driveitem/children?$top=200`, same headers. Each child has `name`, `size`, `file`, `@content.downloadUrl`. Paging is via `@odata.nextLink`; call it with the same headers.
  5. Download with a plain `GET @content.downloadUrl` (`https://my.microsoftpersonalcontent.com/personal/<cid>/_layouts/15/download.aspx?UniqueId=…&Translate=false&tempauth=…&ApiVersion=2.0`). It needs **no auth header**, supports `Range` (206), and names the file by `name` from step 3/4. `GET …/driveitem/content` with the Badger headers also 302s to the same download URL, for files only.
- Old method: `https://api.onedrive.com/v1.0/shares/u!…/root[/content]` without auth now answers **401 `unauthenticated`** for SPO-migrated links, and 403 for dead ones. Microsoft Q&A reports it broke in late 2024. Don't use it.
- Business (SharePoint) links: only "Anyone with the link" links work anonymously. Append `download=1` to a `/:x:/g/personal/…` share URL; this gives a 302 to `download.aspx?…&tempauth=…` [INFERENCE: commonly documented behaviour, not live-probed, since the only forum example's tenant no longer resolves]. Org-only links and `_layouts/15/onedrive.aspx?id=` folder views need a Microsoft sign-in, so use the manual list.

Limits:
- **`tempauth` expiry**: the `@content.downloadUrl` is valid **~1 h**. Measured: tempauth `exp` was 1 h after issue. For a long download or resume after an hour, call step 3/4 again to get a fresh URL.
- **Badger token**: 7 days (`expiryTimeUtc` was issue time + 7 d). Cache it per run; fetch a new one on 401.
- **Throttling**: OneDrive/SharePoint answer 429/503 with `Retry-After`; honour it (Microsoft "avoid getting throttled" docs).
- **Bandwidth**: Microsoft does not publish a per-link download cap for consumer links. Heavy sharing can still get an owner's links blocked [INFERENCE].
- **Dead links**: 403 `accessDenied` "The sharing link no longer exists…" and 404 `itemNotFound` "Requested sharing link could not be found". 2 of 7 forum links were dead. Throwaway business tenants (`*.onmicrosoft.com` dev tenants) vanish entirely; 241933's host is NXDOMAIN.
- **Other**: no CAPTCHA and no region block seen. Folder ZIP ("Download" on a multi-selection) is a separate web-only zip service; not needed and not probed.
- The Badger flow is undocumented (it is what onedrive.live.com does in the browser) and may change; failure = manual list.

Probe results (2026-11-03; ≤1 MB Range reads into memory, nothing kept):
- Redirects (curl -L): every `1drv.ms` link → 301 to `onedrive.live.com/:x:/g/personal/…` or `photos.onedrive.com/share/…` → 302 → **200 HTML** web app, including dead links.
- Badger token `POST`: **200**, `authScheme: badger`, expiry +7 d.
- 105930 `1drv.ms/v/s!AtNi…?e=F7NfYX`:
  - driveitem **200**: `D1gital Crush - World PMV Games 2023 (4k).mkv`, 3,911,199,829 B.
  - downloadUrl `Range 0-1048575` → **206** `bytes 0-1048575/3911199829`, Matroska magic.
  - `/driveitem/content` → **302** to download.aspx.
- 197570 `1drv.ms/v/c/0fd15406a6c39fb5/EcEh…?e=ICV07q`: **200** `HMV-Segs with my Legends.mp4`, 425,993,082 B; Range → **206**, `ftypisom`.
- 313748 `1drv.ms/v/c/2baf4e7b08836eb7/IQAv…?e=ppffc5`: **200** `kevin the octopus.mp4`, 522,102,638 B.
  - Range → **206**. Mid-file `bytes=300000000-300999999` → **206**, so resume works.
  - The request also worked without `Origin`/`User-Agent` headers. Without the Badger token: **401**.
- 225360/225365 folder `1drv.ms/f/s!AotX…?e=HLlAgh`: driveitem **200** `HMV WORK` (folder, 5,311,910,754 B, 36 children, among them the posts' `YOMI.mp4` and `Mind Breaker.mp4`).
  - `children?$top=3` → 3 items + `@odata.nextLink`.
  - First child's downloadUrl Range → **206** `/135962022`, `ftypmp42`.
  - `/driveitem/content` on a folder → 404.
- 48731 `1drv.ms/u/s!Ah0Q…`: Badger and legacy API both **403** `accessDenied` (link removed).
- 50543 `photos.onedrive.com/share/…!24085…&redeem=<b64>` → decoded `1drv.ms/v/s!AtNiVMJwYqCSgbwV…`: **404** `itemNotFound`. Passing the photos URL itself as the share id gives 403. Always decode `redeem`.
- Legacy `api.onedrive.com/v1.0/shares/u!…/root`: **401** for all live links.
- 241933 SharePoint `i8u9-my.sharepoint.com`: DNS does not resolve (tenant deleted).
- Tools: no `yt-dlp`, `rclone` or `aria2c` installed; Node `fetch` is enough.

Workarounds considered:
- `api.onedrive.com` `u!` shares API (anonymous): **doesn't work** anymore (401), per Microsoft Q&A.
- Microsoft Graph `/shares/u!…/driveItem`: needs an OAuth token from a Microsoft account / app registration. That's "possible with an Azure app + account", which is out of scope.
- Badger anonymous token + `my.microsoftpersonalcontent.com/_api/v2.0/shares`: **works** for every live consumer link (file, video, folder, old `s!` and new `c/` formats). These are the same calls the OneDrive web viewer makes.
- `onedrive.live.com/download?cid=…&resid=…&authkey=…` (old embed trick): forum links carry no `authkey`, and links are SPO-migrated. Not applicable.
- SharePoint `download=1`: works only for "Anyone" links (not probed; no live example). Org-only links → manual.
- Browser hand-off: always works for live links and is the fallback.

Recommendation for the scout:
- **Detect** (hrefs, after `&amp;`→`&`):
  `/https?:\/\/(?:1drv\.ms\/[a-z]\/(?:s!|c\/)[^\s"<]+|(?:onedrive\.live\.com|photos\.onedrive\.com)\/[^\s"<]*redeem=[^\s"<&]+[^\s"<]*|[a-z0-9-]+-my\.sharepoint\.com\/[^\s"<]+)/gi`.
  Add `photos.onedrive.com`, `sharepoint.com` and `my.microsoftpersonalcontent.com` → 'OneDrive' to `EXTERNAL_HOSTS`; only `1drv.ms` and `onedrive.live.com` are there today.
- **List**: `onedriveFiles(url)` returns `{name,size,host:'onedrive',url:<original share URL>,id:<item id>}`.
  - Store the *share URL* (plus child item name), not the 1-hour downloadUrl.
  - Recurse into child folders only if needed; nested folders were not seen or probed.
- **Download**: in `downloadFile`, for `file.host === 'onedrive'`, resolve a fresh `@content.downloadUrl`: re-run driveitem, or children and match by name. Then `GET` it with `Range: bytes=<have>-` (206).
  - On 401 from download.aspx (tempauth expired) re-resolve once.
  - On 429/503 sleep `Retry-After`.
  - On 403/404 from driveitem, or any non-JSON → manual list.
- **SharePoint** links: try `url + (url.includes('?') ? '&' : '?') + 'download=1'`. If the final response is `text/html` (login page), send it to the manual list.

```js
const OD_API = 'https://my.microsoftpersonalcontent.com/_api/v2.0/shares';
let badger = null;                                                     // { token, exp }
async function badgerHeaders() {
  if (!badger || Date.now() > badger.exp - 3600e3) {
    const r = await fetch('https://api-badgerp.svc.ms/v1.0/token', { method: 'POST',
      headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ appId: '5cbed6ac-a083-4e14-b191-b4ba07653de2' }) });
    const j = await r.json(); badger = { token: j.token, exp: Date.parse(j.expiryTimeUtc) };
  }
  return { Authorization: `Badger ${badger.token}`, Prefer: 'autoredeem' };
}
function shareUrlOf(link) {                                            // photos/onedrive.live URLs carry the 1drv.ms link in redeem=
  const redeem = new URL(link).searchParams.get('redeem');
  return redeem ? Buffer.from(redeem, 'base64url').toString() : link;
}
const shareId = (u) => 'u!' + Buffer.from(u).toString('base64url');   // base64url already drops '=' and maps +/ → -_
async function odJson(url) {
  const r = await fetch(url, { headers: await badgerHeaders() });
  if (r.status === 429 || r.status === 503) throw Object.assign(new Error('onedrive throttled'), { retryAfter: Number(r.headers.get('retry-after')) || 60 });
  if (!r.ok) throw new Error(`onedrive: HTTP ${r.status} (dead link → manual list)`);
  return r.json();
}
async function onedriveFiles(link) {                                   // → [{ name, size, host, url, downloadUrl }]
  const share = shareUrlOf(link);
  const base = `${OD_API}/${shareId(share)}/driveitem`;
  const item = await odJson(base);
  const pick = (x) => ({ name: x.name, size: x.size, host: 'onedrive', url: share, downloadUrl: x['@content.downloadUrl'] });
  if (!item.folder) return [pick(item)];
  const out = [];
  for (let next = `${base}/children?$top=200`; next; ) {
    const page = await odJson(next);
    out.push(...page.value.filter((x) => x.file).map(pick));
    next = page['@odata.nextLink'];
  }
  return out;
}
// downloadFile: downloadUrl lives ~1 h → re-run onedriveFiles(file.url), match by name, then
// GET downloadUrl with Range: bytes=<have>- (expect 206; no auth header needed).
```

Manual (browser) instructions if needed:
- **Single file or video**: open the post's OneDrive link. If it opens in the photo/video viewer, click **⋯ → Download** (or the **Download** button in the top bar). Keep the original filename shown in the viewer (e.g. `kevin the octopus.mp4`) and save it to `~/Downloads/for-fs`.
- **Folder**: open the link, tick only the wanted video and click **Download**; that saves the single file. Selecting several files gives `OneDrive_<n>_<date>.zip`; avoid that or unzip it into `~/Downloads/for-fs`.
- **SharePoint/business link that asks you to sign in**: it is not public, so skip it.
- **"This item might not exist or is no longer available" / "link has expired"**: the file is gone.

Sources:
- Gist "OneDrive simple downloader" (Badger token appId, `Prefer: autoredeem`, `my.microsoftpersonalcontent.com/_api/v2.0/shares/u!…/driveitem`): https://gist.github.com/beppler/b7c16597ce71cf2a60fb11eb3b8294a0
- Microsoft Q&A, legacy `api.onedrive.com/v1.0/shares/u!…` direct-link conversion broke: https://learn.microsoft.com/en-us/answers/questions/2130053/issues-with-onedrive-direct-link-conversion
- Microsoft Q&A, shareId no longer usable anonymously: https://learn.microsoft.com/en-us/answers/questions/2183980/the-shareid-obtained-from-createlink-can-no-longer
- Graph shares API and `u!` base64url share-id encoding: https://learn.microsoft.com/en-us/graph/api/shares-get?view=graph-rest-1.0
- Throttling (429/503 + Retry-After): https://learn.microsoft.com/en-us/sharepoint/dev/general-development/how-to-avoid-getting-throttled-or-blocked-in-sharepoint-online
- Live probes listed above (2026-11-03); forum searches via `https://discuss.eroscripts.com/search.json?q=…` and `/t/<id>.json`.
