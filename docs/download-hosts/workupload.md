# Workupload
Verdict: auto-with-limits

Seen on EroScripts: Rare, and most links are dead. Forum search `workupload.com` (3 pages × 3 queries, 2026-11-03) found ~80 distinct links across ~75 topics, dated 2021-04 to 2026-10. They usually sit in **replies**, not in the first post: 89697 post 14, 197619 posts 15/18/24, 268572 post 10, 169351 post 23. A scout that only parses `posts[0].cooked` misses them. (`att.external` from `getTopicAttachments` does see them; that's why the candidate list says `Workupload`.) In the candidate list, only 89697 and 197619 mention Workupload. **All their Workupload links are dead (404):** `archive/pY4bkwWWYJ`, `file/uvcb45xEMLf`, `file/kVZcUZskTzx`, `file/JxWJHfCajrX`. Both posts also have MEGA.
Liveness of 76 probed unique links: **9 alive (~12 %)**, 67 × HTTP 404 "File not found". Alive: `file/H3SRpZJ4tGR` (t169351), `file/E9tttwaf5Kr` (t316072), `file/PkbfHMbQFS6` (t306558), `file/6ENJpuELDQm` (t281719), `file/xbDqwKyTYjd` (t104649), `file/pDmxx24C6eh` (t89847), `file/GU8mP9MuBLD` (t21576), and the password-protected `file/Dj7feMYhy2c` (t119547) and `archive/zX2p4BGm63` (t87972, password `moko` in the post).
Link shapes: `https://workupload.com/file/<11 alnum>`, `https://workupload.com/archive/<10 alnum>`. Uploaders sometimes add a `.deleteme` suffix to dodge scanning; the post says to strip it (e.g. 87972: `RJ374870.z01.deleteme`). Split archives (`.z01/.z02/.zip`, "extract twice" double 7z) also show up.

How downloads work (all observed live, 2026-11-03):
1. **Bot check (SHA-256 proof-of-work, no CAPTCHA image, no wait timer).** Without a fresh check, every HTML/API page returns 200 with `<title>workupload - Are you a human?</title>`. That page's inline JS does the following:
   - `GET /puzzle` → `{"success":true,"data":{"puzzle":"<unixTs>.<hex>","range":10000,"find":[3 sha256 hex]}}`. It also sets cookie `captcha` (domain `.workupload.com`, Max-Age 30 d), which holds the puzzle plus an encrypted `data` blob.
   - Find each `i < range` where `sha256(puzzle + i)` is in `find`. That's 3 hits from ≤10 000 hashes, about **5 ms** in Node.
   - `POST /captcha` with form `captcha=<i1> <i2> <i3> ` (space-separated, trailing space). The reply is 200 with an empty body, and the `captcha` cookie is re-set.
   - The browser re-solves every 20 s (`setInterval(solvePuzzle, 20000)`). Measured validity: pages still passed 16 s and 31 s after solving and were challenged again at 47 s.
2. **File page** `GET /file/<key>` gives:
   - `<h1>NAME (166.61 MB)</h1>`
   - a table with `Filename:&nbsp;</td><td …>NAME</td>`, `Filesize:&nbsp;</td><td>174704820 (Byte)</td>` and `Checksum:&nbsp;</td><td …><sha256> (SHA256)`
   - a button `<a href="/start/<key>">`
   - for a dead file: HTTP 404, title "File not found".
3. **Start**: the `/start/<key>` page only runs XHR `GET /api/file/getDownloadServer/<key>` → `{"success":true,"data":{"url":"https://f91.workupload.com/download/<key>"}}` (server `fNN` varies: f84/f91/f93/f95). **Skipping `/start` works:** calling the API directly right after a solve returned the URL.
4. **Download** `GET https://fNN.workupload.com/download/<key>` with the same cookies → 200 `application/octet-stream`, `Content-Disposition: attachment; filename="…"`, `Content-Length`.
   - **`Range` is ignored**: a request with `Range: bytes=1048576-` got 200 with the full length, no `Content-Range` and no `Accept-Ranges`.
   - Without cookies the reply is `302 → https://workupload.com/file/<key>` (meta-refresh page).
   - The download URL still worked 65 s after the solve, so the bot-check window only matters for the page/API calls.
5. **Password-protected** file or archive: the page holds `<form name="passwordprotected_file|archive">` with fields `passwordprotected_<kind>[password]`, `[submit]` (empty) and `[key]` (= key). No CSRF token.
   - `POST` the form to the same URL → 200 with the normal page, plus a new `token` cookie (26 chars, a session id).
   - The download needs **both** `captcha` and `token`. Without `token`, `getDownloadServer` still returns a URL, but the file server answers 302 back to the file page.
   - A wrong password → 200 with the password form again (no error text) and a fresh `token`.
6. **Archive** `GET /archive/<key>` gives:
   - `<h1>KEY 4.77 GB</h1>` and a table of rows `<b>NAME</b> … <td>2.00 GB</td> … <a href="/file/<fileKey>">`
   - "Archive information" JS with the exact total in bytes
   - a "Download archive" link `/archive/<key>/start`. That page holds `var kas = ["<fileKey>_<bytes>", …]` (exact sizes) and fetches each member through `getDownloadServer` one after another. **There is no zipped bundle;** an archive is just a folder of `/file/` keys.

Limits:
- No visible free quota, wait timer or CAPTCHA. Observed: 9 download starts and ~90 page loads in ~10 min from one IP, none refused.
- Speed: 2 MiB arrived in 1.3–2.2 s (≈0.9–1.6 MB/s, mostly time-to-first-byte). Sustained speed was not measured under the 2 MB probe rule; the site's own archive JS assumes 32 MiB/s (`timer = 1000/(32768*1024)*size + 3000`). [INFERENCE] One accidental full 174.7 MB body arrived within a single short cell, so sustained speed is well above 1 MB/s.
- Max 2 GB per file (FAQ #5, and the home page says "Up to 2 GB files").
- **Files are deleted after 30 days without a download** (FAQ #6: "If your file is 30 days inactive (no call receive), it will be removed"). That's why ~88 % of the forum's links are dead.
- No resume (Range ignored); a broken transfer must restart from byte 0.
- `robots.txt` on workupload.com: `Disallow: /download/ /image/ /thumb/ /legal/ /imprint`. The actual downloads come from `fNN.workupload.com` (its robots.txt → 403). The Terms (/legal/terms) say nothing about automated access.
- ToS-in-spirit caveat: the "Are you a human?" page is an anti-bot gate. Solving it with the site's own published JS algorithm is exactly what its page does, but it is still a bot check. Main should decide whether that is acceptable; if not, the verdict becomes browser-handoff.

Probe results (2026-11-03, Node/Bun `fetch`, manual cookie jar, UA Chrome 130):
- `GET /file/uvcb45xEMLf` (no cookies) → 200 "Are you a human?" + PoW script.
- `GET /puzzle` → 200 JSON (range 10000, 3 targets). `POST /captcha` → 200 empty. Solved in 5 ms; solutions e.g. `[3615,4921,9494]`.
- Example posts:
  - 89697 `archive/pY4bkwWWYJ` → 404
  - 197619 `file/uvcb45xEMLf`, `file/kVZcUZskTzx` → 404
  - forum-search link set: 76 unique probed, 9 alive (list above)
- `GET /start/PkbfHMbQFS6` → 200 page with the `getDownloadServer` XHR. `GET /api/file/getDownloadServer/PkbfHMbQFS6` → 200 `{"success":true,"data":{"url":"https://f91.workupload.com/download/PkbfHMbQFS6"}}`.
- Download URL with cookies → 200, `Content-Length 174704820`, MP4 (`ftypmp42`), filename in `Content-Disposition`. Without cookies → 302 to the file page.
- **Mistake on this probe:** the first test used `Range: bytes=0-1048575` and buffered the body. The server ignored Range, so the whole 174.7 MB file came down. It was deleted from /tmp right away. Every later probe streamed and aborted at 2 MiB.
- Range test `bytes=1048576-` → 200 full length (no resume).
- `getDownloadServer/E9tttwaf5Kr` straight after the solve, skipping `/start` → OK. 2 MiB in 1.4 s. The same URL 65 s later → 200 again.
- Bot-check lifetime: OK at 16 s and 31 s, challenged at 47 s.
- Password (topic 87972/119547 text "Password: moko"):
  - `POST /file/Dj7feMYhy2c` → 200, page `RJ01037485_Double_Archive.7z (1.96 GB)`, `token` cookie set. Download → 200, 2 107 119 482 B.
  - With the `token` cookie removed → 302 to the file page.
  - Wrong password with a fresh session → form shown again.
  - `POST /archive/zX2p4BGm63` → member list (3 files: `RJ374870.z01.deleteme`, `.z02.deleteme`, `RJ374870.zip`). `/archive/zX2p4BGm63/start` → `kas = ["H7GxdJZzVLC_2147483648","uyUbGR27GZr_2147483648","tXEYLKbQNQt_824377859"]`.
- Transport quirk: Bun's fetch threw `ECONNRESET` on the first request after more than 5 s idle (server `Keep-Alive: timeout=5`). One immediate retry fixed it every time.
- Tooling: `yt-dlp`, `gallery-dl`, `rclone`, `aria2c` are not installed (`which` found only curl/node). None is needed.

Workarounds considered:
- **Replicate the site's own PoW in Node:** works (above). It's cheap, needs no third-party service, no account and no CAPTCHA solver.
- Skip `/start` and call `getDownloadServer` directly: works and saves one request.
- `/archive/<key>/start` as a one-shot bundle: it doesn't exist (it just loops per file); iterate the member `/file/` keys instead.
- Resume through Range: doesn't work (ignored). Retry from zero; check the size and the page's SHA-256.
- Reuse one `captcha` cookie for a long session: no, it's valid for <47 s. Re-solve before any page/API call that comes >20 s after the last solve, and whenever the "Are you a human?" title shows up.
- Download-manager CLIs (JDownloader has a Workupload plugin; yt-dlp/gallery-dl don't support it): not needed.
- Premium/coupon (`/api/file/coupon`, login): not used. "possible with a Workupload account/coupon", probably not needed since free has no visible limit.
- Browser hand-off: works, but only needed if the PoW flow changes or is ruled out on ToS grounds.

Recommendation for the scout:
- **Detect:** `/https?:\/\/(?:www\.)?workupload\.com\/(file|archive)\/([A-Za-z0-9]{8,14})/g`. Run it over **all posts'** `cooked` in the topic JSON (the scout already reads them for `att.external`), not only `posts[0]`. Host id `workupload`; remove it from the "can't fetch" `EXTERNAL_HOSTS` handling once it's implemented.
- **List** (`workuploadFiles(kind, key, postText)`), one PoW-gated session per topic:
  - `/file/<key>`: name, size and sha256 from the `Filename/Filesize/Checksum` table.
  - `/archive/<key>`: member keys from `href="/file/<k>"` plus names from `<b>NAME</b>`. Exact sizes come from `/archive/<key>/start` `kas` or from each member's file page (which also gives the sha256).
  - 404 → dead link. Record it like `MEGA (unreadable)`, e.g. `hosts.add('Workupload (deleted)')`, and never send it to the manual list.
  - Password form → take candidates from the post text (`/pass(?:word)?\s*[:=]\s*([^\s<]+)/i`), POST, and keep the `token` cookie with the file. No password, or a wrong one → `hosts.add('Workupload (password)')` → manual list.
  - Strip a trailing `.deleteme` from the stored name.
- **Download** (new `file.host === 'workupload'` branch in `downloadFile`): new session → (password POST if needed) → solve → `getDownloadServer` → stream the `url` with the same cookies to `tmp`. Check `Content-Length` == the listed size and the SHA-256 from the page.
  - On 302, or a body that is HTML: re-solve once and retry.
  - On a network error: restart from 0 (no Range), at most 2 tries.
  - Then fall back to `MANUAL_HOSTS` (`manualPageUrl` → `https://workupload.com/file/<key>`).
  - Also fall back if `/puzzle` stops returning `{data:{puzzle,range,find}}`, so a changed bot check degrades to the manual list instead of failing silently.
- Pace workupload requests ≥ 500 ms apart and run one download at a time.

```js
import { createHash } from 'node:crypto';
const WU = 'https://workupload.com', UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Safari/537.36';
const HUMAN = /<title>workupload - Are you a human\?/;
export function workuploadSession() {
  const jar = new Map(); let solvedAt = 0;
  const req = async (url, init = {}, retry = true) => {
    const headers = { 'User-Agent': UA, Cookie: [...jar].map(([k, v]) => `${k}=${v}`).join('; '), ...init.headers };
    let r; try { r = await fetch(url.startsWith('http') ? url : WU + url, { ...init, headers, redirect: 'manual' }); }
    catch (e) { if (retry) return req(url, init, false); throw e; }              // stale keep-alive (timeout=5)
    for (const c of r.headers.getSetCookie()) { const kv = c.split(';')[0], i = kv.indexOf('='); jar.set(kv.slice(0, i), kv.slice(i + 1)); }
    return r;
  };
  const xhr = { 'X-Requested-With': 'XMLHttpRequest', Accept: 'application/json' };
  const solve = async () => {                                                    // the site's own sha256 PoW, ~5 ms
    const d = (await (await req('/puzzle', { headers: xhr })).json())?.data;
    if (!d?.puzzle || !Array.isArray(d.find)) throw new Error('workupload: bot check changed');
    const hits = [];
    for (let i = 0; i < d.range; i++) if (d.find.includes(createHash('sha256').update(d.puzzle + i).digest('hex'))) hits.push(i);
    await req('/captcha', { method: 'POST', headers: { ...xhr, 'Content-Type': 'application/x-www-form-urlencoded' }, body: `captcha=${encodeURIComponent(hits.join(' ') + ' ')}` });
    solvedAt = Date.now();
  };
  const fresh = () => (Date.now() - solvedAt > 20000 ? solve() : null);
  const page = async (path, init) => {                                           // null = deleted (404)
    for (let n = 0; n < 2; n++) {
      await fresh(); const r = await req(path, init); const html = await r.text();
      if (r.status === 404) return null;
      if (!HUMAN.test(html)) return html; solvedAt = 0;
    }
    throw new Error('workupload: bot check keeps coming back');
  };
  const unlock = (kind, key, password) => page(`/${kind}/${key}`, { method: 'POST', headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ [`passwordprotected_${kind}[password]`]: password, [`passwordprotected_${kind}[submit]`]: '', [`passwordprotected_${kind}[key]`]: key }) });
  const fileInfo = (html) => ({ name: html.match(/Filename:&nbsp;<\/td><td[^>]*>([^<]+)/)?.[1].replace(/\.deleteme$/, ''),
    size: Number(html.match(/Filesize:&nbsp;<\/td><td>(\d+)/)?.[1]), sha256: html.match(/([0-9a-f]{64}) \(SHA256\)/)?.[1] });
  const download = async (key) => {                                              // Response streaming the file; no Range support
    for (let n = 0; n < 2; n++, solvedAt = 0) {
      await fresh(); const { data } = await (await req(`/api/file/getDownloadServer/${key}`, { headers: xhr })).json();
      const r = await req(data.url); if (r.status === 200 && !/text\/html/.test(r.headers.get('content-type'))) return r;
    }
    throw new Error('workupload: download refused (download it by hand: --manual-list)');
  };
  return { page, unlock, fileInfo, download };
}
```
Archive members: `[...html.matchAll(/<b>([^<]+)<\/b>[\s\S]*?href="\/file\/([A-Za-z0-9]+)"/g)]` → `{name, key}`. A password-protected page shows `/name="passwordprotected_(file|archive)\[password\]"/`.

Manual (browser) instructions if needed:
1. Open `https://workupload.com/file/<key>` (or `/archive/<key>`). The "Security Check – Checking that you are not a robot" page passes by itself within a second or two.
2. If it asks for a password, enter the one from the EroScripts post (often "Password: …" near the link) and confirm.
3. Click **Download**. Its button only shows ~1–3.5 s after load, behind an ad. For an archive, click each member's "Download" or "Download archive", which downloads the members one after another (allow multiple downloads).
4. Save into `~/Downloads/for-fs` and keep the server's filename. If it ends in `.deleteme`, remove that suffix. For split archives (`.z01/.z02/.zip`), keep every part side by side under its original name.
5. "File not found" means the file was deleted (30 days without downloads); use the post's other host or skip it.

Sources:
- https://workupload.com/faq (#5: 2 GB per file; #6: deletion after 30 days inactive)
- https://workupload.com/ ("Up to 2 GB files | without registration")
- https://workupload.com/robots.txt
- https://workupload.com/legal/terms
- Live page source of https://workupload.com/file/PkbfHMbQFS6, its `/start/`, `/puzzle`, `/captcha` and `/api/file/getDownloadServer/` (2026-11-03)
- https://workupload.com/archive/zX2p4BGm63 and its `/start` (2026-11-03)
- EroScripts topics 89697, 197619, 87972, 119547 and `search.json?q=workupload.com`
