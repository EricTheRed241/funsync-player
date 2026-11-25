// Does an EroScripts post image actually show the video?
//
// "Fetch EroScripts previews" used to take a topic's thumbnail or the first
// big image of its first post. Plenty of posts lead with a creator banner,
// a Patreon button, a funscript heatmap or an avatar, and those became
// library posters. Now every image is a candidate that must earn its place:
//
// 1. collectPreviewCandidates() lists the topic thumbnail and every image in
//    the first post, minus the ones that can't be a picture of the video
//    (avatars, emoji, quotes, oneboxes, images linking off-site, wrong size
//    or aspect).
// 2. Each remaining image is compared with frames sampled across the video
//    (backend/services/ffmpeg.py sample_gray_frames) by perceptual hash:
//    a 64-bit DCT hash (pHash) per frame, plus the left eye for
//    side-by-side VR. An image is accepted only when some frame is within
//    MATCH_THRESHOLD bits of it.
//
// Pure functions only (no Electron, no I/O): the main process, the dry-run
// CLI (scripts/eroscripts-preview-check.mjs) and the tests share them.

// ---- Candidate rules ----

/** Shortest acceptable longest side, in px. Smaller makes a blurry tile. */
const MIN_LONG_SIDE = 320;
/** Width / height range of a poster. Below: portraits, avatars, square
 *  logos. Above: banners, button strips and funscript heatmaps. */
const MIN_ASPECT = 1.2;
const MAX_ASPECT = 3.2;
/** Most images considered per topic (each one is a download). */
const MAX_CANDIDATES = 8;

const FORUM_HOST = 'discuss.eroscripts.com';
const UPLOAD_CDN_HOST = 'eroscripts-discourse.eroscripts.com';
const IMAGE_HOSTS = new Set([FORUM_HOST, UPLOAD_CDN_HOST]);

// Hosts creators link their banners to: an image wrapped in a link to one
// of these is an ad for the creator, never a frame of the video. Any other
// off-site link around an image counts as a banner too; these only name it.
const PROMO_HOSTS = [
  'patreon.com', 'ko-fi.com', 'buymeacoffee.com', 'gumroad.com', 'subscribestar.com',
  'subscribestar.adult', 'fansly.com', 'onlyfans.com', 'fanbox.cc', 'boosty.to',
  'itch.io', 'linktr.ee', 'discord.gg', 'discord.com', 'twitter.com', 'x.com',
  'throne.com', 'payhip.com', 'sellfy.com', 'ko-fi.me',
];

// Elements whose images are never the post's own content: quoted posts
// (with the quoted user's avatar), link previews of other sites, @mentions
// and user cards.
const FOREIGN_CONTAINER_CLASSES = ['quote', 'onebox', 'mention', 'user-card', 'badge'];
// Image classes that mark site chrome rather than uploaded content.
const CHROME_IMG_CLASSES = ['emoji', 'avatar', 'site-icon', 'onebox-avatar', 'badge'];

const _ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", '#39': "'", '#x27': "'" };
function decodeEntities(text) {
  return String(text).replace(/&(#x?[0-9a-f]+|[a-z]+);/gi, (whole, body) => {
    const key = body.toLowerCase();
    if (Object.prototype.hasOwnProperty.call(_ENTITIES, key)) return _ENTITIES[key];
    if (key.startsWith('#x')) return String.fromCodePoint(parseInt(key.slice(2), 16));
    if (key.startsWith('#')) return String.fromCodePoint(parseInt(key.slice(1), 10));
    return whole;
  });
}

function attr(attrs, name) {
  const m = attrs.match(new RegExp(`(?:^|\\s)${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i'));
  return m ? decodeEntities(m[1] ?? m[2] ?? m[3]) : null;
}

function classes(attrs) {
  return (attr(attrs, 'class') || '').toLowerCase().split(/\s+/).filter(Boolean);
}

function absoluteUrl(url) {
  if (!url) return null;
  if (url.startsWith('//')) return `https:${url}`;
  return url.startsWith('http') ? url : `https://${FORUM_HOST}${url.startsWith('/') ? '' : '/'}${url}`;
}

function hostOf(url) {
  try { return new URL(url).hostname.toLowerCase().replace(/^www\./, ''); } catch { return null; }
}

function promoHost(host) {
  return PROMO_HOSTS.find((h) => host === h || host.endsWith(`.${h}`)) || null;
}

/**
 * Why `url` can't be a picture of the video, judged by the URL alone, or
 * null when it may be one.
 */
function urlExclusion(url) {
  let u;
  try { u = new URL(url); } catch { return 'invalid url'; }
  const p = u.pathname.toLowerCase();
  if (p.includes('/images/emoji/')) return 'emoji';
  if (p.includes('/user_avatar/') || p.includes('/letter_avatar') || p.includes('avatar')) return 'avatar';
  if (!IMAGE_HOSTS.has(u.hostname.toLowerCase())) return 'not a forum upload';
  // Uploads live under /uploads/… on the forum and /original|/optimized/…
  // on the CDN; anything else (/images/, /plugins/, badges) is site chrome.
  if (!/^\/(uploads\/|original\/|optimized\/)/.test(p)) return 'site image';
  if (!/\.(jpe?g|png|gif|webp|avif)$/.test(p)) return 'not an image';
  return null;
}

/**
 * Why an image of `width`×`height` can't be a poster, or null. Unknown
 * dimensions (0) pass here and are checked again once decoded.
 */
function dimensionExclusion(width, height) {
  const w = Number(width) || 0;
  const h = Number(height) || 0;
  if (!w || !h) return null;
  if (Math.max(w, h) < MIN_LONG_SIDE) return 'too small';
  const aspect = w / h;
  if (aspect < MIN_ASPECT || aspect > MAX_ASPECT) return 'wrong aspect';
  return null;
}

// Discourse names derivatives after the original's SHA-1
// (`…/original/4X/9/7/c/<sha1>.webp`, `…/optimized/4X/9/7/c/<sha1>_2_690x388.webp`),
// so the topic thumbnail and the post image it was made from share a key.
function uploadKey(url) {
  const m = String(url).match(/\/([0-9a-f]{40})(?:_[^/]*)?\.[a-z0-9]+(?:\?|$)/i);
  return m ? m[1].toLowerCase() : url;
}

/**
 * Every image of a topic that could be a frame of the video.
 *
 * @param {object} data `/t/{id}.json` payload
 * @param {{extraUrls?: string[]}} [opts] image URLs known from elsewhere
 *   (a search result's preview, a link recorded by the sync CLI); they go
 *   through the same URL rules
 * @returns {{candidates: Array<{url: string, origin: string, width: number, height: number}>,
 *   excluded: Array<{url: string, origin: string, reason: string}>}}
 */
function collectPreviewCandidates(data, { extraUrls = [] } = {}) {
  const candidates = [];
  const excluded = [];
  const seen = new Set();

  const consider = (rawUrl, origin, width = 0, height = 0, context = null) => {
    const url = absoluteUrl(rawUrl);
    if (!url) return;
    const key = uploadKey(url);
    if (seen.has(key)) return;
    seen.add(key);
    const reason = context || urlExclusion(url) || dimensionExclusion(width, height);
    if (reason) excluded.push({ url, origin, reason });
    else candidates.push({ url, origin, width: Number(width) || 0, height: Number(height) || 0 });
  };

  for (const url of extraUrls) if (typeof url === 'string' && url) consider(url, 'known');

  // The topic's representative image at poster size, with the original's
  // dimensions for the aspect check.
  const thumbs = Array.isArray(data?.thumbnails) ? data.thumbnails.filter((x) => x?.url) : [];
  if (thumbs.length) {
    const original = thumbs.reduce((a, b) => ((Number(b.width) || 0) > (Number(a.width) || 0) ? b : a));
    const sized = thumbs
      .filter((x) => Number(x.width) >= 640)
      .sort((a, b) => Number(a.width) - Number(b.width))[0] || original;
    consider(sized.url, 'thumbnail', original.width, original.height);
  } else if (data?.image_url) {
    consider(data.image_url, 'thumbnail');
  }

  // First post, in document order. Track the open <aside>/<a>/<div>/<span>
  // elements so an image knows what it sits in.
  const html = data?.post_stream?.posts?.[0]?.cooked || '';
  const stack = [];
  for (const m of html.matchAll(/<(\/?)([a-z][a-z0-9]*)\b([^>]*)>/gi)) {
    const closing = m[1] === '/';
    const tag = m[2].toLowerCase();
    const attrs = m[3] || '';
    if (tag === 'aside' || tag === 'a' || tag === 'div' || tag === 'span') {
      if (closing) {
        const at = stack.map((e) => e.tag).lastIndexOf(tag);
        if (at >= 0) stack.length = at;
      } else if (!attrs.trim().endsWith('/')) {
        stack.push({ tag, cls: classes(attrs), href: tag === 'a' ? attr(attrs, 'href') : null });
      }
      continue;
    }
    if (tag !== 'img' || closing) continue;
    const src = attr(attrs, 'src');
    if (!src) continue;
    const imgClasses = classes(attrs);
    let context = null;
    const chrome = CHROME_IMG_CLASSES.find((c) => imgClasses.includes(c));
    if (chrome) context = chrome === 'emoji' ? 'emoji' : chrome === 'badge' ? 'site image' : 'avatar';
    for (const el of stack) {
      if (context) break;
      if (el.cls.some((c) => FOREIGN_CONTAINER_CLASSES.includes(c))) {
        context = el.cls.includes('quote') ? 'quoted post' : el.cls.includes('onebox') ? 'link preview' : 'avatar';
      } else if (el.tag === 'a' && el.href) {
        const host = hostOf(absoluteUrl(el.href));
        if (host && !IMAGE_HOSTS.has(host)) context = promoHost(host) ? `banner (${promoHost(host)})` : `banner (${host})`;
      }
    }
    consider(src, 'post', attr(attrs, 'width'), attr(attrs, 'height'), context);
  }

  return { candidates: candidates.slice(0, MAX_CANDIDATES), excluded };
}

// ---- Perceptual hashing ----

/** Bits (of 64) two pHashes may differ by and still be the same picture.
 *
 *  Calibrated on a real library (133 posters of the PR #14 batch, their
 *  videos sampled by keyframe, every poster checked by eye): every poster
 *  at distance ≤ 12 to its own video was a frame of it; 14 and above were
 *  mostly banners, title cards and other videos' images. Against the
 *  other creators' videos (impostors) 0.13% of pairs reach ≤ 12, 1.2%
 *  ≤ 14, 4.5% ≤ 16. dHash separated far worse (4% impostors at 12). */
const MATCH_THRESHOLD = 12;
/** A 32×32 thumbnail whose luma standard deviation is below this is too
 *  flat (black, fades, solid fills) for its hash to mean anything. */
const MIN_CONTRAST = 8;
/** Frames at least this wide relative to their height are taken for
 *  side-by-side VR and also compared by their left eye. */
const VR_MIN_ASPECT = 1.8;
/** pHash works on a DCT of a HASH_SIZE² thumbnail, keeping 8×8 coefficients. */
const HASH_SIZE = 32;

/**
 * RGBA (or BGRA) pixels → 8-bit luma.
 * @param {Uint8Array|Buffer} pixels
 * @param {number} width
 * @param {number} height
 * @param {'rgba'|'bgra'} [order]
 */
function toGray(pixels, width, height, order = 'rgba') {
  const out = new Uint8Array(width * height);
  const [r, b] = order === 'bgra' ? [2, 0] : [0, 2];
  for (let i = 0, j = 0; i < out.length; i++, j += 4) {
    out[i] = (pixels[j + r] * 299 + pixels[j + 1] * 587 + pixels[j + b] * 114) / 1000;
  }
  return out;
}

/**
 * Box-filter `gray` (w×h, optionally a sub-rectangle) down to tw×th, with
 * fractional pixel coverage so any source size maps evenly.
 */
function areaResize(gray, w, h, tw, th, rect = { x: 0, y: 0, w, h }) {
  const out = new Float64Array(tw * th);
  const sx = rect.w / tw;
  const sy = rect.h / th;
  for (let ty = 0; ty < th; ty++) {
    const y0 = rect.y + ty * sy;
    const y1 = y0 + sy;
    for (let tx = 0; tx < tw; tx++) {
      const x0 = rect.x + tx * sx;
      const x1 = x0 + sx;
      let sum = 0;
      let area = 0;
      for (let y = Math.floor(y0); y < Math.min(Math.ceil(y1), h); y++) {
        const wy = Math.min(y + 1, y1) - Math.max(y, y0);
        if (wy <= 0) continue;
        for (let x = Math.floor(x0); x < Math.min(Math.ceil(x1), w); x++) {
          const wx = Math.min(x + 1, x1) - Math.max(x, x0);
          if (wx <= 0) continue;
          sum += gray[y * w + x] * wx * wy;
          area += wx * wy;
        }
      }
      out[ty * tw + tx] = area ? sum / area : 0;
    }
  }
  return out;
}

// cos((2x+1)uπ / 2N) for the 8 low frequencies.
const DCT_COS = Array.from({ length: 8 }, (_, u) =>
  Float64Array.from({ length: HASH_SIZE }, (_, x) => Math.cos(((2 * x + 1) * u * Math.PI) / (2 * HASH_SIZE))));

/**
 * 64-bit perceptual hash (pHash) of a grayscale image or a rectangle of it,
 * as 16 hex digits, plus the thumbnail's contrast. The image is shrunk to
 * 32×32; each bit says whether one of the 8×8 lowest DCT frequencies is
 * above the median of the 63 non-DC ones.
 *
 * @returns {{hash: string, contrast: number}}
 */
function pHash(gray, w, h, rect) {
  const N = HASH_SIZE;
  const s = areaResize(gray, w, h, N, N, rect);
  // Separable 2-D DCT-II, low 8×8 only: rows first, then columns.
  const rows = new Float64Array(N * 8);
  for (let y = 0; y < N; y++) {
    for (let u = 0; u < 8; u++) {
      let sum = 0;
      for (let x = 0; x < N; x++) sum += s[y * N + x] * DCT_COS[u][x];
      rows[y * 8 + u] = sum;
    }
  }
  const dct = new Float64Array(64);
  for (let v = 0; v < 8; v++) {
    for (let u = 0; u < 8; u++) {
      let sum = 0;
      for (let y = 0; y < N; y++) sum += rows[y * 8 + u] * DCT_COS[v][y];
      dct[v * 8 + u] = sum;
    }
  }
  const median = Array.from(dct.subarray(1)).sort((a, b) => a - b)[31];
  let hex = '';
  for (let i = 0; i < 64; i += 8) {
    let byte = 0;
    for (let j = 0; j < 8; j++) byte = (byte << 1) | (dct[i + j] > median ? 1 : 0);
    hex += byte.toString(16).padStart(2, '0');
  }
  let mean = 0;
  for (const v of s) mean += v;
  mean /= s.length;
  let variance = 0;
  for (const v of s) variance += (v - mean) ** 2;
  return { hash: hex, contrast: Math.sqrt(variance / s.length) };
}

const POPCOUNT8 = Uint8Array.from({ length: 256 }, (_, i) => {
  let n = 0;
  for (let v = i; v; v >>= 1) n += v & 1;
  return n;
});

/** Number of differing bits between two hex hashes of equal length. */
function hamming(a, b) {
  let d = 0;
  for (let i = 0; i < a.length; i += 2) {
    d += POPCOUNT8[parseInt(a.slice(i, i + 2), 16) ^ parseInt(b.slice(i, i + 2), 16)];
  }
  return d;
}

/**
 * Hashes of one video frame: the whole frame, plus its left half when the
 * frame looks like side-by-side VR (a poster of a VR video is usually one
 * eye). Flat frames yield nothing.
 *
 * @returns {Array<{view: 'full'|'left', hash: string}>}
 */
function frameHashes(gray, w, h) {
  const out = [];
  const full = pHash(gray, w, h);
  if (full.contrast >= MIN_CONTRAST) out.push({ view: 'full', hash: full.hash });
  if (w / h >= VR_MIN_ASPECT) {
    const left = pHash(gray, w, h, { x: 0, y: 0, w: w / 2, h });
    if (left.contrast >= MIN_CONTRAST) out.push({ view: 'left', hash: left.hash });
  }
  return out;
}

/**
 * Hash every sampled frame of a video.
 *
 * @param {{width: number, height: number, frames: Array<{pct: number, gray: Uint8Array}>}} sample
 * @returns {Array<{pct: number, view: string, hash: string}>}
 */
function videoHashes(sample) {
  const out = [];
  for (const f of sample?.frames || []) {
    for (const h of frameHashes(f.gray, sample.width, sample.height)) out.push({ pct: f.pct, ...h });
  }
  return out;
}

/**
 * Hash a candidate image (each decoded frame of it, for animations).
 *
 * @param {Array<{gray: Uint8Array, width: number, height: number}>} frames
 * @returns {Array<{frame: number, hash: string}>} flat frames left out
 */
function imageHashes(frames) {
  const out = [];
  frames.forEach((f, i) => {
    const { hash, contrast } = pHash(f.gray, f.width, f.height);
    if (contrast >= MIN_CONTRAST) out.push({ frame: i, hash });
  });
  return out;
}

/**
 * Closest pair between a candidate's hashes and the video's.
 *
 * @returns {{distance: number, frame: number, pct: number, view: string}|null}
 *   null when either side has nothing to compare
 */
function bestMatch(imgHashes, vidHashes) {
  let best = null;
  for (const i of imgHashes) {
    for (const v of vidHashes) {
      const distance = hamming(i.hash, v.hash);
      if (!best || distance < best.distance) best = { distance, frame: i.frame, pct: v.pct, view: v.view };
    }
  }
  return best;
}

/** Is a match close enough to call the image a frame of the video? */
function isAccepted(match, threshold = MATCH_THRESHOLD) {
  return !!match && match.distance <= threshold;
}

/**
 * The accepted candidate with the smallest distance, or null.
 * @template {{match?: {distance: number}|null}} T
 * @param {T[]} scored
 * @returns {T|null}
 */
function pickBest(scored, threshold = MATCH_THRESHOLD) {
  let best = null;
  for (const c of scored) {
    if (!isAccepted(c.match, threshold)) continue;
    if (!best || c.match.distance < best.match.distance) best = c;
  }
  return best;
}

module.exports = {
  MIN_LONG_SIDE,
  MIN_ASPECT,
  MAX_ASPECT,
  MAX_CANDIDATES,
  MATCH_THRESHOLD,
  MIN_CONTRAST,
  VR_MIN_ASPECT,
  collectPreviewCandidates,
  urlExclusion,
  dimensionExclusion,
  toGray,
  areaResize,
  pHash,
  hamming,
  frameHashes,
  videoHashes,
  imageHashes,
  bestMatch,
  isAccepted,
  pickBest,
};
