# Gofile
Verdict: auto-with-limits

Seen on EroScripts: **Very common, and usually in replies.** A forum search `gofile.io category:14 order:latest` returned 6 full pages, 300 topics, all posted 2026-09-17 … 2026-11-03 (about 7 weeks; the search stops there, so the real total is higher). In 81 of those 300 the link is in the first post. Of the 300: 264 were never judged by the scout, 13 are `no-video` (e.g. 344940, 344931, 343857, 343583, 344695#3, 344672#11, 344272#5, 344104#5, 343826#5, 342731#4) and 2 are candidates. 11 candidates in `eroscripts-scout.json` list Gofile as a host: 296363, 125426, 291669, 325466, 240773, 75569, 328815, 255998, 265277, 327672, 301494. In those, the Gofile link is in **a reply** (first post only for 291669, 255998 and 301494). Examples: 296363 #19 `gofile.io/d/V0gih8`, 75569 #7 `gofile.io/d/ug1yVy`, 125426 #18 `gofile.io/d/ZoW1RT`. `topicFiles()` reads only the first post's `cooked`, so reply mirrors are missed.
Link shapes:
- `https://gofile.io/d/<code>`: the code is 6 characters on older public folders (`E2TgxP`) and 8 on content made in 2026 (`kRIqtYgo`). A UUID also works (`/d/8cd0dbf9-…`).
- A code can point at a **folder** or at **a single file**. For example `TzmNzf0w` returns `type:"file"` (Bulma.mp4) with no children.
- Folders can have subfolders. 344940 `kRIqtYgo` has `Funscript/ OST/ Photos/ Videos/`, and its Videos folder holds four versions of the video (0.4–7.5 GB).
- Some posts zip the scene: 325466 is the split-ZIP Piper Perri post.

How downloads work (the web app's guest flow, read from gofile.io's own JS on 2026-11-03: `/js/app.js` → `services/api.js`, `services/contents.js`, `core/wt.js`, `/js/wt.obf.js`, `files/manager.js`):
1. **Guest account:** `POST https://api.gofile.io/accounts` with no body. The answer is `{status:"ok", data:{id, rootFolder, tier:"guest", token}}`, where `token` is 32 characters. The site keeps it in localStorage and copies it into the cookie `accountToken=<token>; domain=gofile.io`. `GET /accounts/website` (Bearer) shows tier and `ipTraffic` (bytes per day per IP).
2. **Website token (`X-Website-Token`):** `sha256(\`${navigator.userAgent}::${navigator.language}::${accountToken}::${Math.floor(Date.now()/1000/14400)}::${SALT}\`)` as hex.
   - The time slot changes every 4 hours.
   - The salt on 2026-11-03 was `12af056dacea0b`. I confirmed that by running `wt.obf.js` and checking the hash byte for byte.
   - Gofile **rotates the salt on the server**. gallery-dl changed it 2026-03-06, 03-28, 06-13 and 08-17; the old value was `5d4f7g8sd45fsd`.
   - `core/wt.js` says retired salts stop being accepted and that "a tab old enough to still hold one would get its IP **instant-banned**". The previous salt keeps working for about 24 hours (`RotateWtSecret` grace). The page itself refuses to sign requests once it has been open 6 hours.
   - gallery-dl issue #9183 is a user who got an IP ban from a stale token.
3. **Listing:** `GET https://api.gofile.io/contents/<code|uuid>?page=1&pageSize=1000&sortField=name&sortDirection=1[&password=<sha256 hex of password>]`
   - Headers: `Authorization: Bearer <token>`, `X-Website-Token: <wt>`, `X-BL: en-US`. The User-Agent and language must be the ones used in the hash.
   - Answer: `{status, data, metadata:{totalCount,totalPages,page,pageSize,hasNextPage}}`.
   - A folder's `data.children` is `{uuid: item}`. A file item has `type:"file"`, `name`, `size`, `md5`, `mimetype`, `serverSelected` (e.g. `store4`, `cold-na-phx-1`, `file-eu-par-1`), `link` (`https://<server>.gofile.io/download/web/<fileUuid>/<urlencoded name>`), `thumbnail`, `downloadCount`, and the optional flags `isFrozen` and `overloaded`.
   - A subfolder item has `type:"folder"` and an `id`, and is listed the same way.
   - Names can come back HTML-escaped (`Sun, Sand, Sea &amp; Sex - 4K.mp4`), so decode `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&#39;`.
4. **Access gates come back as HTTP 200 with `data.canAccess === false`:**
   - `password:true` means a password is needed. `passwordStatus` is `passwordWrong` or `passwordOk` once a hash was sent; the hash also works for the file UUIDs inside.
   - `public:false` means the content is not public.
   - `expire` set means the link expired.
   - Deleted content answers `{"status":"error-notFound","data":{}}`, also with HTTP 200 for share codes.
5. **Download:** `GET <item.link>` with `Cookie: accountToken=<the same token>`.
   - The token is only accepted after **that same token has listed the file's folder** (`files/manager.js`: "A listing call on the target folder is what authorizes the direct link").
   - The answer is `206`/`200` with `Accept-Ranges: bytes` and `Content-Disposition: attachment; filename*=UTF-8''…`, so **Range resume works**.
   - With no cookie, a foreign cookie, or a token that hasn't listed this folder, the server answers `302 → https://gofile.io/d/<fileUuid>` with a 79-byte HTML body.

**Premium only:**
- direct links (`POST /contents/<id>/directlinks`), folder/multi-file ZIP download, import (`/contents/import`), and restoring cold storage
- search, copy and move
- officially, "Direct API access to listings": the API docs page (`/api`, `get-content`) says non-premium tiers get `error-notPremium`. **Our listing works only because it makes the same signed request the website makes.**

Limits:
- Free traffic is **100 GB per 30 days** (`checkout/checkout.js` `PRICING.free.trafficPer30d`; Premium page). The FAQ says free traffic "may be counted by IP address", and "some download functionality can be temporarily restricted" once over the limit. I didn't see what that response looks like. This IP's `ipTraffic` shows about 8.5 GB in month key `10` (days 11/13/14; probably October); my 2 MB probes didn't show up during the session.
- No speed throttle, according to the FAQ.
- **Retention:** 10 days of inactivity for free content; it stays longer while it keeps being downloaded (FAQ).
- Expired content is deleted or goes to **cold storage**. Cold-storage items have `isFrozen:true` and can't be downloaded without a Premium import.
- `overloaded:true` means "Free downloads are paused for a while" on that server.
- API rate limit: `error-rateLimit`/HTTP 429. gallery-dl users report a 1-minute wait.
- The salt rotates, and a stale salt risks an IP ban (above).
- Terms ("Platform Abuse") forbid getting around traffic, storage or rate limits "through automated means, multiple accounts". So **use one guest token and keep it**, and never rotate accounts to get more traffic.

Probe results (2026-11-03, Node `fetch`; token, cookie and forum session never printed):
- `POST /accounts` → 200 `ok`, tier `guest`. I made 3 guest accounts while testing; the scout needs one.
- Running `wt.obf.js` in a `vm` with a stub `navigator` gave the same hash as sha256 of `UA::en-US::token::124563::12af056dacea0b`: **true**.
- Listings (`/contents/<code>`, wt + Bearer):

| Post | Code | Result |
|---|---|---|
| 291669 | `E2TgxP` | ok, 1 file, 1.42 GB, store4 |
| 296363 | `V0gih8` | ok, 6 Hegre 4K files, 3.4–5.5 GB (servers cold-na-phx-1, file-eu-par-1, store1/10, cold-na-phx-7, store-eu-par-8; none `isFrozen`) |
| 75569 | `ug1yVy` | ok, 6 mp4, 0.4–2.4 GB |
| 125426 | `ZoW1RT` | `error-notFound` |
| 240773 | `q7RWaK` | `error-notFound` |
| 325466 | `B8omoZ` | `error-notFound` |
| 328815 | `e5oA1ks3` | `error-notFound` |
| 255998 | `1hpx3R`, `0A7lDH` | `error-notFound` |
| 265277 | `7mGi3I` | `error-notFound` |
| 327672 | `FwOc3UQ9` | `error-notFound` |
| 301494 | `bkqi4H` | `error-notFound` |
| 344940 | `kRIqtYgo` | 4 subfolders (Videos: 4 files) |
| 344940 | `TzmNzf0w` | single file, Bulma.mp4 |
| 344931 | `gUHz4t2X` | subfolders |
| 343857 | `ca0NmEzU` | 3 files on cold* servers |
| 343583 | `vz24edqp` | mp4, funscript, ofsp |

- **9 of 13 links from the older candidates are already deleted**; all links from October 2026 posts were alive.
- Range downloads (`Range: bytes=0-2097151`, streamed in memory, nothing saved):
  - 291669 (store4): with the cookie → **206**, 2 097 152 bytes, `ftyp` header, `Content-Range …/1421932282`.
  - Mid-file `bytes=1000000000-1000999999` → 206.
  - 296363 file on `cold-na-phx-1` → 206.
  - 75569 `FREE FOR ALL.mp4` → 206.
  - 343583 file on `cold2` → 206.
  - With no cookie, a random 32-character cookie, or a fresh token that hadn't listed the folder → **302** to `gofile.io/d/<uuid>`.
  - A token that had listed a different folder → 302 for this file; after it listed `E2TgxP` → 206. So **authorization is per token and per listed folder**.
  - The listing token still worked about 15 minutes later.
- Not probed, on purpose: a missing or wrong `X-Website-Token`, because of the IP-ban risk. Password folders: none of the probed posts had one; the flow above is from the site's code.
- Probe files in `/tmp` deleted.

Workarounds considered:
- **Re-implementing the hash with a hard-coded salt** (what gallery-dl does): works today. But a rotation silently breaks it, and the site warns that a stale salt may get the IP banned. Use it only as a fallback, never when the rotation date is unknown.
- **Running gofile's current `wt.obf.js` each run**: works, and survives rotations because it's the code the website runs. Run it sandboxed: `node --permission -e <runner>` with no fs/net/child grants, the source and inputs on stdin, inside a `vm` context with only a primitive `navigator` stub (Node 26.4 tested).
  - It reads only `navigator.userAgent`, `navigator.language` and `Date`.
  - Get the file fresh at the start of every run, and again if the run lasts longer than 6 hours (the site's own limit).
- **yt-dlp:** not installed. Gofile is in yt-dlp's `unsupported.py` since 2026-06-28 (#17085; the extractor was removed). Not an option.
- **gallery-dl:** not installed. It is maintained on Codeberg now (the GitHub mirror is stale with the March salt). Its `gofile.py` uses the same flow with the salt hard-coded, `12af056dacea0b` since 2026-08-17, plus a `salt` option, `password` and `recursive`. It would work today, but it gets stale (repeated 401 issues gh#9183, #9706, cb#133). The scout shouldn't depend on it.
- **Premium direct links or ZIPs:** possible with a paid Premium account (also gives full API access and cold-storage import). Not used.
- **Browser hand-off:** the gofile page downloads without CAPTCHA. It's useful only as the fallback (rate limit, ban, salt broken) and for `isFrozen`/`overloaded` files, which a person can't download either.

Recommendation for the scout:
- **Detect** in each post (all posts of the topic, not just the first; reply mirrors are common): `/https?:\/\/(?:www\.)?gofile\.io\/d\/([A-Za-z0-9-]{6,36})/g`.
- **Token:** one persistent guest token in `<userData>/gofile-guest.json` `{token, createdAt}`. Never print it, and never in config.json.
  - If it's missing, or `/accounts/website` answers `error-token` or `error-wrongToken`, mint one with `POST /accounts`.
  - Never create more than one per run.
- **wt:** fetch `https://gofile.io/js/wt.obf.js` once per run, run it in the sandboxed child, and cache the wt per 4-hour slot. Use one fixed desktop UA (`Mozilla/5.0 … Chrome/…`) plus `X-BL: en-US` for listing and downloads.
- **List** recursively; pace about 1 request per second and stop on 429.
  - `error-notFound` → the post becomes `no-video` for this host.
  - `canAccess:false` with `password` → read a password from the post text (`password|pass|pw: (\S+)`); otherwise list the files for the manual list, marked "password".
  - Skip `isFrozen` and `overloaded` items for this pass (`overloaded` → retry on a later pass with `--wait`).
- **Files** come back as `{name, size, host:'gofile', url:item.link, folder: folderCode, md5}`. Check `md5` after download if wanted.
- **Download:** first re-list the folder with the stored token in the same run (that authorizes it), then `GET link` with `Cookie: accountToken=<token>`, a Range header from the `.part` size, and the same UA.
  - A 302 to `gofile.io/d/` means not authorized: re-list once and retry; a second 302 → manual list.
  - 429, 5xx or a network error → retry with backoff.
  - If the traffic cap is hit (100 GB per 30 days per IP), stop gofile for this run and add the rest to the manual list. Count the GB used per run against `/accounts/website.ipTraffic`.
- **Manual-list fallback:** add `'gofile'` to `MANUAL_HOSTS`. `manualPageUrl` for gofile = `https://gofile.io/d/<folder code>`, so the person sees the whole folder; keep the file name for matching. Remove `gofile.io` from `EXTERNAL_HOSTS` handling once it can be fetched.
- **Retention:** links die after about 10 idle days, so fetch soon after a post is judged.

```js
import crypto from 'node:crypto'; import { spawnSync } from 'node:child_process'; import fs from 'node:fs';
const UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Safari/537.36';
const API = 'https://api.gofile.io', LANG = 'en-US';
const RUNNER = `let r='';process.stdin.on('data',d=>r+=d).on('end',async()=>{const{src,ua,lang,token}=JSON.parse(r);const vm=require('vm');
const c=vm.createContext({});vm.runInContext('globalThis.navigator={userAgent:'+JSON.stringify(ua)+',language:'+JSON.stringify(lang)+'};globalThis.window=globalThis.self=globalThis;',c);
vm.runInContext(src,c,{timeout:2000});process.stdout.write(String(await vm.runInContext('generateWT('+JSON.stringify(token)+')',c)));});`;
let wtSrc = null, wtSrcAt = 0;
async function websiteToken(token) {               // re-fetch gofile's generator per run / every 6 h: the salt rotates
  if (!wtSrc || Date.now() - wtSrcAt > 6 * 3600e3) { wtSrc = await (await fetch('https://gofile.io/js/wt.obf.js', { headers: { 'User-Agent': UA } })).text(); wtSrcAt = Date.now(); }
  const out = spawnSync(process.execPath, ['--permission', '-e', RUNNER], { input: JSON.stringify({ src: wtSrc, ua: UA, lang: LANG, token }), encoding: 'utf8', timeout: 10_000 });
  if (out.status !== 0 || !/^[0-9a-f]{64}$/.test(out.stdout)) throw new Error('gofile: website token generator failed');
  return out.stdout;
}
export async function gofileToken(file) {          // one persistent guest token, never logged
  try { return JSON.parse(fs.readFileSync(file, 'utf8')).token; } catch {}
  const d = await (await fetch(`${API}/accounts`, { method: 'POST', headers: { 'User-Agent': UA } })).json();
  if (d.status !== 'ok') throw new Error(`gofile: ${d.status}`);
  fs.writeFileSync(file, JSON.stringify({ token: d.data.token, createdAt: Date.now() }), { mode: 0o600 }); return d.data.token;
}
const unescape = (s) => s.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'");
/** Files of a gofile code (folder or single file), recursive: [{name,size,url,md5,folder}]. Listing also authorizes the downloads. */
export async function gofileFiles(token, code, password, out = []) {
  const q = new URLSearchParams({ page: '1', pageSize: '1000', sortField: 'name', sortDirection: '1' });
  if (password) q.set('password', crypto.createHash('sha256').update(password).digest('hex'));
  const r = await fetch(`${API}/contents/${code}?${q}`, { headers: { 'User-Agent': UA, Authorization: `Bearer ${token}`, 'X-Website-Token': await websiteToken(token), 'X-BL': LANG } });
  const j = await r.json().catch(() => ({ status: `http-${r.status}` }));
  if (j.status !== 'ok') throw Object.assign(new Error(`gofile: ${j.status}`), { status: j.status });  // error-notFound, error-rateLimit…
  const d = j.data; if (d.canAccess === false) throw Object.assign(new Error('gofile: locked'), { status: d.password ? 'password' : 'private' });
  for (const it of d.type === 'file' ? [d] : Object.values(d.children || {})) {
    if (it.type === 'folder') { await new Promise((s) => setTimeout(s, 1000)); await gofileFiles(token, it.id, password, out); }
    else if (!it.isFrozen && !it.overloaded) out.push({ name: unescape(it.name), size: it.size, md5: it.md5, host: 'gofile', url: it.link, folder: code });
  }
  return out;
}
/** Resumable download; call gofileFiles(token, file.folder) earlier in the same run. */
export async function gofileDownload(token, file, part) {
  const have = fs.existsSync(part) ? fs.statSync(part).size : 0;
  const r = await fetch(file.url, { redirect: 'manual', headers: { 'User-Agent': UA, Cookie: `accountToken=${token}`, ...(have ? { Range: `bytes=${have}-` } : {}) } });
  if (r.status === 302) throw Object.assign(new Error('gofile: not authorized (re-list folder, else manual list)'), { status: 'unauthorized' });
  if (r.status !== 200 && r.status !== 206) throw new Error(`gofile: HTTP ${r.status}`);
  const { pipeline } = await import('node:stream/promises'); const { Readable } = await import('node:stream');
  await pipeline(Readable.fromWeb(r.body), fs.createWriteStream(part, { flags: r.status === 206 ? 'a' : 'w' }));
}
```
(The sketch is longer than 40 lines because the sandboxed wt runner is in it. Without a stable salt there is no shorter safe version.)

Manual (browser) instructions if needed:
1. Open the post's `https://gofile.io/d/<code>` link (the manual list gives the folder link). The page makes a guest account by itself; no login and no CAPTCHA.
2. In the folder, click a file's ⋮ menu (or open it) → **Download**. Download each needed file one by one: "Download as ZIP" and multi-select are Premium-only.
3. Open subfolders (e.g. `Videos/`, `Funscript/`) and take the video and script files, not the Photos/OST extras.
4. Keep the file name exactly as gofile shows it, and save into `~/Downloads/for-fs/` (or `~/Downloads`). The next `--fetch` matches by name and size.
5. "Content not found" means the link is dead; look for a newer mirror in later replies of the post. "File in cold storage" or "Server currently overloaded" means it can't be done for free now; skip it and try later.
6. Don't use a VPN or shared IP for big batches: free traffic is 100 GB per 30 days per IP.

Sources:
- gofile.io web app JS, read 2026-11-03: https://gofile.io/js/app.js, https://gofile.io/js/wt.obf.js, `/js/core/wt.js`, `/js/services/api.js`, `/js/services/contents.js`, `/js/files/manager.js`, `/js/files/passwordStore.js`, `/js/accounts/accountService.js`, `/js/checkout/checkout.js` (`free.trafficPer30d: '100 GB'`), `/js/pages/faq.js`, `/js/pages/terms.js`, `/js/pages/api.js`
- Gofile API reference: https://gofile.io/api · FAQ: https://gofile.io/faq · Terms: https://gofile.io/terms · Premium: https://gofile.io/premium
- gallery-dl gofile extractor (Codeberg, salt `12af056dacea0b`, 2026-08-17): https://codeberg.org/mikf/gallery-dl/src/branch/master/gallery_dl/extractor/gofile.py, issues https://github.com/mikf/gallery-dl/issues/9183 (IP ban after 401), https://github.com/mikf/gallery-dl/issues/9706, https://codeberg.org/mikf/gallery-dl/issues/133
- yt-dlp: gofile listed as unsupported, https://github.com/yt-dlp/yt-dlp/blob/master/yt_dlp/extractor/unsupported.py (commit "[ie/unsupported] Update unsupported sites (#17085)", 2026-06-28)
