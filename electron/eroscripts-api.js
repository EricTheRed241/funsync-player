// EroScripts API — Discourse REST API client for discuss.eroscripts.com
// Runs in main process (no CORS restrictions).

const log = require('./logger');

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
// rides on every download request.
const DOWNLOAD_HOSTS = new Set([new URL(BASE_URL).hostname]);

const SCRIPT_FILE_RE = /\.(funscript|zip)$/i;
const PIXELDRAIN_RE = /https?:\/\/(?:www\.)?pixeldrain\.(?:com|net)\/(u|l)\/([A-Za-z0-9]+)/gi;

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
 * @returns {string|null}
 */
const MIN_THUMB_WIDTH = 400;

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

function pickThumbnail(thumbnails) {
  if (!Array.isArray(thumbnails) || thumbnails.length === 0) return null;
  const usable = thumbnails.filter(x => x && typeof x.url === 'string' && x.url);
  if (usable.length === 0) return null;
  const big = usable
    .filter(x => Number(x.width) >= MIN_THUMB_WIDTH)
    .sort((a, b) => Number(a.width) - Number(b.width));
  return (big[0] || usable[0]).url;
}

/**
 * Script-relevant links in one post's `cooked` HTML: forum attachments
 * (`.funscript` / `.zip`), `upload://` short links, pixeldrain file and
 * list links, and links to hosts FunSync can't fetch from.
 */
function parsePostLinks(html) {
  const attachments = [];
  const shortUrls = [];
  const pixeldrain = [];
  const external = [];
  for (const m of html.matchAll(/<a[^>]*class="attachment"[^>]*href="([^"]+)"[^>]*>([^<]+)<\/a>/gi)) {
    const name = decodeEntities(m[2].trim());
    if (SCRIPT_FILE_RE.test(name)) attachments.push({ name, href: decodeEntities(m[1]) });
  }
  for (const m of html.matchAll(/href="(upload:\/\/[^"]+)"/gi)) shortUrls.push(m[1]);
  for (const m of html.matchAll(PIXELDRAIN_RE)) pixeldrain.push({ kind: m[1].toLowerCase(), id: m[2] });
  for (const m of html.matchAll(/href="(https?:\/\/[^"]+)"/gi)) {
    const url = decodeEntities(m[1]);
    let host;
    try { host = new URL(url).hostname.replace(/^www\./, ''); } catch { continue; }
    const known = EXTERNAL_HOSTS.find(([h]) => host === h || host.endsWith(`.${h}`));
    if (known) external.push({ host: known[1], url });
  }
  return { attachments, shortUrls, pixeldrain, external };
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
        return { results: [], error: 'Rate limited — try again in a moment' };
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
            tags, thumbnail, avatar,
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
   */
  async getTopicAttachments(topicId) {
    try {
      const resp = await fetch(`${BASE_URL}/t/${topicId}.json`, {
        headers: this._headers(),
      });

      if (resp.status === 429) {
        return { attachments: [], error: 'Rate limited' };
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
      const add = (a) => {
        if (!a.url || seenUrls.has(a.url)) return;
        seenUrls.add(a.url);
        attachments.push(a);
      };
      const shortUrls = [];
      const pixeldrain = [];
      const external = new Map();

      for (const post of posts) {
        const from = { postNumber: post.post_number, username: post.username, byAuthor: post.username === author };
        const links = parsePostLinks(post.cooked || '');
        for (const a of links.attachments) {
          add({ name: a.name, url: a.href.startsWith('http') ? a.href : `${BASE_URL}${a.href}`, host: 'eroscripts', ...from });
        }
        for (const short of links.shortUrls) shortUrls.push({ short, from });
        for (const p of links.pixeldrain) pixeldrain.push({ ...p, from });
        for (const e of links.external) if (!external.has(e.url)) external.set(e.url, e);
      }

      await this._resolveShortUrls(shortUrls, add);
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

  async _resolveShortUrls(shortUrls, add) {
    if (shortUrls.length === 0) return;
    try {
      const resp = await fetch(`${BASE_URL}/uploads/lookup-urls`, {
        method: 'POST',
        headers: { ...this._headers(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ short_urls: shortUrls.map((s) => s.short) }),
      });
      if (!resp.ok) return;
      const resolved = await resp.json();
      for (const item of resolved) {
        if (!item.url || !SCRIPT_FILE_RE.test(item.url)) continue;
        const from = shortUrls.find((s) => s.short === item.short_url)?.from || {};
        add({
          name: item.url.split('/').pop(),
          url: item.url.startsWith('http') ? item.url : `${BASE_URL}${item.url}`,
          host: 'eroscripts',
          ...from,
        });
      }
    } catch { /* ignore */ }
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
   * Fetch the first image URL from a topic's first post.
   * Used for lazy thumbnail loading in search results.
   */
  async getTopicImage(topicId) {
    try {
      const resp = await fetch(`${BASE_URL}/t/${topicId}.json`, {
        headers: this._headers(),
      });
      const data = await this._safeJson(resp);
      if (!data) return null;

      const html = data.post_stream?.posts?.[0]?.cooked || '';
      const imgMatch = html.match(/<img[^>]+src="([^"]+\.(jpg|jpeg|png|gif|webp)(\?[^"]*)?)"/i);
      if (imgMatch) {
        let url = imgMatch[1];
        if (!url.startsWith('http')) url = `${BASE_URL}${url}`;
        return url;
      }
      return null;
    } catch {
      return null;
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

module.exports = { EroScriptsAPI, parsePostLinks };
