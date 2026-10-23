// EroScripts API — Discourse REST API client for discuss.eroscripts.com
// Runs in main process (no CORS restrictions).

const log = require('./logger');
const { collectPreviewCandidates } = require('./preview-match');

const BASE_URL = 'https://discuss.eroscripts.com';
const USER_AGENT = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';

// Subcategory slugs under Scripts. Paid posts link out to the creator's
// store and carry no attachments; saying so beats "no scripts found".
const PAID_SLUG = 'paid-scripts';

// Replies are scanned too (updated versions, other users' variants), but a
// 1000-reply thread is not worth 50 requests. The topic endpoint returns the
// first 20; the rest come in batches of 20.
const MAX_POSTS = 100;
const POST_BATCH = 20;
// pixeldrain's API lists files freely but refuses programmatic downloads
// for free accounts ("hotlinking … requires a paid subscription", captcha),
// so its scripts are listed by name and opened in the browser instead.
const MAX_PIXELDRAIN_LOOKUPS = 10;

// Downloads go only to the forum itself, over https: the session cookie
// rides on every download request. Upload links on the S3 CDN are rewritten
// to the forum's short URL (which redirects there), so the CDN needs no entry.
const FORUM_HOST = new URL(BASE_URL).hostname;
const DOWNLOAD_HOSTS = new Set([FORUM_HOST]);
const UPLOAD_CDN_HOST = 'eroscripts-discourse.eroscripts.com';

// Post preview images (a video's library thumbnail) come from the forum or
// its upload CDN, where Discourse stores `/original/…` and `/optimized/…`
// images. They're held in memory, so the size is capped.
const PREVIEW_HOSTS = new Set([FORUM_HOST, UPLOAD_CDN_HOST]);
const PREVIEW_MAX_BYTES = 15 * 1024 * 1024;
const PREVIEW_MAX_REDIRECTS = 3;
const PREVIEW_TIMEOUT_MS = 30_000;
// Topic payloads reused by the preview batch (see getTopicData).
const TOPIC_CACHE_MS = 10 * 60_000;
const TOPIC_CACHE_SIZE = 50;

/**
 * Read a response body, giving up (null) once it passes `limit` bytes:
 * a missing or wrong Content-Length can't make it buffer more.
 */
async function readCapped(resp, limit) {
  if (!resp.body?.getReader) {
    const buf = Buffer.from(await resp.arrayBuffer());
    return buf.length > limit ? null : buf;
  }
  const reader = resp.body.getReader();
  const chunks = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > limit) {
      try { await reader.cancel(); } catch { /* already closed */ }
      return null;
    }
    chunks.push(value);
  }
  return Buffer.concat(chunks.map((c) => Buffer.from(c.buffer, c.byteOffset, c.byteLength)), total);
}

const SCRIPT_FILE_RE = /\.(funscript|zip)$/i;
const PIXELDRAIN_RE = /https?:\/\/(?:www\.)?pixeldrain\.(?:com|net)\/(u|l)\/([A-Za-z0-9]+)/gi;
// Discourse stores an original upload under its SHA-1 (`…/original/4X/b/5/7/<sha1>.ext`);
// its short URL is that SHA-1 in base 62.
const ORIGINAL_UPLOAD_RE = /\/original\/\d+X\/(?:[0-9a-f]\/)*([0-9a-f]{40})(\.[a-z0-9]+)$/i;
const BASE62 = '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ';

// Off-forum file hosts FunSync cannot fetch from. Named in the UI so a post
// whose scripts live there reads as "hosted on MEGA", not "no scripts".
const EXTERNAL_HOSTS = [
  ['mega.nz', 'MEGA'], ['mega.io', 'MEGA'], ['drive.google.com', 'Google Drive'],
  ['mediafire.com', 'MediaFire'], ['gofile.io', 'Gofile'], ['dropbox.com', 'Dropbox'],
  ['files.fm', 'Files.fm'], ['workupload.com', 'Workupload'], ['1drv.ms', 'OneDrive'],
  ['onedrive.live.com', 'OneDrive'], ['terabox.com', 'TeraBox'],
  ['pixeldrain.com', 'pixeldrain'], ['pixeldrain.net', 'pixeldrain'],
];

// Diagnostic logging — routes through electron-log so entries land in the
// rotating main log under %APPDATA%/funsync-player/logs/ (5MB cap) instead
// of a hand-rolled file in the install dir that silently fails to write on
// packaged Windows builds (Program Files is read-only).
function _debugLog(msg) {
  try { log.debug(`[EroScripts] ${msg}`); } catch { /* log unavailable */ }
}

/**
 * Pick a thumbnail from a Discourse `thumbnails[]` array.
 *
 * Entries come in several sizes (typically 1366/1920, 1024, 800, 400, 200,
 * 100, 50 px wide, the smaller ones as `optimized/` derivatives). The
 * smallest entry at least `MIN_THUMB_WIDTH` wins — big enough not to look
 * soft, small enough not to pull a full-size image per row.
 *
 * 400 because the row renders the thumbnail at 120 CSS px, which is 240
 * device px on a 2x display: a 200px source would be upscaled and visibly
 * soft there, while 1024+ is wasteful for a 120px slot. Topics whose
 * original is smaller than 400 (plenty are ~200) have no such entry and
 * fall through to the largest they do offer.
 *
 * @param {Array<{width?: number, url?: string}>} thumbnails
 * @param {number} [minWidth]
 * @returns {string|null}
 */
const MIN_THUMB_WIDTH = 400;

// Library posters are stored 640 px wide (electron/custom-thumbs.js), so a
// post preview imported as a video's thumbnail asks for at least that.
const POSTER_WIDTH = 640;
// Below this a preview makes a blurry tile. Plenty of posts set a 160×90
// "thumbnail" for the topic list and show the real preview further down.
const MIN_POSTER_SOURCE_WIDTH = 320;

// Absolute URL for a forum-relative one (`/uploads/…`).
function absoluteUrl(url) {
  if (!url) return null;
  return url.startsWith('http') ? url : `${BASE_URL}${url}`;
}

/** Widest `thumbnails[]` entry's width, or 0. */
function widestThumbnail(thumbnails) {
  if (!Array.isArray(thumbnails)) return 0;
  return Math.max(0, ...thumbnails.map((x) => Number(x?.width) || 0));
}

/**
 * Poster-size preview URL from a search result's `thumbnails[]`, or null
 * when the topic's image is too small to be worth it (the topic endpoint
 * may know a better one: see topicPreviewUrl).
 */
function searchPreviewUrl(thumbnails) {
  if (widestThumbnail(thumbnails) < MIN_POSTER_SOURCE_WIDTH) return null;
  return absoluteUrl(pickThumbnail(thumbnails, POSTER_WIDTH));
}

/**
 * The preview image of a topic (`/t/{id}.json` payload), as a URL, or null.
 *
 * Discourse's representative image for the topic (`thumbnails[]` /
 * `image_url`) wins when it is big enough to make a tile, at poster size.
 * Otherwise the first post's first image at least MIN_POSTER_SOURCE_WIDTH
 * wide (by its width attribute), skipping emoji: headings like
 * ":framed_picture: Preview" put one first. Failing both, whatever small
 * image the topic has beats no preview.
 */
function topicPreviewUrl(data) {
  const representative = pickThumbnail(data?.thumbnails, POSTER_WIDTH) || data?.image_url || null;
  if (representative && widestThumbnail(data?.thumbnails) >= MIN_POSTER_SOURCE_WIDTH) {
    return absoluteUrl(representative);
  }

  let firstImage = null;
  const html = data?.post_stream?.posts?.[0]?.cooked || '';
  for (const [tag, src] of html.matchAll(/<img[^>]+src="([^"]+)"[^>]*>/gi)) {
    if (/class="[^"]*\bemoji\b/i.test(tag) || src.includes('/images/emoji/')) continue;
    if (!/\.(jpe?g|png|gif|webp|avif)(\?[^"]*)?$/i.test(src)) continue;
    const url = absoluteUrl(decodeEntities(src));
    firstImage ||= url;
    const width = Number(tag.match(/\swidth="(\d+)"/i)?.[1]) || 0;
    if (width >= MIN_POSTER_SOURCE_WIDTH) return url;
  }
  return absoluteUrl(representative) || firstImage;
}

/**
 * Decode the HTML entities Discourse emits inside post markup.
 *
 * Attachment names come from the link TEXT of the rendered `cooked` HTML,
 * so a script called `Luna Snow & Sue Storm.funscript` arrives as
 * `Luna Snow &amp; Sue Storm.funscript` and would be saved to disk under
 * that literal name. That also breaks multi-axis pairing, which matches a
 * main against its companions by exact base name.
 *
 * Deliberately a small explicit table plus numeric refs rather than a DOM
 * round-trip: this runs in the main process, where there is no document.
 */
const _ENTITIES = {
  amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', '#39': "'", '#x27': "'",
};

function decodeEntities(text) {
  if (typeof text !== 'string' || !text.includes('&')) return text;
  return text.replace(/&(#x?[0-9a-f]+|[a-z]+);/gi, (whole, body) => {
    const key = body.toLowerCase();
    if (Object.prototype.hasOwnProperty.call(_ENTITIES, key)) return _ENTITIES[key];
    if (key.startsWith('#x')) {
      const code = parseInt(key.slice(2), 16);
      return Number.isFinite(code) ? String.fromCodePoint(code) : whole;
    }
    if (key.startsWith('#')) {
      const code = parseInt(key.slice(1), 10);
      return Number.isFinite(code) ? String.fromCodePoint(code) : whole;
    }
    // Unknown entity — leave it alone rather than mangling the name.
    return whole;
  });
}

function pickThumbnail(thumbnails, minWidth = MIN_THUMB_WIDTH) {
  if (!Array.isArray(thumbnails) || thumbnails.length === 0) return null;
  const usable = thumbnails.filter(x => x && typeof x.url === 'string' && x.url);
  if (usable.length === 0) return null;
  const big = usable
    .filter(x => Number(x.width) >= minWidth)
    .sort((a, b) => Number(a.width) - Number(b.width));
  return (big[0] || usable[0]).url;
}

function base62Sha1(sha1) {
  let n = BigInt(`0x${sha1}`);
  let out = '';
  while (n > 0n) {
    out = BASE62[Number(n % 62n)] + out;
    n /= 62n;
  }
  return out || '0';
}

/**
 * Canonical https forum URL for a link to a forum upload, or null if the
 * link points anywhere else. Accepts `upload://` short links, relative and
 * absolute `/uploads/…` paths on the forum, and originals on the upload CDN
 * (rewritten to their forum short URL). Query and fragment (`?dl=1`) are
 * dropped so the same file always maps to the same URL.
 */
function forumUploadUrl(href) {
  if (/^upload:\/\//i.test(href)) return `${BASE_URL}/uploads/short-url/${href.slice('upload://'.length)}`;
  let url;
  try { url = new URL(href, BASE_URL); } catch { return null; }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') return null;
  const onForum = url.hostname === FORUM_HOST;
  if (!onForum && url.hostname !== UPLOAD_CDN_HOST) return null;
  if (onForum && !url.pathname.startsWith('/uploads/')) return null;
  const original = url.pathname.match(ORIGINAL_UPLOAD_RE);
  if (original) return `${BASE_URL}/uploads/short-url/${base62Sha1(original[1].toLowerCase())}${original[2]}`;
  return onForum ? `${BASE_URL}${url.pathname}` : null;
}

function urlFileName(url) {
  const last = new URL(url).pathname.split('/').pop();
  try { return decodeURIComponent(last); } catch { return last; }
}

/**
 * Script-relevant links in one post's `cooked` HTML: forum uploads
 * (`.funscript` / `.zip`) as `{name, url}`, pixeldrain file and list links,
 * and links to hosts FunSync can't fetch from.
 *
 * An upload is any `<a>` pointing at a forum upload, whatever its other
 * attributes: Discourse marks some with `class="attachment"`, but posts
 * also carry plain `<a href="/uploads/short-url/….funscript">` links. The
 * name is the link text when that names a script file, else the URL's.
 */
function parsePostLinks(html) {
  const attachments = [];
  const seen = new Set();
  const pixeldrain = [];
  const external = [];
  for (const m of html.matchAll(/<a\s([^>]*)>([\s\S]*?)<\/a>/gi)) {
    const href = m[1].match(/(?:^|\s)href\s*=\s*(?:"([^"]*)"|'([^']*)')/i);
    const url = href && forumUploadUrl(decodeEntities(href[1] ?? href[2]));
    if (!url || seen.has(url)) continue;
    const urlName = urlFileName(url);
    // A link to an image or other file is not a script, whatever its text says.
    if (!SCRIPT_FILE_RE.test(urlName) && /\.[a-z0-9]+$/i.test(urlName)) continue;
    const text = decodeEntities(m[2].replace(/<[^>]*>/g, '')).trim();
    const name = SCRIPT_FILE_RE.test(text) ? text : urlName;
    if (!SCRIPT_FILE_RE.test(name)) continue;
    seen.add(url);
    attachments.push({ name, url });
  }
  for (const m of html.matchAll(PIXELDRAIN_RE)) pixeldrain.push({ kind: m[1].toLowerCase(), id: m[2] });
  for (const m of html.matchAll(/href="(https?:\/\/[^"]+)"/gi)) {
    const url = decodeEntities(m[1]);
    let host;
    try { host = new URL(url).hostname.replace(/^www\./, ''); } catch { continue; }
    const known = EXTERNAL_HOSTS.find(([h]) => host === h || host.endsWith(`.${h}`));
    if (known) external.push({ host: known[1], url });
  }
  return { attachments, pixeldrain, external };
}

class EroScriptsAPI {
  constructor() {
    this._cookie = null;
    this._username = null;
    this._sessionCookies = '';
  }

  get isLoggedIn() {
    return !!this._cookie;
  }

  get username() {
    return this._username;
  }

  restoreSession(cookie, username) {
    this._cookie = cookie;
    this._username = username;
    this._sessionCookies = cookie ? `_t=${cookie}` : '';
    _debugLog(`SESSION restored for ${username}, cookie length: ${(cookie || '').length}`);
  }

  /**
   * Validate the current session is still active.
   * Makes a lightweight request to check if authenticated.
   * @returns {{ valid: boolean }}
   */
  async validateSession() {
    if (!this._cookie) return { valid: false };

    try {
      // Use the session/current endpoint — lightweight, returns user info if logged in
      const resp = await fetch(`${BASE_URL}/session/current.json`, {
        headers: this._headers(),
      });
      _debugLog(`VALIDATE session status: ${resp.status}`);

      if (resp.status === 403 || resp.status === 401) {
        return { valid: false };
      }
      if (!resp.ok) {
        // Server error (503 etc.) — don't invalidate on transient failures
        return { valid: true };
      }

      const data = await this._safeJson(resp);
      if (!data) {
        // HTML response (Cloudflare) — session likely expired
        return { valid: false };
      }

      // If we get user data back, session is valid
      return { valid: !!data.current_user };
    } catch {
      // Network error — assume valid, don't log out on connectivity issues
      return { valid: true };
    }
  }

  logout() {
    this._cookie = null;
    this._username = null;
    this._sessionCookies = '';
  }

  /**
   * Search for funscript topics.
   */
  async search(query, page = 1) {
    try {
      const params = new URLSearchParams({
        q: `${query} #scripts`,
        page: String(page),
      });

      const resp = await fetch(`${BASE_URL}/search.json?${params}`, {
        headers: this._headers(),
      });

      log.info(`[EroScripts] Search status: ${resp.status} for query: ${query}`);
      _debugLog(`SEARCH query="${query}" status=${resp.status}`);
      _debugLog(`SEARCH cookies sent: ${(this._headers().Cookie || '').length} bytes`);
      if (resp.status !== 200) {
        const bodyPreview = await resp.clone().text();
        _debugLog(`SEARCH error body: ${bodyPreview.substring(0, 500)}`);
      }

      if (resp.status === 429) {
        return { results: [], error: 'Rate limited — try again in a moment', rateLimited: true };
      }
      if (resp.status === 403 || resp.status === 401) {
        // `authExpired` is the machine-readable half of this. The message
        // alone forced every caller to string-match, and the auto-match
        // path simply ignored it — so a dead session looked exactly like
        // "this video has no script", silently, for weeks.
        return { results: [], error: 'Access denied — try logging in again', authExpired: true };
      }

      const data = await this._safeJson(resp);
      if (!data) {
        return { results: [], error: `Search failed (${resp.status}) — try logging out and back in` };
      }

      // Log structure for debugging
      log.info('[EroScripts] Search response: topics=%d, posts=%d, users=%d',
        (data.topics || []).length, (data.posts || []).length, (data.users || []).length);
      if (data.topics?.length > 0) {
        const t = data.topics[0];
        // Field list and tags called out on their own lines. The truncated
        // dump below used to cut at 500 chars, which lands before `tags` on
        // almost every topic — so "does search actually return tags?" was
        // unanswerable from the logs, despite the parser and the panel both
        // being wired for them.
        log.info('[EroScripts] Topic fields: %s', Object.keys(t).join(','));
        log.info('[EroScripts] Topic tags: %s', JSON.stringify(t.tags ?? null));
        log.info('[EroScripts] Sample topic:', JSON.stringify(t).substring(0, 1500));
      }
      if (data.posts?.length > 0) {
        const p = data.posts[0];
        log.info('[EroScripts] Sample post:', JSON.stringify(p).substring(0, 500));
      }

      const users = new Map((data.users || []).map(u => [u.id, u]));
      const topicsMap = new Map((data.topics || []).map(t => [t.id, t]));
      const posts = data.posts || [];

      // Build results from whichever is available — topics or posts
      let topics;

      if (data.topics && data.topics.length > 0) {
        // Topics available (some Discourse versions return these)
        topics = data.topics.map(t => {
          const tags = (t.tags || []).map(tag => typeof tag === 'string' ? tag : (tag.name || tag.text || String(tag)));

          const post = posts.find(p => p.topic_id === t.id);
          const user = post ? users.get(post.user_id) : null;

          // Search topics carry `thumbnails[]` (several sizes), NOT the
          // `image_url` this used to read — that field only exists on the
          // topic endpoint, so every search result came back with a null
          // thumbnail and the panel paid for a second request per row to
          // fetch one. Prefer an entry near the rendered size.
          let thumbnail = pickThumbnail(t.thumbnails) || t.image_url || null;
          if (thumbnail && !thumbnail.startsWith('http')) thumbnail = `${BASE_URL}${thumbnail}`;
          // The same picture at poster size, for a video's library
          // thumbnail; null when it's too small (getTopicImage decides then).
          const preview = searchPreviewUrl(t.thumbnails);

          let avatar = post?.avatar_template || null;
          if (avatar) {
            avatar = avatar.replace('{size}', '90');
            if (!avatar.startsWith('http')) avatar = `${BASE_URL}${avatar}`;
          }

          return {
            id: t.id, title: t.title, slug: t.slug,
            categoryId: t.category_id ?? null,
            createdAt: t.created_at,
            // Like count lives on the POST in a search response, not the
            // topic — `t.like_count` doesn't exist there, so this read 0 for
            // every result and the panel (which hides a 0) showed nothing.
            // The post was already being looked up two lines above for the
            // avatar.
            likeCount: post?.like_count ?? t.like_count ?? 0,
            // `views` genuinely isn't in the search serializer — not on the
            // topic and not on the post. It only exists on /t/{id}.json.
            // Left at 0 rather than faked; the panel hides a 0 view count.
            views: t.views || 0,
            tags, thumbnail, preview, avatar,
            creator: post?.username || user?.username || null,
            url: `${BASE_URL}/t/${t.slug || t.id}/${t.id}`,
          };
        });
      } else {
        // No topics — build from posts (EroScripts returns results this way)
        const seen = new Set();
        topics = [];

        for (const post of posts) {
          if (seen.has(post.topic_id)) continue;
          seen.add(post.topic_id);

          const rawTopic = topicsMap.get(post.topic_id);
          const user = users.get(post.user_id);

          // Extract thumbnail from post blurb HTML
          let thumbnail = rawTopic?.image_url || null;
          if (!thumbnail && post.blurb) {
            const imgMatch = post.blurb.match(/<img[^>]+src="([^"]+)"/i);
            if (imgMatch) {
              thumbnail = imgMatch[1];
            }
          }
          if (thumbnail && !thumbnail.startsWith('http')) thumbnail = `${BASE_URL}${thumbnail}`;

          const tags = rawTopic?.tags || [];
          const normTags = tags.map(tag => typeof tag === 'string' ? tag : (tag.name || tag.text || String(tag)));

          // Build avatar URL from template
          let avatar = post.avatar_template || null;
          if (avatar) {
            avatar = avatar.replace('{size}', '90');
            if (!avatar.startsWith('http')) avatar = `${BASE_URL}${avatar}`;
          }

          topics.push({
            id: post.topic_id,
            categoryId: rawTopic?.category_id ?? null,
            title: rawTopic?.title || post.name || post.topic?.title || `Topic ${post.topic_id}`,
            slug: rawTopic?.slug || null,
            createdAt: post.created_at,
            likeCount: post.like_count || rawTopic?.like_count || 0,
            views: rawTopic?.views || 0,
            tags: normTags,
            thumbnail,
            preview: null,
            avatar,
            creator: post.username || user?.username || null,
            url: `${BASE_URL}/t/${rawTopic?.slug || post.topic_id}/${post.topic_id}`,
          });
        }
      }

      const slugs = await this._categorySlugs();
      for (const topic of topics) {
        topic.category = slugs.get(topic.categoryId) || null;
        topic.paid = topic.category === PAID_SLUG;
      }
      return { results: topics };
    } catch (err) {
      log.error('[EroScripts] Search error:', err.message);
      return { results: [], error: err.message };
    }
  }

  /**
   * Script files in a topic: attachments on every post (the author's
   * first, in post order, then other users'). Funscripts the posts link on
   * pixeldrain come back in `linked` (name + page URL, opened in the
   * browser: pixeldrain refuses programmatic downloads). Links to other
   * hosts FunSync can't fetch from come back in `external` so the UI can
   * name them; `paid` marks the Paid Scripts category, whose posts link to
   * a store instead of attaching files.
   *
   * Each attachment / linked entry: {name, url, host, postNumber, username, byAuthor}.
   * `preview` is the topic's poster image URL (see topicPreviewUrl), or null.
   */
  async getTopicAttachments(topicId) {
    try {
      const resp = await fetch(`${BASE_URL}/t/${topicId}.json`, {
        headers: this._headers(),
      });

      if (resp.status === 429) {
        return { attachments: [], error: 'Rate limited', rateLimited: true };
      }
      if (resp.status === 403 || resp.status === 401) {
        return { attachments: [], error: 'Access denied — try logging in again', authExpired: true };
      }

      const data = await this._safeJson(resp);
      if (!data) {
        return { attachments: [], error: `Failed to load topic (${resp.status})` };
      }

      const posts = await this._allPosts(topicId, data.post_stream);
      if (posts.length === 0) {
        return { attachments: [], error: 'No posts found' };
      }

      const author = posts[0].username;
      const attachments = [];
      const seenUrls = new Set();
      const pixeldrain = [];
      const external = new Map();

      for (const post of posts) {
        const from = { postNumber: post.post_number, username: post.username, byAuthor: post.username === author };
        const links = parsePostLinks(post.cooked || '');
        for (const a of links.attachments) {
          if (seenUrls.has(a.url)) continue;
          seenUrls.add(a.url);
          attachments.push({ ...a, host: 'eroscripts', ...from });
        }
        for (const p of links.pixeldrain) pixeldrain.push({ ...p, from });
        for (const e of links.external) if (!external.has(e.url)) external.set(e.url, e);
      }

      const linked = await this._resolvePixeldrain(pixeldrain);

      // Stable sort: the author's files first, each side in post order.
      const authorFirst = (a, b) => (Number(b.byAuthor) - Number(a.byAuthor)) || (a.postNumber - b.postNumber);
      attachments.sort(authorFirst);
      linked.sort(authorFirst);

      const slugs = await this._categorySlugs();
      return {
        attachments,
        linked,
        external: [...external.values()],
        paid: slugs.get(data.category_id) === PAID_SLUG,
        // The topic's poster-size image, as getTopicImage picks it.
        preview: topicPreviewUrl(data),
      };
    } catch (err) {
      log.error('[EroScripts] Topic error:', err.message);
      return { attachments: [], error: err.message };
    }
  }

  /** First page from the topic payload, then up to MAX_POSTS in batches. */
  async _allPosts(topicId, stream) {
    const posts = [...(stream?.posts || [])];
    const have = new Set(posts.map((p) => p.id));
    const missing = (stream?.stream || []).filter((id) => !have.has(id)).slice(0, Math.max(0, MAX_POSTS - posts.length));
    for (let i = 0; i < missing.length; i += POST_BATCH) {
      const ids = missing.slice(i, i + POST_BATCH).map((id) => `post_ids[]=${id}`).join('&');
      try {
        const resp = await fetch(`${BASE_URL}/t/${topicId}/posts.json?${ids}`, { headers: this._headers() });
        const page = resp.ok ? await this._safeJson(resp) : null;
        posts.push(...(page?.post_stream?.posts || []));
      } catch { /* a missing batch only hides replies */ }
    }
    return posts.sort((a, b) => a.post_number - b.post_number);
  }

  /**
   * Funscripts behind pixeldrain file (`/u/`) and list (`/l/`) links, via
   * pixeldrain's public listing API, as `{name, url}` where `url` is the
   * file's pixeldrain page. No forum cookie goes to pixeldrain.
   */
  async _resolvePixeldrain(links) {
    const found = [];
    const seen = new Set();
    const unique = links.filter((l) => !seen.has(`${l.kind}:${l.id}`) && seen.add(`${l.kind}:${l.id}`));
    for (const link of unique.slice(0, MAX_PIXELDRAIN_LOOKUPS)) {
      try {
        const api = link.kind === 'l'
          ? `https://pixeldrain.com/api/list/${link.id}`
          : `https://pixeldrain.com/api/file/${link.id}/info`;
        const resp = await fetch(api, { headers: { 'User-Agent': USER_AGENT }, signal: AbortSignal.timeout(10_000) });
        if (!resp.ok) continue;
        const info = await resp.json();
        const files = link.kind === 'l' ? (info.files || []) : [info];
        for (const f of files) {
          if (!f?.id || typeof f.name !== 'string' || !/\.funscript$/i.test(f.name)) continue;
          if (found.some((x) => x.id === f.id)) continue;
          found.push({ id: f.id, name: f.name, url: `https://pixeldrain.com/u/${f.id}`, host: 'pixeldrain', ...link.from });
        }
      } catch { /* host down or link dead — the post's other files still show */ }
    }
    return found.map(({ id, ...rest }) => rest);
  }

  /** Category id → slug, fetched once per session. Empty map on failure. */
  async _categorySlugs() {
    if (this._categories) return this._categories;
    const map = new Map();
    try {
      const resp = await fetch(`${BASE_URL}/categories.json?include_subcategories=true`, { headers: this._headers() });
      const data = resp.ok ? await this._safeJson(resp) : null;
      for (const c of data?.category_list?.categories || []) {
        map.set(c.id, c.slug);
        for (const sub of c.subcategory_list || []) map.set(sub.id, sub.slug);
      }
      if (map.size > 0) this._categories = map;
    } catch { /* retried on the next call */ }
    return map;
  }

  /**
   * A topic's `/t/{id}.json` payload, or null. Kept for TOPIC_CACHE_MS:
   * a creator bundle links dozens of videos to one topic, and the preview
   * batch asks for it once per video.
   */
  async getTopicData(topicId) {
    const key = String(topicId);
    const hit = this._topicCache?.get(key);
    if (hit && Date.now() - hit.at < TOPIC_CACHE_MS) return hit.data;
    try {
      const resp = await fetch(`${BASE_URL}/t/${encodeURIComponent(key)}.json`, {
        headers: this._headers(),
      });
      const data = await this._safeJson(resp);
      if (!data) return null;
      this._topicCache ||= new Map();
      if (this._topicCache.size >= TOPIC_CACHE_SIZE) this._topicCache.delete(this._topicCache.keys().next().value);
      this._topicCache.set(key, { at: Date.now(), data });
      return data;
    } catch {
      return null;
    }
  }

  /**
   * The topic's preview image URL, or null. See topicPreviewUrl.
   */
  async getTopicImage(topicId) {
    const data = await this.getTopicData(topicId);
    return data ? topicPreviewUrl(data) : null;
  }

  /**
   * Every image of the topic that could show the video, and the ones
   * ruled out (see preview-match.js collectPreviewCandidates), or null when
   * the topic can't be loaded. `extraUrls` (a search result's preview, a
   * URL the sync CLI recorded) go through the same rules.
   */
  async getPreviewCandidates(topicId, extraUrls = []) {
    const data = await this.getTopicData(topicId);
    return data ? collectPreviewCandidates(data, { extraUrls }) : null;
  }

  /**
   * Download a post's preview image into memory, to become a video's
   * library thumbnail.
   *
   * https only, and only from the forum or its upload CDN. The session
   * cookie goes to the forum alone: CDN images are public. Redirects are
   * followed by hand so every hop passes the same checks. Refuses anything
   * that isn't an image or is larger than PREVIEW_MAX_BYTES.
   *
   * @param {string} url
   * @returns {Promise<{success: true, bytes: Buffer, contentType: string}
   *   | {success: false, error: string}>}
   */
  async downloadPreviewImage(url) {
    let current = url;
    try {
      for (let hop = 0; hop <= PREVIEW_MAX_REDIRECTS; hop++) {
        let u;
        try { u = new URL(current); } catch { return { success: false, error: 'Unsupported image link' }; }
        if (u.protocol !== 'https:' || !PREVIEW_HOSTS.has(u.hostname)) {
          return { success: false, error: 'Unsupported image link' };
        }
        const headers = { 'User-Agent': USER_AGENT, 'Accept': 'image/*' };
        if (u.hostname === FORUM_HOST) {
          const cookie = this._headers().Cookie;
          if (cookie) headers.Cookie = cookie;
        }
        const resp = await fetch(u.href, {
          headers, redirect: 'manual', signal: AbortSignal.timeout(PREVIEW_TIMEOUT_MS),
        });
        if (resp.status >= 300 && resp.status < 400) {
          const location = resp.headers.get('location');
          if (!location) return { success: false, error: `Image download failed (${resp.status})` };
          current = new URL(location, u).href;
          continue;
        }
        if (!resp.ok) return { success: false, error: `Image download failed (${resp.status})` };

        const contentType = (resp.headers.get('content-type') || '').split(';')[0].trim().toLowerCase();
        if (!contentType.startsWith('image/')) return { success: false, error: 'Not an image' };
        const declared = Number(resp.headers.get('content-length'));
        if (declared > PREVIEW_MAX_BYTES) return { success: false, error: 'Image too large' };

        const bytes = await readCapped(resp, PREVIEW_MAX_BYTES);
        if (!bytes) return { success: false, error: 'Image too large' };
        return { success: true, bytes, contentType };
      }
      return { success: false, error: 'Too many redirects' };
    } catch (err) {
      log.warn('[EroScripts] Preview download error:', err.message);
      return { success: false, error: err.message };
    }
  }

  /**
   * Download a file from a URL.
   *
   * Only https links on the forum itself are fetched, since the session
   * cookie rides along. Refuses to replace an existing file unless
   * `overwrite` is set, and reports that as `{exists: true}` so the caller
   * can ask the user. The check is the open itself (`wx`), so nothing can
   * slip in between.
   */
  async downloadFile(url, savePath, { overwrite = false } = {}) {
    const fs = require('fs');
    let host;
    try {
      const u = new URL(url);
      if (u.protocol === 'https:') host = u.hostname;
    } catch { /* rejected below */ }
    if (!host || !DOWNLOAD_HOSTS.has(host)) {
      return { success: false, error: 'Unsupported download link' };
    }
    try {
      const resp = await fetch(url, { headers: this._headers() });
      if (!resp.ok) {
        return { success: false, error: `Download failed (${resp.status})` };
      }

      const buffer = Buffer.from(await resp.arrayBuffer());
      fs.writeFileSync(savePath, buffer, { flag: overwrite ? 'w' : 'wx' });
      log.info(`[EroScripts] Downloaded: ${savePath}`);
      return { success: true };
    } catch (err) {
      if (err.code === 'EEXIST') return { success: false, exists: true };
      log.error('[EroScripts] Download error:', err.message);
      return { success: false, error: err.message };
    }
  }

  /**
   * Fetch a script file (`.funscript` / `.zip`) into memory, to compare it
   * with a local one. Same host rule as `downloadFile`: https forum links
   * only. The forum redirects uploads to its CDN; redirects are followed by
   * hand so the session cookie goes to the forum alone.
   *
   * @param {string} url
   * @param {{maxBytes?: number}} [opts]
   * @returns {Promise<{success: true, bytes: Buffer} | {success: false,
   *   error: string, status?: number, rateLimited?: boolean, authExpired?: boolean}>}
   */
  async fetchScript(url, { maxBytes = 25 * 1024 * 1024 } = {}) {
    let current = url;
    try {
      for (let hop = 0; hop <= 3; hop++) {
        let u;
        try { u = new URL(current); } catch { return { success: false, error: 'Unsupported download link' }; }
        const onForum = u.hostname === FORUM_HOST;
        // The first hop must be the forum; only its redirects may reach the CDN.
        if (u.protocol !== 'https:' || !(onForum || (hop > 0 && u.hostname === UPLOAD_CDN_HOST))) {
          return { success: false, error: 'Unsupported download link' };
        }
        const headers = onForum ? this._headers() : { 'User-Agent': USER_AGENT };
        const resp = await fetch(u.href, { headers, redirect: 'manual', signal: AbortSignal.timeout(60_000) });
        if (resp.status >= 300 && resp.status < 400) {
          const location = resp.headers.get('location');
          if (!location) return { success: false, error: `Download failed (${resp.status})`, status: resp.status };
          current = new URL(location, u).href;
          continue;
        }
        if (resp.status === 429) return { success: false, error: 'Rate limited', status: 429, rateLimited: true };
        if (onForum && (resp.status === 401 || resp.status === 403)) {
          return { success: false, error: 'Access denied — try logging in again', status: resp.status, authExpired: true };
        }
        if (!resp.ok) return { success: false, error: `Download failed (${resp.status})`, status: resp.status };
        if (Number(resp.headers.get('content-length')) > maxBytes) return { success: false, error: 'File too large' };
        const bytes = await readCapped(resp, maxBytes);
        if (!bytes) return { success: false, error: 'File too large' };
        return { success: true, bytes };
      }
      return { success: false, error: 'Too many redirects' };
    } catch (err) {
      return { success: false, error: err.message };
    }
  }

  // --- Internal ---

  _headers() {
    const h = { 'User-Agent': USER_AGENT, 'Accept': 'application/json' };
    // Send all session cookies (includes _t + Cloudflare cf_clearance etc.)
    if (this._sessionCookies) {
      h['Cookie'] = this._sessionCookies;
    } else if (this._cookie) {
      h['Cookie'] = `_t=${this._cookie}`;
    }
    return h;
  }

  async _safeJson(resp) {
    try {
      const text = await resp.text();
      // Guard against HTML responses (DOCTYPE, login redirects, etc.)
      if (text.startsWith('<!') || text.startsWith('<html')) {
        log.warn('[EroScripts] Received HTML instead of JSON');
        _debugLog(`HTML response (${resp.status}): ${text.substring(0, 300)}`);
        return null;
      }
      return JSON.parse(text);
    } catch (err) {
      _debugLog(`JSON parse error: ${err.message}`);
      return null;
    }
  }

}

module.exports = { EroScriptsAPI, parsePostLinks, topicPreviewUrl };
