# Video streaming sites (rule34video, hanime1, iwara, hmvmania, spankbang, newgrounds, pornhub, xvideos, eporner)
Verdict: auto-with-limits (rule34video, hmvmania: plain Node `fetch`; iwara, hanime1, spankbang, pornhub, xvideos, eporner: through yt-dlp with curl_cffi impersonation) · browser-handoff (newgrounds, private iwara videos) · not-feasible (Patreon / Fanbox / Fantia / Gumroad paywalls)

Status (2026-11-09): **rule34video is implemented** in `npm run eroscripts:scout` (`scripts/stream-hosts.mjs`, `planWithStreams` in `scripts/eroscripts-scout-lib.mjs`; see the README's scout section). The other sites aren't.

Status (2026-11-09): **vrbangers.com (paid member downloads) is implemented** too: `npm run vrbangers:scout` / `scripts/vrbangers-host.mjs` (public scene page at scan time, signed links through headless Chrome at download time; see the README's scout section).

Seen on EroScripts:
A random sample of 50 of the 1,038 `no-video` posts (seed 7, 49 readable, 1 deleted → 404), fetched 2026-11-03. Every one of the 49 had forum script attachments. They had passed content screening (animated, sex, exclusions) and then failed `planScenes`, because their video sits on a site the scout can't read.

| Site | Share of no-video posts (n=49) | ≈ of 1,038 | yt-dlp works | Login needed | Notes |
|---|---|---|---|---|---|
| rule34video.com | 19 (39%) | ~400 | **n**: `Rule34Video` extractor in 2026.10.26 fails with "No video formats found" (page markup changed). Plain fetch works | no | ddos-guard, but Node `fetch` gets 200. Formats 360p–1080p plus **4K (2160p60)** on 4 of 8 checked. Duration in JSON-LD. 13 of 19 posts are multi-axis. Some uploads are 15–33 s teasers (the scripts are that short too) |
| hanime1.me | 10 (20%) | ~210 | y (generic extractor + `--impersonate chrome`) | no | Cloudflare: Node fetch → 403. Mostly **2D hentai** (the user's −3 rank). 720p/1080p mp4 on `vdownload-N.hembed.com`, signed `token`+`expires`, needs `Referer: https://hanime1.me/` |
| iwara.tv | 6 (12%) | ~125 | y with `--impersonate chrome` (n without: "Failed to parse JSON", a Cloudflare challenge) | **no for 6/6 tested** (R-18 "ecchi" included). Private and "may need login" videos need an iwara account | `Source` = original upload (720p–1080p seen). yt-dlp gives no duration, ffprobe on the URL does |
| hmvmania.com | 2 (4%) | ~40 | n (no extractor; generic doesn't find it) | no | WordPress; mp4 URL is in `data-video-source="…"`. Node fetch works; Range OK. One of 2 links was 404 (page gone) |
| spankbang.com | 1 (2%) + 1 more found by search | ~20 | y with `--impersonate chrome` (n without: HTTP 403) | no | Cloudflare. Direct mp4 `vdownload-NN.sb-cd.com/…-1080p.mp4?…` (CDN77, Range OK) plus HLS. Duration `NA`; use ffprobe |
| newgrounds.com | 1 (2%) | ~20 | n (HTTP 403 "NG Guard", even with impersonation) | yes for adult content [INFERENCE] | browser-handoff |
| pornhub.com | 0 | ~0 | y | no (from this Mac) | 49/50 search hits for `pornhub.com/view_video` were never judged: they're live-action posts the scout filters out before links are read |
| xvideos.com | 0 (3 found by search) | <10 | y | no | Same: mostly live action |
| eporner.com | 0 (2 found by search) | <10 | y | no | Same; has 2160p and AV1 variants |
| Patreon / Fanbox / Fantia / Gumroad / Payhip / DLsite / Unifans (paywall or creator page only, no streaming link) | 9 (18%) | ~190 | n/a | paid | not-feasible |
| file host linked but still no-video (MEGA folder, pixeldrain, gofile, `pixeldrain /api/file/…` shape) | 3 (6%) | ~60 | — | — | Belongs to the file-host reports: the scout only reads `pixeldrain.com/(u|l)/` and MEGA, and gofile isn't fetched |
| nothing linked | 1 (2%) | ~20 | — | — | — |

**36 of 49 no-video posts (73%, 95% CI about ±12 pp) link at least one streaming site the scout could read: about 760 posts.** 28 of those 36 carry the `multi-axis` tag. 3 posts link two sites (rule34video + spankbang/iwara/hanime1). VR is rare (1 of 36).
Link shapes:
- `https://rule34video.com/video/<id>/<slug>/` (also `/members/<n>/` and `/search/<q>/` profile/search links, which aren't videos)
- `https://hanime1.me/watch?v=<n>`
- `https://www.iwara.tv/video/<id 14–17 alnum>/<slug>` (also `/profile/<user>`)
- `https://hmvmania.com/video/<slug>/#/?playlistId=0&videoId=0`
- `https://spankbang.com/<id>/video/<slug>`
- `https://www.newgrounds.com/portal/view/<n>`
- `https://www.pornhub.com/view_video.php?viewkey=<key>`
- `https://www.xvideos.com/video.<id>/<slug>`
- `https://(de.)eporner.com/video-<id>/<slug>/`

Examples by site:
- rule34video: 111493, 146899, 149432, 152443, 152471, 186978, 231090, 235131, 243813, 253640, 255592, 289736, 290099, 309309, 323708, 326035, 340132, 341001, 342901
- hanime1: 240189, 290099, 292949, 306033, 322664, 331004, 338114, 338260, 339563, 343687
- iwara: 152471, 225465, 260952, 283242, 288768, 307617
- hmvmania: 284742, 303731
- spankbang: 152443, 38756
- newgrounds: 105941
- xvideos: 318442, 318252, 317034
- eporner: 82316, 82533
- Patreon-only: e.g. the `patreon.com/c/wata…` posts

How downloads work:
- **rule34video (KVS CMS):** `GET /video/<id>/<slug>/` (any slug works) returns HTML whose player flashvars hold:
  - `video_url: '…_360.mp4/?v-acctoken=…'` and `video_alt_url[2-4]: '…/get_file/<srv>/<hash>/<id-1000>/<id>/<id>_<720p|1080p|2160p>.mp4/?v-acctoken=…'`, each with `video_alt_urlN_text: '720p'|'1080p'|'4k'`. The same URLs, plus `&download=true&download_filename=…`, sit in `<a class="tag_item tag_item_download vp-sheet__item">` menu items.
  - `GET get_file/…` answers 302 (server ddos-guard) to `https://<region>-cdnNN-prem.boomio-cdn.com/remote_control.php?file=…&acctoken=…` and then 206 with Range support.
  - The acctoken's first field is the bitrate cap in kbps (22876 for the 23 Mb/s 2160p file), not an account.
  - Duration: JSON-LD `"duration": "PT0H3M21S"`. No cookies needed, `Referer: https://rule34video.com/` is sent.
  - Why yt-dlp fails: its regex `<a[^>]+href="([^"]+download=true[^"]+)".*>(\S+) ([^<]+)p</a>` expects `MP4 1080p</a>` right after the tag, but the link text is now wrapped in `<svg>`/`<span>`.
- **hmvmania:** `GET /video/<slug>/`, then `data-video-source="https://hmvmania.com/wp-content/uploads/<y>/<m>/<file>.mp4"` (Cloudflare-cached static file, Range 206).
- **iwara:** what yt-dlp's `iwara` extractor does:
  1. `GET https://api.iwara.tv/video/<id>` (JSON, optional `Authorization: Bearer <token>` after `POST user/login`).
  2. `fileUrl` (`https://files.iwara.tv/file/<uuid>?expires=…&hash=…`) is fetched with header `X-Version: sha1("<last path seg>_<expires>_<constant>")` and gives a list of qualities (`Source`, `540`, `360`, `preview`).
  3. `https:` + `src.download`; the CDN host is e.g. `hime.iwara.tv`, which accepts GET with Range (HEAD → 405).
  - Both api.iwara.tv and www answer **403 `cf-mitigated: challenge`** to curl/Node fetch, so a Chrome TLS fingerprint (yt-dlp `--impersonate chrome` = curl_cffi) is required.
- **hanime1:** Cloudflare page (`/watch?v=<n>`, needs impersonation). `<source>` tags hold `https://vdownload-N.hembed.com/<n>-<720p|480p|1080p>.mp4?token=…&expires=<unix>`. The CDN needs `Referer: https://hanime1.me/` (403 without it, 206 with it).
- **spankbang:** Cloudflare page (impersonation). The extractor posts to the stream JSON and returns direct mp4s per height on `vdownload-NN.sb-cd.com` (signed query, CDN77, Range 206) plus HLS.
- **pornhub / xvideos / eporner:** standard yt-dlp extractors, no impersonation needed. Pornhub gives HLS + mp4 ≤1080p30; eporner gives h264 + AV1 up to 2160p.

Limits:
- **rule34video:** answers 429 above ~3 requests/s (seen 2026-10-27 during the poster run, see the funsync-player skill). Stream bitrate is capped per token (22.9 Mb/s on 4K). Whether acctoken URLs expire wasn't measured, so read the page right before each download [INFERENCE: KVS links are session/time scoped].
- **iwara:** Cloudflare challenge on every endpoint without a browser-grade TLS fingerprint. `errors.privateVideo` and "may need login" need an iwara account ("possible with an iwara account": yt-dlp `--username/--password` or `.netrc` machine `iwara`; not tested, no account created).
- **hanime1:** Cloudflare. Signed links carry `expires` (unix, ≈ +12 h from the probe) and need the Referer.
- **spankbang:** Cloudflare (plain 403). Signed CDN links.
- **newgrounds:** "NG Guard" anti-bot page (403) for curl, Node and yt-dlp with impersonation; adult works need a logged-in account with the adult filter on [INFERENCE from the site's policy, not probed].
- **pornhub:** blocked by age-verification laws in some regions (several US states, France…); worked from this Mac on 2026-11-03.
- **Impersonation:** curl_cffi passes Cloudflare today; Cloudflare can tighten at any time. Then the fallback is the manual list.

Probe results (2026-11-03):
- **Tools.** yt-dlp is **not installed** (`which yt-dlp` → not found; no pip/pipx/uv copy). Probes ran ephemerally via `uvx --from 'yt-dlp[default,curl-cffi]' yt-dlp` (uv cache only), version **2026.10.26** (= current brew stable). Installing for real: `brew install yt-dlp`; Homebrew's formula already bundles `curl-cffi` (`pypi_packages "yt-dlp[default,curl-cffi]"`) and deno. Also available: `ffmpeg`/`ffprobe` (/opt/homebrew/bin). gallery-dl and aria2c are absent.
- **Forum.** 56 requests (session check, 50 topics, 4 searches, 1 topic), ≥ 800 ms apart, cookie never printed.
- **rule34video:**
  - `yt-dlp -F` on 3122623 and 3291859 → `ERROR: No video formats found!`
  - curl page → 200 with flashvars. 8 posts' pages via curl at 1 req/s: all 200, 4×(360p…4k), 4×(360p…1080p). Durations 0:15–10:00.
  - 1 MB Range test of the 2160p URL: 302 → 206, `content-range 0-1048575/588570864`, ffprobe h264 3840×2160 60 fps. File deleted.
  - Node `fetch` page → 200, server ddos-guard.
- **spankbang 8ajmj:**
  - `-F` without impersonation → HTTP 403. With `--impersonate chrome`: 240p/480p/720p/1080p (60 fps) mp4 + HLS.
  - `-g -f 1080p` then 1 MB Range: 206, `0-1048575/91730824`, 1920×1080 60 fps. File deleted.
  - `duration` = NA.
- **iwara:**
  - fyicIcwG3DLn0U without impersonation → JSON parse error (Cloudflare challenge page; curl to api/www → 403 `cf-mitigated: challenge`).
  - With impersonation, all 6 iwara videos linked from the sample listed formats `[preview,]360,540,Source` anonymously (age_limit 18).
  - Source URL: Range 0-0 → 206 `…/186215342`. ffprobe on 3 Source URLs: 1920×1080 60 fps / 1280×720 30 / 1920×1080 60; durations 278.4 s, 305.7 s, 136.7 s.
- **hanime1 104088, 150063, 103468:**
  - curl/Node → 403 Cloudflare. yt-dlp generic + impersonation → 3 mp4s (480/720/1080).
  - 1080p signed URL: Range 0-0 with Referer hanime1.me → 206 `…/86296536`. ffprobe without that Referer → 403.
- **hmvmania:**
  - zzbusio-best-of-hentai-2: page 200 (Node too), `data-video-source` mp4, Range 0-0 → 206 `…/65083672`.
  - radical-phonk-hmv: 404 (page removed).
- **newgrounds 884004:** yt-dlp (with and without impersonation) and curl → 403, title "NG Guard".
- **pornhub `682386c15d3eb`:** 240–1080p30, duration 1010 s.
- **xvideos `ubkpdob25ec`:** mp4-low/high + HLS ≤1080p, 1014 s.
- **eporner `eywIL1eifS2`:** 240p…2160p (h264 + AV1), 1223 s. All three with `--skip-download`.

Workarounds considered:
- **Plain Node `fetch` for every site.** Works for rule34video and hmvmania only; the Cloudflare sites (iwara, hanime1, spankbang) return 403 challenges, and undici can't fake a Chrome TLS fingerprint.
- **yt-dlp as a subprocess, impersonating Chrome (curl_cffi).** Works for iwara, hanime1, spankbang, pornhub, xvideos, eporner. Needs `brew install yt-dlp`. This is the recommended route for those sites.
- **yt-dlp for rule34video.** Doesn't work: the extractor is broken in 2026.10.26 (markup change). Parse the flashvars ourselves: simpler, no subprocess, gives 4K. Revisit if yt-dlp fixes it.
- **Headless browser for Cloudflare / NG Guard.** Not needed for the sites yt-dlp covers. Newgrounds' guard plus adult login make a hand-off the only sane option.
- **iwara login.** Not needed for the 6 tested videos. Private ones are "possible with an iwara account" (yt-dlp `--netrc`); no account was created.
- **Patreon / Fanbox / Fantia.** Paid posts. "Possible with a paid membership + `--cookies-from-browser`"; out of scope. Not feasible.
- **Mirrors.** When a post links the same video on several sites, prefer the direct/highest one: rule34video (4K, no subprocess) > iwara Source > eporner > spankbang > hanime1 > pornhub/xvideos.

Recommendation for the scout:
1. **Detect** in `topicFiles()` (after MEGA/pixeldrain), over the same `hrefs` list, with the `#…` fragment stripped:
   ```
   const STREAM_RE = /^https?:\/\/(?:www\.|de\.|m\.)?(rule34video\.com\/video\/\d+|iwara\.tv\/video\/[A-Za-z0-9]+|hanime1\.me\/watch\?v=\d+|hmvmania\.com\/video\/[^/#?]+|spankbang\.com\/[a-z0-9]+\/video\/|pornhub\.(?:com|org)\/view_video\.php\?viewkey=\w+|xvideos\.com\/video\.?\w+|eporner\.com\/(?:video-|hd-porn\/)\w+)/i
   ```
   Also add the hosts to `EXTERNAL_HOSTS` (rule34video.com, iwara.tv, hanime1.me, spankbang.com, hmvmania.com, newgrounds.com, pornhub.com, xvideos.com, eporner.com, patreon.com) so the candidate list's `hosts` shows them.
2. **List** every link as pseudo-files `{name: '<title> <height>p<fps>fps.mp4', size, host: 'stream', url: <page url>, height, fps, duration}`. One entry per quality is fine: `planScenes` pairs by name and `lib.videoPreference(name,size,vr)` already scores `2160`/`60fps` words against `FLAT_MAX_HEIGHT` / `VR_MAX_HEIGHT` / `VR_MAX_FPS`, so ≤4K flat and ≤8K/≤60 fps VR come out right.
   - rule34video/hmvmania: parse the page with `fetch`. `size` comes from a `Range: bytes=0-0` probe (`content-range` total), or leave it 0.
   - Others: `yt-dlp --impersonate chrome -J --no-warnings <url>`, then `formats[]` (height, fps, filesize|filesize_approx, vcodec, protocol). Skip `preview`, prefer h264/https over HLS and over AV1 (headset decode).
3. **Verify the match before downloading.** Video duration vs the paired main script's `lib.scriptDurationMs()`:
   - Accept when `last ≤ dur + 2 s` and `last ≥ 0.8·dur` (scripts may stop before credits). Otherwise skip that link and try the next mirror.
   - Duration sources: rule34video JSON-LD; yt-dlp `duration` (pornhub/xvideos/eporner); `ffprobe -show_entries format=duration` on the chosen direct URL with its `http_headers` (iwara, spankbang, hanime1 give none). This also catches 15–33 s rule34video teasers when the script is full length.
   - Afterwards, check again with ffprobe on the finished file (the same check, the same tolerance).
4. **Download into `_staging/<name>.part`:**
   - rule34video/hmvmania: reuse the existing HTTP streaming branch with `Referer` + UA; Range is supported. Re-read the page just before downloading (fresh acctoken). Space rule34video requests ≥ 400 ms (< 3 req/s), and back off 60 s on 429.
   - yt-dlp sites:
     ```
     yt-dlp --impersonate chrome -f "<format_id>" --no-playlist --no-part -o <staging file> --retries 5 --fragment-retries 10 --concurrent-fragments 4 <page url>
     ```
     For VR use `-f "b[width<=8192][fps<=60]/bv*[width<=8192][fps<=60]+ba"`, for flat `-f "b[height<=2160]/bv*[height<=2160]+ba"`, when not picking by id. HLS needs ffmpeg (installed). Retry = rerun the pass: the scout deletes `.part` on failure, so there's no resume, the same as today.
5. **Fall back to the manual list** when yt-dlp is missing (`which yt-dlp` fails), or on exit codes/messages `HTTP Error 403`, `login required`, `privateVideo`, `NG Guard`, after 3 passes failing. Add `manualPageUrl` for newgrounds and private iwara links; mark Patreon/Fanbox/Fantia-only posts `paywalled` instead of `no-video`.
6. Existing `no-video` verdicts aren't re-judged automatically: run a `--full` pass with a `RULES_VERSION` bump, or `--add` the ids.

```js
import { execFile } from 'node:child_process';
const UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36';
const ytdlp = (args) => new Promise((ok, no) => execFile('yt-dlp', ['--impersonate', 'chrome', '--no-warnings', ...args],
  { maxBuffer: 64 << 20 }, (e, out, err) => (e ? no(new Error(String(err).trim().split('\n').at(-1))) : ok(out))));
const LABEL_H = { '4k': 2160, '8k': 4320, '1440p': 1440, '1080p': 1080, '720p': 720, '480p': 480, '360p': 360 };
const isoSec = (d) => { const m = /PT(\d+)H(\d+)M(\d+)S/.exec(d || ''); return m ? m[1] * 3600 + m[2] * 60 + +m[3] : null; };

/** Video variants a streaming page offers: [{name, size, host:'stream', url, direct, headers, height, fps, duration}]. */
export async function streamFiles(page) {
  const url = page.split('#')[0];
  if (/rule34video\.com\/video\//.test(url)) {
    const html = await (await fetch(url, { headers: { 'user-agent': UA } })).text();
    const title = (html.match(/<title>([^<]+?)(?: - Rule34Video)?<\/title>/) || [])[1] || 'video';
    const duration = isoSec((html.match(/"duration": "([^"]+)"/) || [])[1]);
    return [...html.matchAll(/video_(alt_)?url(\d?): '([^']+)'/g)].map(([, , n, direct]) => {
      const label = (html.match(new RegExp(`video_${n ? 'alt_' : ''}url${n}_text: '([^']+)'`)) || [])[1] || '';
      const height = LABEL_H[label.toLowerCase()] || 0;
      return { name: `${title} ${height}p.mp4`, size: 0, host: 'stream', url, direct, height, fps: 60, duration,
        headers: { 'user-agent': UA, referer: 'https://rule34video.com/' } };
    });
  }
  if (/hmvmania\.com\/video\//.test(url)) {
    const html = await (await fetch(url, { headers: { 'user-agent': UA } })).text();
    const direct = (html.match(/data-video-source="([^"]+\.mp4)"/) || [])[1];
    return direct ? [{ name: decodeURIComponent(direct.split('/').pop()), size: 0, host: 'stream', url, direct,
      headers: { 'user-agent': UA, referer: 'https://hmvmania.com/' } }] : [];
  }
  const j = JSON.parse(await ytdlp(['-J', '--no-playlist', url]));          // iwara, hanime1, spankbang, pornhub, xvideos, eporner
  return j.formats.filter((f) => f.format_id !== 'preview' && f.vcodec !== 'none' && !/av01/.test(f.vcodec || ''))
    .map((f) => { const height = f.height || +(/-(\d{3,4})p\.mp4/.exec(f.url) || [])[1] || 0;
      return { name: `${j.title} ${height}p${f.fps ? `${Math.round(f.fps)}fps` : ''}.${f.ext}`, size: f.filesize || f.filesize_approx || 0,
        host: 'stream', url, formatId: f.format_id, direct: f.protocol === 'https' ? f.url : null, headers: f.http_headers,
        height, fps: f.fps, duration: j.duration ?? null }; });
}
/** Script/video match: the script's last action falls inside the video's last 20%. */
export const durationMatches = (videoSec, scriptMs) => videoSec > 0 && scriptMs / 1000 <= videoSec + 2 && scriptMs / 1000 >= 0.8 * videoSec;
```

Manual (browser) instructions if needed (newgrounds, private iwara, anything yt-dlp rejects):
1. Open the post with `npm run manual:open`. Open the video link next to it and log in if the site asks (Newgrounds: an account with adult content enabled; iwara: your own account for private/"following" videos).
2. Download the highest quality offered, up to 4K for flat, or up to 8K / 60 fps for VR. Newgrounds: "Download" under the player; iwara: the download button → "Source".
3. Keep the site's filename, or rename it to the post's script name (`<Script name>.mp4`), so the scout pairs it by name.
4. Save it into `~/Downloads/for-fs/`. The next `--fetch` pass picks it up by name/size and removes it.

Patreon/Fanbox/Fantia-only posts: no hand-off (paid).

Sources:
- yt-dlp supported sites: https://github.com/yt-dlp/yt-dlp/blob/master/supportedsites.md
- yt-dlp extractors (read in the uv cache, 2026.10.26):
  - https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/rule34video.py (download-link regex, line 75)
  - https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/iwara.py (X-Version sha1, `impersonate=True`, login/privateVideo)
  - https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/spankbang.py (`impersonate=True`)
- yt-dlp impersonation: https://github.com/yt-dlp/yt-dlp#impersonation
- Homebrew formula (bundles curl-cffi): https://github.com/Homebrew/homebrew-core/blob/HEAD/Formula/y/yt-dlp.rb
- rule34video 429 above ~3 req/s: funsync-player skill (2026-10-27 poster run)
- Probes: this session, 2026-11-03 (HTTP statuses above)
