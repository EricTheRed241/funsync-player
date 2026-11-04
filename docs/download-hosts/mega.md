# MEGA
Verdict: auto-with-limits

Seen on EroScripts: Common. It is the biggest host in the download queue: 157 of the 191 candidates' videos (107 posts, 108 unique links, 166.8 GB) plus 43 scripts. The median video is 0.37 GB, 23 are ≥ 2 GB, 6 are ≥ 5 GB and the largest is 12.4 GB. In an even sample of 40 of the 1,038 `no-video` posts, 1 had a MEGA link (82533), and that link is dead (ENOENT). So `no-video` hides few MEGA posts, roughly 2–3 %.
Link shapes seen:
- `https://mega.nz/folder/<8>#<22-char key>`: 91 of 108 candidate links. 14 of the folders have subfolders, 1–32 files each.
- `https://mega.nz/file/<8>#<43-char key>`: 17 of 108.
- Legacy `https://mega.co.nz/#!<8>!<43>` (and `#F!<8>!<22>` for folders). Forum search `"mega.co.nz"` returned 29 posts, for example 160255 and 159882. The scout misses all of them today: its href regex `/mega\.(nz|io)\//` does not match `mega.co.nz`, `megaKeyOk` rejects it too, and `EXTERNAL_HOSTS` lacks it.
- Forms megajs mishandles: `www.mega.nz` throws "Invalid URL: wrong hostname" (the web server 301s it to mega.nz). `/folder/<h>#<k>/folder/<sub>` throws "Invalid argument". `/folder/<h>#<k>/file/<f>` loads, but the scout ignores the resolved node and lists the whole folder.
- `mega.io/file/…` and `mega.io/folder/…` answer HTTP 404. mega.io is the marketing site, not a link host; forum search for `"mega.io/file"` and `"www.mega.nz"` found 0 posts.

How downloads work: The API is anonymous JSON over `POST https://g.api.mega.co.nz/cs?id=<n>` with body `[{...}]`. megajs 1.3.10 is already a dependency.
- File link metadata: `{a:"g", p:<handle>}` returns `{s, at}`, the size and AES-encrypted attributes (name).
- Folder link: `{a:"f", c:1, ca:1, r:1}` with `?n=<folderHandle>` returns the whole tree. A child is downloaded with `{a:"g", g:1, n:<childHandle>}` plus `?n=<folderHandle>`.
- Download URL: adding `g:1` (and `ssl:0|2`) returns `g: "http://gfsNNN.userstorage.mega.co.nz/dl/<token>"`.
- The data is AES-128-CTR ciphertext. It is fetched as `<g>/<start>-<end>` (path form, 200), or with a `Range: bytes=` header (206 + `Content-Range`).
- Decryption can start at any 16-byte offset; megajs `download({start})` handles alignment. The MEGA "MAC" (a CBC-MAC integrity check) is verified in-stream only when `start === 0 && !end`. `megajs.verify(key)` checks a finished plaintext file.
- No cookies, no CAPTCHA, no hashcash on these calls (none seen).
- Errors on `g`, all permanent, i.e. a dead link:
  - `-9 ENOENT`: deleted.
  - `-6 ETOOMANY`: the owner's account was suspended. The MEGA webclient (`download2.js:920`) reads it that way: "If `g` request return ETOOMANY, it means the user who originally owned the file is suspended".
  - `-16 EBLOCKED`: taken down.

Limits:
- **Transfer quota per IP, rolling 6 h.** "the amount of data that can be downloaded from your IP address in six hours is limited … influenced by … geographical location and our current network usage" (MEGA Help, updated 2026-05-23). Free accounts are also counted by IP, so a free login gains nothing. MEGA publishes no number.
  - JDownloader's KB says "As of 2024-07 … free users can not download files >=5GB". I take that as the effective anonymous allowance being ≈ 5 GB per 6 h [INFERENCE: third-party statement, varies].
  - Since May 2026 MEGA says quota "only begins to apply to a given file once that file has been transferred in full more than 10 times". Popular EroScripts files will be past that.
- **The quota is readable anonymously:** `POST cs` `[{a:"uq",xfer:1,qc:1}]` returns `{bt, tah:[6 hourly byte buckets, oldest first], tar, …}`. `tal` (the allowance) is not returned to anonymous users.
- **Time until quota frees** (the MEGA webclient's formula, `download2.js` `_overquotaInfo`): `3600 − bt % 3600`, plus `3600` for each leading empty bucket in `tah`.
- **Over quota:**
  - A data request answers **HTTP 509** with header `X-MEGA-Time-Left: <s>`. megajs turns it into `Error("Bandwidth limit reached: N seconds until it resets")` with `err.timeLimit`.
  - The SDK also handles `g` returning `{e:-17, tl:<s>}` (`commands.cpp` `CommandGetFile`, field `tl`). megajs collapses that case to "MEGA servers returned an invalid response, maybe caused by rate limit", without the seconds.
- **Reservation:** `a:g` with `g:1` reserves the whole file in `tar`. I saw 1,334,839,159 = size − 2 MB after a 2 MB fetch. It was released about 60 s after the token went unused. Don't request download URLs for files you won't fetch right away.
- **Partial downloads:** only bytes actually transferred count (`tah` grew by exactly what was fetched). A tool that asks for the whole file needs a free quota ≥ the file size. A chunked or ranged downloader gets whatever fits and continues after the reset (JD KB: "Other tools will just download parts of that file until MEGA servers say 'enough for today' and resume later"). megajs fetches ≤ 1 MiB chunks, so it is in the second group.
- **Expiry:** MEGA "discard[s] incomplete transfers after 48 hours of inactivity" (help). This does not affect us: every megajs `download()` asks for a fresh `g` URL.

Probe results (2026-11-03, this machine, anonymous, megajs 1.3.10):
- Forum: 40 sampled `no-video` posts (all 200, 1 with a MEGA link), 6 search calls and 3 topic reads, about 49 forum requests in total.
- `loadAttributes()` on all 109 MEGA URLs (108 candidate links + 1 from the `no-video` sample, post 82533): **108 OK**. 91 are folders and 14 of those are nested. The one failure was 82533 with `ENOENT (-9)`.
- The six example posts load fine:

  | Post | Link | Files | Size |
  |---|---|---|---|
  | 109589 | folder | 4 | 3.64 GB |
  | 142298 | folder | 4 | 4.86 GB |
  | 309370 | folder | 13 | 8.60 GB |
  | 70706 | file | 1 | 1.34 GB |
  | 94296 | folder | 5 | 1.04 GB |
  | 118094 | folder | 6 | 5.60 GB |

- The legacy link in post 160255 (`mega.co.nz/#!…`) gives `ETOOMANY (-6)`: the owner is suspended, so the link is dead.
- Raw API on 70706:
  - `uq` before: `bt 18383, tah [0,0,0,0,0,0], tar 0, rtt 1`. No hashcash.
  - `g`: returned `s, at, msd, fa, g, ip, fh`.
  - `GET <g>/0-2097151`: 200, 2,097,152 bytes in 1.8 s.
  - `Range: bytes=0-1023`: 206, `content-range bytes 0-1023/1336936311`.
  - `uq` after: `tah[5] = 2097152`, `tar = 1334839159`.
- Folder child of 94296: `download({start:0,end:1MiB-1})` begins with `ftyp` (valid MP4). `download({start:524293,end:1MiB-1,maxConnections:1})` matched the head slice byte for byte, so an unaligned resume decrypts correctly.
- Full resume test on a 45,279-byte funscript inside a candidate folder:
  - Part 1 with `{end: cut-1}` gave 18,114 bytes.
  - `download({start:18114})` appended to the file gave 45,279 bytes, equal to the size.
  - `pipeline(createReadStream, verify(node.key))` passed. After flipping 1 byte, verify fails with "MAC verification failed".
- `verify()` runs at about 27 MiB/s: about 40 s per GB, 7.5 min for the 12.4 GB file.
- `qbq` (MEGA's "query bandwidth quota" pre-check) returned `0` for every query, even a bogus handle. It tells nothing while quota is available, and I could not test it near the limit.
- `g` with `n` in the body but no `?n=<folder>` query returns `-2` (EARGS).
- I did not trigger a real 509: that needs about 5 GB of quota. The 509 / `X-MEGA-Time-Left` handling comes from the megajs source (`main.node-cjs.js:1175-1183`) and the MEGA webclient (`download2.js:992`).
- Pitfall: destroying a `download({maxConnections:1})` stream crashes Node with an unhandled `AbortError` (megajs 1.3.10, line 1200). Keep the default of 4 connections.
- All temporary bytes were deleted.

Workarounds considered:
- **Resume partial downloads (megajs `download({start})` + `verify`): works, recommended.** Proven above. Without it, the scout deletes the `.part` on 509 and re-spends the same quota on the next try. Files larger than one window's allowance (6 videos ≥ 5 GB) can never finish today.
- **Sleep exactly the reset time** (`err.timeLimit`, else `uq`) instead of a fixed `--wait 10m`: works. 10 min is far too short (the reset is often hours), and 1 h may oversleep. Quota frees one hourly bucket at a time, so resuming as soon as the oldest bucket expires takes whatever it freed.
- **MEGAcmd 2.7.0** (`brew install --cask megacmd-app`, not installed): `mega-get <link> <dir>` works anonymously. It resumes and waits out over-quota on its own (SDK `tl`). The quota is the same per IP, so it adds nothing over megajs + resume, and it needs a background server, a GUI-sized install and output parsing. Not needed.
- **megatools 1.11.5** (`brew install megatools`, not installed): `megatools dl --path <dir> <link>` resumes automatically through a temp file ("Implemented automatic resume", 1.10.0) and allows "more retries during download (when over quota)" (#403). Same quota, so it gains nothing over megajs.
- **rclone:** its mega backend needs an account login. It is not for anonymous public links.
- **yt-dlp / gallery-dl / aria2c:** not installed, and none of them decrypts MEGA. Not useful.
- **Browser hand-off:** the browser uses the same per-IP quota, so it adds no bandwidth. It only helps if the person downloads on another network.
- **New IP (VPN, proxy rotation, IPv6 tricks), free account, CAPTCHA services:** a free account gives the same IP-based quota, and rotating IPs is quota evasion against the spirit of the ToS. Not recommended, not done.
- **Possible with a paid MEGA Pro / Pro Flexi account:** it has its own account transfer quota (log in with megajs `Storage`, pass its `api` to `File.fromURL(url, { api })`). Not evaluated (no paid services).

Recommendation for the scout:
1. **Detection (scan).** Match `/https?:\/\/(?:www\.)?mega(?:\.co)?\.nz\/(?:(?:file|folder)\/[\w-]{8}#[\w-]{22,43}(?:\/(?:file|folder)\/[\w-]{8})?|#F?![\w-]{8}![\w-]{22,43})/`.
   - Normalise the host to `https://mega.nz` before `File.fromURL`. Let `megaKeyOk` accept `mega(?:\.co)?\.nz`, and add `['mega.co.nz','MEGA']` to `EXTERNAL_HOSTS`. Drop `mega.io` from the scout regex: mega.io links 404.
   - In `megaFiles`, use `const node = await f.loadAttributes()` (it resolves `/file/<h>` to that file).
   - For `/folder/<sub>` links: strip the suffix, load the root, then find the child folder whose `downloadId[1] === sub` and list only that subtree.
   - Turn ENOENT(-9), ETOOMANY(-6) and EBLOCKED(-16) into host `MEGA (gone)`, not "unreadable", so these posts end as `no-video` and are not retried.
2. **Download.**
   - Keep `.part` for MEGA (no `rmSync` on error, and not in the SIGINT handler).
   - Resume with `node.download({ start: partSize })` in append mode. Keep the default 4 connections.
   - After a resumed download, run `verify(node.key)` over the whole file and delete the `.part` on MAC failure. A download that starts at 0 is MAC-checked in-stream.
3. **Quota.**
   - On 509, take `err.timeLimit`. On "invalid response" or EOVERQUOTA, compute the wait from `uq`.
   - Set `megaUntil = now + (s + 60) s`. For the rest of the pass, skip MEGA files as **waiting** (not failed) and keep downloading forum and pixeldrain.
   - Order the next pass with scenes that have a `.part` first.
   - Between passes: if only MEGA is waiting, sleep until `megaUntil` instead of `--wait` (print "MEGA quota: resumes at HH:MM"). With `--passes N` this drains the queue unattended.
   - Expect about 5 GB per 6 h, so the current 167 GB takes ≈ 33 windows ≈ 8–9 days of continuous running [INFERENCE from the reported allowance].
4. **Retry and fallback.**
   - Network errors (ECONNRESET, timeouts, 5xx): keep the `.part` and retry on the next pass from where it stopped.
   - Do not put MEGA files on the manual list: a browser hits the same per-IP quota.
   - The only manual case is the user's own paid account, or downloading on another network.
5. **Do not request `g:1` download URLs ahead of time** (it reserves quota). `loadAttributes()` alone is fine.

```js
const { File, verify } = await import('megajs');
const MEGA_RE = /https?:\/\/(?:www\.)?mega(?:\.co)?\.nz\/(?:(?:file|folder)\/[\w-]{8}#[\w-]{22,43}(?:\/(?:file|folder)\/[\w-]{8})?|#F?![\w-]{8}![\w-]{22,43})/g;
const megaUrl = (u) => u.replace(/^https?:\/\/(?:www\.)?mega(?:\.co)?\.nz/, 'https://mega.nz');
const MEGA_GONE = /ENOENT \(-9\)|ETOOMANY \(-6\)|EBLOCKED \(-16\)/;
class MegaQuotaError extends Error { constructor(s) { super(`MEGA transfer quota used up; frees in ${Math.ceil(s / 60)} min`); this.seconds = s; } }

/** Seconds until the oldest used hour of this IP's 6 h window frees (MEGA webclient formula). */
async function megaQuotaWait() {
  const r = await fetch(`https://g.api.mega.co.nz/cs?id=${Date.now() % 1e9}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '[{"a":"uq","xfer":1,"qc":1}]' });
  const [q] = await r.json();
  let s = 3600 - ((q.bt | 0) % 3600);
  for (const used of q.tah || []) { if (used) break; s += 3600; }
  return s;
}

/** Download a megajs file node to `tmp`, resuming a `.part` left by an earlier attempt. */
async function megaDownload(node, tmp) {
  let have = fs.existsSync(tmp) ? fs.statSync(tmp).size : 0;
  if (have > node.size) { fs.rmSync(tmp); have = 0; }
  if (have < node.size) {
    try {
      await pipeline(node.download({ start: have }), fs.createWriteStream(tmp, { flags: 'a' }));
    } catch (err) {
      if (err.timeLimit) throw new MegaQuotaError(Number(err.timeLimit));
      if (/invalid response|EOVERQUOTA/.test(err.message)) throw new MegaQuotaError(await megaQuotaWait());
      throw err; // network trouble: .part stays, next pass resumes
    }
  }
  if (have > 0) { // resumed: megajs skipped the MAC check, so check the whole file now
    try { await pipeline(fs.createReadStream(tmp), verify(node.key)); } catch (e) { fs.rmSync(tmp, { force: true }); throw e; }
  }
}
// fetchPass: if (file.host === 'mega' && Date.now() < megaUntil) { waiting++; continue; }
//   catch (e) { if (e instanceof MegaQuotaError) { megaUntil = Date.now() + (e.seconds + 60) * 1000; waiting++; } }
// fetchAll: if (!r.failed && r.megaUntil > Date.now()) await sleep(r.megaUntil - Date.now()); // instead of opts.wait
```

Manual (browser) instructions if needed: Normally not needed: a browser shares the same per-IP quota, and the scout with resume does the same job unattended. If the user still wants to fetch one by hand (for example on another network, or logged in to a paid account):
1. Open the post's MEGA link in a browser. For a folder, right-click the video, then **Download → Standard download**.
2. Let the MEGA web app finish. It pauses on "Transfer quota exceeded" and continues by itself after the countdown; keep the tab open.
3. Keep MEGA's filename exactly as offered (a ` (1)` suffix is fine), so `findLocalCopy` matches it by name and size.
4. Move the file into `~/Downloads/for-fs`, or leave it in `~/Downloads`; the next `--fetch` takes it from there.

Sources:
- MEGA Help, transfer quota (updated 2026-05-23): https://help.mega.io/files-folders/transfers/transfer-quota
- MEGA webclient, over-quota timer (`uq`, `tah`/`bt` formula), 509 handling and ETOOMANY = owner suspended: https://github.com/meganz/webclient/blob/master/js/transfers/download2.js
- MEGA webclient, `qbq` pre-check: https://github.com/meganz/webclient/blob/master/js/transfers/queue.js
- MEGA webclient, `getTransferQuota`: https://github.com/meganz/webclient/blob/master/js/fm/utils.js
- MEGA SDK, `g` response `tl` and `e` fields: https://github.com/meganz/sdk/blob/master/src/commands.cpp
- megajs 1.3.10 source in the repo: `node_modules/megajs/dist/main.node-cjs.js`. `download()` 1128-1304 (509 / `x-mega-time-left`, `start`/`end`, MAC off when `start>0`). `fromURL` 1385-1420 (hostname check). `verify` 352.
- JDownloader KB, MEGA (partial-range tools vs full-file, "≥5GB" free limit since 2024-07): https://support.jdownloader.org/en/knowledgebase/article/mega-nz-settings-and-troubleshooting
- megatools NEWS (auto-resume, over-quota retries): https://xff.cz/megatools/builds/NEWS
- Homebrew: `megacmd-app` 2.7.0 cask, `megatools` 1.11.5 formula (`brew info`)
