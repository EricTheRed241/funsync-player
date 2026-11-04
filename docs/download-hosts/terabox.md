# TeraBox
Verdict: browser-handoff
(Without an account, files can be listed but not downloaded. Downloading needs a logged-in TeraBox session, i.e. the `ndus` cookie of a TeraBox account. If the user ever signs in with their own free account and pastes that cookie, the result becomes auto-with-limits; see the Recommendation.)

Seen on EroScripts:
- Not common, and mostly an old batch. Forum search `terabox` returns **114 posts**: 104 from 2024, 3 from 2025, 1 from 2026 and 6 from 2020–2022. Of these, 97 are in Free Scripts (category 14) and 17 in category 15. Almost all of them are 2024 live-action, JAV and cosplay single-axis posts, which is a poor fit for the user's rules (animated, hand-made multi-axis).
- One uploader (TeraBox account `uk 4399970748760`) owns most of the 2024 shares. A sample of 12 posts showed:
  - Jul–Sep 2024: TeraBox is the only host.
  - Oct–Dec 2024: **pixeldrain + TeraBox** mirrors.
- On the mirrors, the TeraBox file is usually a **.zip/.7z of the same scene** (video plus script), and pixeldrain holds the plain .mp4:
  - 200233: TeraBox `….zip` 206,622,332 B; pixeldrain `….mp4` 206,969,718 B.
  - 205673: TeraBox `Sugardust - Ellen Joe.zip` 354,514,921 B; pixeldrain `.mp4` 354,524,967 B.
- Examples:
  - 200233 `https://terabox.com/s/1cwnHYtcXOZvu2Jh_4315FQ` (zip, plus pixeldrain dPepi171)
  - 205673 `https://terabox.com/s/1X-egAwnKPBA8nVUEExwQGQ`
  - 245753 `https://terabox.com/s/1hKDZgmT0WtnIjYihi4Qvqw` (zip)
  - 181748 `https://www.terabox.com/sharing/link?surl=qYFGvDKlp_SeIrU8rUaFEw` (.7z)
  - 163130 `http://www.1024terabox.com/s/1u2xpEuXRubAX08vi_zTZ6A` (a **folder** holding .mp4 + .funscript)
  - 179270 and 176600 (`1024terabox.com`)
  - 178077 (`1024tera.com`)
  - 327994 `https://1024terabox.com/s/1XdYlNW0oOwJnQEw_Ye06nQ` (a .funscript only; the videos are on MEGA)
  - 293155 `https://terabox.com/s/1yFN_3Zep0E6N-E84xhTtJA` (2.26 GB .mp4; reply also links pan.baidu.com)
  - 6888 (reply #3, `terabox.com/s/1AdgN76ZCN1FVhDl6CDgOUQ`, 3.8 GB `Num.zip`)
- In the candidate list, only 200233 has TeraBox (as `hosts: ["pixeldrain","TeraBox"]`).
- **Detection gap:** `EXTERNAL_HOSTS` in `electron/eroscripts-api.js` only knows `terabox.com`. Links on `1024terabox.com` / `1024tera.com` / `terabox.app` (163130, 176600, 178077, 179270, 327994) aren't recognised as TeraBox. [INFERENCE] Some of the 1,038 `no-video` verdicts may be posts like these.
- URL shapes:
  - `https://[www.]<domain>/s/1<surl>`: the short URL is `1` followed by the surl.
  - `https://[www.]<domain>/sharing/link?surl=<surl>`
  - `/wap/share/filelist?surl=<surl>` and `/web/share/link?surl=<surl>` (seen in third-party tools)
- Mirror domains, all checked live with `/s/1cwnHYtcXOZvu2Jh_4315FQ`. Each one answered 302 and then 200 on the same share:

  | Domain | Redirects to |
  |---|---|
  | terabox.com, www.terabox.com, dubox.com | www.terabox.com |
  | terabox.app, 1024terabox.com, terasharelink.com, teraboxlink.com, teraboxshare.com, 4funbox.com, mirrobox.com, nephobox.com, momerybox.com | www.terabox.app |
  | 1024tera.com, teraboxapp.com, terafileshare.com, freeterabox.com, tibibox.com | www.1024tera.com |
  | gibibox.com | www.gibibox.com |

  `terabox.fun` redirects to terastream.fun and gives a 404 (not a mirror). The HLS segments come from `v4.freeterabox.com`, and downloads from `d.terabox.com` (and `d*.terabox.com` / `d.1024terabox.com` in third-party tools).

How downloads work:
1. **Open the share page.** `GET https://terabox.com/s/1<surl>` returns a 302 to `https://www.<mirror>/sharing/link?surl=<surl>` (HTML). That response sets the cookies `csrfToken, browserid, lang, TSID, PANWEB` (plus `shareRedirectDomain` on 1024terabox).
   - The HTML embeds `jsToken` URL-encoded, as `fn%28%22<192 hex chars>%22%29`. Regex: `/fn%28%22([0-9A-F]+)%22%29/`.
   - The HTML also has `"randsk"` and `"bdstoken":""` (empty when not logged in).
   - A dead or nonexistent share redirects to `/error/404.html`.
2. **Share metadata** (works without an account). Common query: `app_id=250528&web=1&channel=dubox&clienttype=0&jsToken=<t>`.
   - `GET /api/shorturlinfo?<common>&shorturl=1<surl>&root=1` returns `errno 0, shareid, uk, sign, timestamp, randsk, list[]` with `{server_filename, size, isdir, fs_id, path}`.
   - For a dead share, `shorturlinfo` returns `errno 400210` (and `share/list` returns `errno 140`).
3. **List files** (works without an account): `GET /share/list?<common>&shorturl=<surl>&root=1&page=1&num=1000&by=name&order=asc`. For a subfolder, pass `dir=<path>` instead of `root=1` and recurse. Each item has `server_filename, size, isdir, fs_id, path, md5`, but **no `dlink`** without a login.
4. **Get a download link.** These are the requests the page makes:
   - `/share/download` (with `shareid, sign, timestamp, scene, bdstoken` + body `product=share, nozip=0, fid_list, uk, primaryid`) returns `{"errno":2}` without an account, as GET or POST.
   - `POST /api/sharedownload?<common>&sign=<sign>&timestamp=<ts>` with form `encrypt=0&product=share&type=nolimit&uk=<uk>&primaryid=<shareid>&fid_list=[<fs_id>]&extra={"sekey":"<decoded randsk>"}` returns **errno 0** and `list[0].dlink = https://d.terabox.com/file/<md5>?fid=<uk>-250528-<fs_id>&…&expires=1h&…`, even without an account.
5. **Fetch the dlink.** Without a login this gives **HTTP 403** `{"error_code":31045,"error_msg":"user not exists"}`, with or without the share page's cookies.
   - According to the tools listed under Sources, the dlink works when the request carries the cookie `ndus=<value>` from a logged-in account (a free account is enough) and the **same User-Agent** that requested the link. Range requests are supported, and the link is valid for 1 h (`expires=1h`).
   - I could not test this myself (no account allowed).
6. **Video preview stream** (works without an account). `GET /share/streaming?<common>&uk&shareid&type=M3U8_AUTO_480&fid=<fs_id>&sign&timestamp` returns an m3u8.
   - It is only a **30-second preview** (3 segments): `M3U8_AUTO_360` gives 22 s, `M3U8_AUTO_480` and `M3U8_FLV_264_480` give 30 s.
   - `M3U8_AUTO_720` and `M3U8_AUTO_1080` answer `errno 130`.
   - It only works for video files, not the zips most EroScripts shares contain.
7. **The page's own Download button.** Its code (`index.it0l155j.js`: `/share/download`, `/api/invoker/send`, `_launchPC`, "Please download and install the PC client to continue.") hands off to a login or to the TeraBox desktop client. There is no anonymous browser download.

Limits:
- **Downloading needs a login.** Probed in step 5 above. The third-party tools under Sources say the same: "TeraBox lets you LIST a public share without logging in, but it now blocks anonymous download-link generation… you must supply the ndus cookie from ANY free TeraBox account".
- **Free-account speed** (third-party claims only; TeraBox doesn't publish the numbers):
  - roughly 200–800 KB/s and one file at a time for free accounts, versus 15–20 MB/s and 5 files for Premium (cloudslinker).
  - "free download speeds of 1.5–6.5 MB/s" (cloudslinker guide).
  - At 500 KB/s, a 350 MB zip takes about 12 min.
- **Link expiry:** each dlink lasts 1 h (`expires=1h` in the URL).
- **Other share features:** password-protected shares need `/share/verify` (pwd), which sets the `BOXCLND` cookie used as the `sekey`. Some shares are paid ("linkpay", `purchase_*` fields).
- **Page scripts:** heavy ad scripts on the page (adsterra on 1024terabox.com).
- **Rate:** I saw no rate limit in about 30 anonymous API calls on 2026-11-03.

Probe results (2026-11-03, Node 26 `fetch`, no account; forum read with the session cookie, about 45 forum requests):
- Share page `/s/1cwnHYtcXOZvu2Jh_4315FQ` (post 200233): 302, then 200. jsToken (192 hex) extracted.
- `shorturlinfo` errno 0: one file, `Candy Love - … cum inside.zip`, 206,622,332 B, `fs_id 694128947516654`.
- `share/list` errno 0, same file, no dlink.
- Seven more shares listed fine without an account:
  - 181748: `MIDV-791 part1 edit.7z`, 360 MB
  - 245753: zip, 370 MB
  - 205673: zip, 354 MB
  - 327994: `.funscript`, 790 KB
  - 293155: `.mp4`, 2.26 GB
  - 6888: `Num.zip`, 3.8 GB
  - 163130: a folder, listed recursively with `dir=`: `…1화_기본.mp4` 887,766,259 B + `.funscript` 44,907 B
- `/share/download` gave errno 2 (3 variants).
- `/api/sharedownload` gave errno 0 and a dlink. **The dlink, with a 1 MB Range request, answered 403 / 31045 "user not exists"**, both with and without the anonymous cookies. No bytes were received.
- `/share/streaming` on 293155: a 30-second m3u8 at 360p/480p; segment 0 answered **206** to a 1 MB Range request (an MPEG-TS file starting with byte `0x47`, deleted afterwards). 720p/1080p gave errno 130. On 200233 (zip) it gave errno 31304.
- Bogus share: `/error/404.html`, `shorturlinfo` errno 400210.
- pixeldrain mirrors: `api/file/dPepi171/info` gives an .mp4, 206,969,718 B; `cRgqqnJs` gives an .mp4, 354,524,967 B.
- Tools: yt-dlp, gallery-dl, rclone and aria2c are **not installed**; `unar` is.

Workarounds considered:
- **The share's own `/api/sharedownload` without an account:** doesn't work. It returns a dlink, but the CDN refuses it (31045) without a logged-in user.
- **Preview streaming (`/share/streaming`) + ffmpeg:** doesn't work. It gives a 30-second 480p preview only, and nothing for zips.
- **The user's own free TeraBox account `ndus` cookie in config:** would work. The documented flow is: the same `/api/sharedownload` (or `/share/download`) with `Cookie: ndus=…`, then a dlink GET with the same UA and Range. alist/OpenList, `seiya-dev/terabox-node`, `mugarrajohnson/terabox-direct-links` and the herobenhero docs all do this.
  - It needs the user to sign in to TeraBox (Google/Apple/email). Per the rules I didn't create an account, so this path is **untested**.
  - The speed is throttled (see Limits).
  - The cookie is long-lived, but it is account credentials: store it like the forum cookie and never log it.
  - Note it as "possible with the user's own TeraBox login".
- **Save to own TeraBox, then download** (`/share/transfer` into the account): same login requirement, no gain for the scout.
- **Third-party resolvers** (teraboxdownloader.org, TeraPlay, TeraSaver, Telegram bots, Flask "TeraboxDownloader" clones): they proxy through *their* accounts or cookies. Shady and against the rules; not used.
- **yt-dlp:** no extractor. Issue #5484 was closed "not planned".
- **gallery-dl:** no extractor.
- **rclone:** no upstream backend. There is an open request #7496 and an unmerged PR #8508, plus forks (`mill-master/rclone-terabox`, `rclone-extra`), all cookie or login based.
- **alist / OpenList `terabox` driver:** needs the account cookie. It only works on the user's own drive, not on others' shares.
- **Headless browser:** wouldn't help. The block is the missing login, not a CAPTCHA.
- **Use the post's other mirror instead:** works, and it's the best option. Most 2024 TeraBox posts also link pixeldrain with the same video, unpacked. The scout already handles pixeldrain, including the manual list when pixeldrain asks for a CAPTCHA.

Recommendation for the scout:
- **Detection.** Add every mirror to `EXTERNAL_HOSTS` (all labelled `'TeraBox'`): terabox.com, terabox.app, 1024terabox.com, 1024tera.com, teraboxapp.com, terasharelink.com, terafileshare.com, teraboxlink.com, teraboxshare.com, 4funbox.com, mirrobox.com, nephobox.com, momerybox.com, freeterabox.com, tibibox.com, gibibox.com, dubox.com.
  - URL regex: `/https?:\/\/(?:www\.)?((?:1024)?tera(?:box(?:app|link|share)?|sharelink|fileshare)\.(?:com|app)|1024tera\.com|4funbox\.com|mirrobox\.com|nephobox\.com|momerybox\.com|freeterabox\.com|tibibox\.com|gibibox\.com|dubox\.com)\/(?:s\/1([\w-]+)|(?:sharing|web\/share|wap\/share)\/(?:link|filelist)\?surl=([\w-]+))/g`. The surl is `m[2] || m[3]`.
- **Listing** (no account needed). In `topicFiles`, list TeraBox shares like pixeldrain:
  - one share-page GET (for jsToken + cookies), then `share/list` (recurse into `isdir==='1'` with `dir=`);
  - push `{name: server_filename, size: +size, host: 'terabox', url: 'https://www.terabox.com/sharing/link?surl=<surl>', fsId}`.
  - Zips go through the existing `packed` path (PR #73).
- **Prefer the other mirror.** Push TeraBox files *after* pixeldrain/MEGA, and drop a TeraBox archive when a non-TeraBox file in the same post has size within ±1% (or the same base name). This covers the 2024 pixeldrain + TeraBox pairs.
- **Without an account: manual list.**
  - Set `MANUAL_HOSTS = new Set(['pixeldrain','terabox'])`.
  - In `manualPageUrl`, return the `https://www.terabox.com/sharing/link?surl=…` page for `host==='terabox'`.
  - In `downloadFile`, throw `terabox: needs a TeraBox login (download it by hand: --manual-list)` unless the file was already picked up from `~/Downloads/for-fs` by name/size. That existing matching works because the share listing gives the exact name and byte size.
- **Optional, with the user's own `ndus`** (e.g. `settings.terabox.ndus`, never logged): per file, `POST /api/sharedownload` (below), then GET the dlink with the same UA + `Cookie: ndus=…`, following redirects.
  - Resume: `Range: bytes=<partSize>-`, writing to the `.part`.
  - A dlink expires after 1 h, so on a 403 or a stall, request a fresh dlink and resume from the `.part` size.
  - On `errno` ≠ 0, or `31045` / `-6` / `400141` (login expired, verification wanted): fall back to the manual list.
  - Expect free-account throttling.
- **Error mapping.** `errno 400210`/`140`, or a redirect to `/error/404.html`: the share is gone, mark it `TeraBox (dead)`. `/share/verify` needed (password): manual.

```js
const UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36';
async function teraboxShare(surl, ndus) {               // → {origin, base, info, cookie}
  const jar = new Map(ndus ? [['ndus', ndus]] : []);
  const ck = () => [...jar].map(([k, v]) => `${k}=${v}`).join('; ');
  let u = `https://www.terabox.com/sharing/link?surl=${surl}`, r;
  for (let i = 0; i < 6; i++) {                          // follow mirror redirects by hand to keep cookies
    r = await fetch(u, { redirect: 'manual', headers: { 'User-Agent': UA, Cookie: ck() } });
    for (const c of r.headers.getSetCookie()) { const [kv] = c.split(';'); const j = kv.indexOf('='); jar.set(kv.slice(0, j), kv.slice(j + 1)); }
    if (r.status < 300 || r.status >= 400) break; u = new URL(r.headers.get('location'), u).href;
  }
  if (u.includes('/error/')) throw new Error('terabox: share gone');
  const jsToken = (await r.text()).match(/fn%28%22([0-9A-F]+)%22%29/)?.[1];
  const origin = new URL(u).origin, base = `app_id=250528&web=1&channel=dubox&clienttype=0&jsToken=${jsToken}`;
  const get = (p) => fetch(`${origin}${p}`, { headers: { 'User-Agent': UA, Cookie: ck(), Referer: u } }).then((x) => x.json());
  const info = await get(`/api/shorturlinfo?${base}&shorturl=1${surl}&root=1`);
  if (info.errno) throw new Error(`terabox: errno ${info.errno}`);
  const files = [];
  const walk = async (dir) => {
    const j = await get(`/share/list?${base}&shorturl=${surl}&${dir ? `dir=${encodeURIComponent(dir)}` : 'root=1'}&page=1&num=1000&by=name&order=asc`);
    for (const f of j.list || []) f.isdir === '1' ? await walk(f.path) : files.push({ name: f.server_filename, size: +f.size, fsId: f.fs_id });
  };
  await walk(null);
  return { origin, base, info, files, cookie: ck, referer: u };
}
async function teraboxDlink(s, fsId) {                   // needs ndus in the jar, else the CDN answers 31045
  const body = new URLSearchParams({ encrypt: '0', product: 'share', type: 'nolimit', uk: s.info.uk, primaryid: String(s.info.shareid),
    fid_list: `[${fsId}]`, extra: JSON.stringify({ sekey: decodeURIComponent(s.info.randsk || '') }) });
  const j = await fetch(`${s.origin}/api/sharedownload?${s.base}&sign=${encodeURIComponent(s.info.sign)}&timestamp=${s.info.timestamp}`,
    { method: 'POST', body, headers: { 'User-Agent': UA, Cookie: s.cookie(), Referer: s.referer, 'Content-Type': 'application/x-www-form-urlencoded' } }).then((x) => x.json());
  if (j.errno || !j.list?.[0]?.dlink) throw new Error(`terabox: sharedownload errno ${j.errno}`);
  return j.list[0].dlink;                                // GET with same UA + Cookie ndus + Range: bytes=<part>-; valid 1 h
}
```

Manual (browser) instructions if needed:
1. Before using TeraBox, check whether the same post has a pixeldrain or MEGA link: the 2024 posts nearly always do, and that file is the plain .mp4. If it does, use that link instead.
2. Open the TeraBox link from `download-for-fs.txt` (`npm run manual:open`). Any mirror domain (1024terabox.com, terabox.app, 1024tera.com …) is the same share.
3. TeraBox needs you to be **signed in** to download. Use your own free account (Google/Apple/email sign-in), then click **Download** on the file. If the page offers "Download PC client" / "Save to TeraBox" instead, choose download in the browser if it's offered. Otherwise, save the file to your TeraBox and download it from your own file list.
4. Close the ad pop-ups. Don't install anything the page offers except the official TeraBox app, and only if you want it.
5. Keep the file **exactly as named on the page** (e.g. `Candy Love - wanna remove condom... oke, I'm a good whore, I'll let you cum inside.zip`) and don't unzip it. Save it to `~/Downloads/for-fs/`.
6. The next `--fetch` finds it by name and size, unpacks .zip/.7z itself (unar) and removes it from Downloads.
7. For a folder share, download each file inside it (video + .funscript) with its own name into the same folder.

Sources:
- Live probes above (2026-11-03): terabox.com share API, `s5.teraboxcdn.com/fe-static/fe-v5-web-share/js/{index-*,share.*,shareLink.*}.js`
- https://raw.githubusercontent.com/mugarrajohnson/terabox-direct-links/main/terabox_folder_downloader.py ("blocks anonymous download-link generation … ndus cookie from ANY free TeraBox account")
- https://herobenhero.github.io/1024TeraBox-REST-API/api/download.html (dlink needs `ndus` cookie + same User-Agent; Range supported)
- https://github.com/seiya-dev/terabox-node , https://github.com/dangkhoa2016/terabox-api-ruby (NDUS-cookie clients)
- https://github.com/AlistGo/alist/tree/main/drivers/terabox , https://github.com/OpenListTeam/OpenList/issues/1020 ("user not exists")
- https://github.com/yt-dlp/yt-dlp/issues/5484 (TeraBox support: closed, not planned)
- https://github.com/rclone/rclone/issues/7496 , https://github.com/rclone/rclone/pull/8508 , https://github.com/mill-master/rclone-terabox
- https://www.cloudslinker.com/support-clouds/terabox , https://www.cloudslinker.com/guides/download-terabox-shared-link-files (free-tier speed claims, login required)
- Forum: https://discuss.eroscripts.com/t/200233 , /t/163130 , /t/181748 , /t/205673 , /t/245753 , /t/293155 , /t/327994 , /t/178077 , /t/6888 ; `search.json?q=terabox` (114 posts)
