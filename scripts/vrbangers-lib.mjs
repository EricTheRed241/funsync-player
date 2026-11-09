// VRBangers (vrbangers.com) scene pages: pure helpers for
// scripts/vrbangers-browser.mjs. No IO.
//
// A scene page `https://vrbangers.com/video/<slug>/` carries its data in
// `<script id="__NUXT_DATA__">`, a Nuxt "devalue" array (every object, array
// and string is an entry, objects point at entries by index). For a member the
// array also holds signed links to the downloads,
//   https://mov.vrbangers.com/vrb_<folder>/VRBANGERS_<folder>_<Q>_<projection>.mp4?secure=<token>,<expiry>
// both this scene's and ~700 other scenes' (banners, recommendations), so the
// links are picked by the scene's folder (`videoSettings.videoShortName`, which
// can differ from the slug: anal-virginity → anal_virginity_remastered).
// Trailers (`_trailer_`, trailers.vrbangers.com), MISC/ extras (the toy
// funscript, the 250p preview) are not downloads.

import { VR_MAX_FPS, VR_MAX_HEIGHT } from '../renderer/js/content-preferences.js';

export const SITE = 'https://vrbangers.com';
export const LOGIN_URL = `${SITE}/login/`;
export const DASHBOARD_PATH = '/main-dashboard/';
export const sceneUrl = (slug) => `${SITE}/video/${encodeURIComponent(slug)}/`;

/**
 * The credentials file (`vb_hack.txt`): lines `Username: <email>` and
 * `password: <pw>`, any other lines ignored. null when either is missing.
 */
export function parseCredentials(text) {
  const field = (name) => String(text || '').match(new RegExp(`^\\s*${name}\\s*:\\s*(\\S+)\\s*$`, 'im'))?.[1];
  const user = field('username');
  const password = field('password');
  return user && password ? { user, password } : null;
}

// devalue's special indices (devalue/src/constants.js).
const SPECIAL = { '-1': undefined, '-2': undefined, '-3': NaN, '-4': Infinity, '-5': -Infinity, '-6': -0 };

/**
 * Rebuild the values of a devalue array (`JSON.parse` of `#__NUXT_DATA__`).
 * Nuxt's wrappers (Reactive, ShallowReactive, Ref, …) unwrap to their value;
 * Set and Map become arrays of values / [key, value] pairs, Date an ISO
 * string. Shared and cyclic references stay shared.
 */
export function reviveNuxt(flat) {
  if (!Array.isArray(flat) || !flat.length) return undefined;
  const done = new Map();
  const hydrate = (index) => {
    if (index in SPECIAL) return SPECIAL[index];
    if (done.has(index)) return done.get(index);
    const raw = flat[index];
    if (raw === null || typeof raw !== 'object') { done.set(index, raw); return raw; }
    if (Array.isArray(raw)) {
      if (typeof raw[0] === 'string') {
        const [tag, ...args] = raw;
        if (tag === 'Set') { const out = []; done.set(index, out); for (const i of args) out.push(hydrate(i)); return out; }
        if (tag === 'Map') {
          const out = []; done.set(index, out);
          for (let i = 0; i < args.length; i += 2) out.push([hydrate(args[i]), hydrate(args[i + 1])]);
          return out;
        }
        if (tag === 'Date' || tag === 'BigInt' || tag === 'RegExp') { done.set(index, args[0]); return args[0]; }
        if (tag === 'null') { const out = {}; done.set(index, out); for (let i = 0; i < args.length; i += 2) out[args[i]] = hydrate(args[i + 1]); return out; }
        // Reactive / ShallowReactive / Ref / ShallowRef / EmptyRef / NuxtError / …: the wrapped value.
        const value = args.length && typeof args[0] === 'number' ? hydrate(args[0]) : undefined;
        done.set(index, value);
        return value;
      }
      const out = []; done.set(index, out);
      for (const i of raw) out.push(hydrate(i));
      return out;
    }
    const out = {}; done.set(index, out);
    for (const [key, i] of Object.entries(raw)) out[key] = typeof i === 'number' ? hydrate(i) : i;
    return out;
  };
  return hydrate(0);
}

/** Every plain object reachable from `root`, each once. */
function* objectsIn(root) {
  const seen = new Set();
  const stack = [root];
  while (stack.length) {
    const value = stack.pop();
    if (value === null || typeof value !== 'object' || seen.has(value)) continue;
    seen.add(value);
    if (!Array.isArray(value)) yield value;
    for (const child of Array.isArray(value) ? value : Object.values(value)) {
      if (child !== null && typeof child === 'object') stack.push(child);
    }
  }
}

/**
 * The scene object for `slug` in a revived payload: an object with that slug
 * and `videoSettings`; one with a download list first, then the fullest (the
 * page's own copy has more fields than a recommendation card of the same
 * scene). null when absent.
 */
export function findScene(root, slug) {
  const rank = (o) => (Array.isArray(o.downLoadVideoListForFront) && o.downLoadVideoListForFront.length ? 1000 : 0) + Object.keys(o).length;
  let best = null;
  for (const o of objectsIn(root)) {
    if (o.slug !== slug || !o.videoSettings || typeof o.videoSettings !== 'object') continue;
    if (!best || rank(o) > rank(best)) best = o;
  }
  return best;
}

/** `/vrb_<folder>/VRBANGERS_<folder>_<Q>_<projection>.mp4?secure=<token>,<expiry>` → its parts; null for anything else. */
export function parseVideoLink(url) {
  if (typeof url !== 'string') return null;
  const m = url.match(/^https:\/\/mov\.vrbangers\.com\/vrb_([^/?#]+)\/(VRBANGERS_([^/?#]+)_(\d+)K_([a-z0-9_]+)\.mp4)\?secure=([^&#]+)/i);
  if (!m || m[3] !== m[1]) return null;
  const expiry = Number(m[6].split(',').pop());
  return {
    folder: m[1],
    file: m[2],
    quality: `${m[4]}K`,
    k: Number(m[4]),
    projection: m[5],
    expires: Number.isFinite(expiry) && expiry > 0 ? expiry : null,
    url,
  };
}

// The scene's download folder from a slug when the payload has no scene
// object: `one-last-bang` → `one_last_bang`.
const slugFolder = (slug) => String(slug).toLowerCase().replace(/-/g, '_');

// A trailer or a MISC/ preview (`…_250p.mp4`), not a download.
const isExtra = (link) => /_trailer_|_\d+p\.mp4/i.test(link.file);

/** `{ qualityKey: bytes }` from a scene object's `videoFiles`. */
function fileSizesOf(scene) {
  const out = {};
  for (const f of Array.isArray(scene?.videoFiles) ? scene.videoFiles : []) {
    if (f && typeof f.qualityKey === 'string' && Number.isFinite(f.fileSize)) out[f.qualityKey] = f.fileSize;
  }
  return out;
}

/** All signed download links in a raw devalue array (strings only), parsed. */
function downloadLinks(flat) {
  const out = [];
  for (const value of flat) {
    if (typeof value !== 'string' || !value.includes('.mp4?secure=')) continue;
    const link = parseVideoLink(value);
    if (link && !isExtra(link)) out.push(link);
  }
  return out;
}

/**
 * The folder holding `slug`'s videos among the folders in a payload: the
 * scene's own `videoShortName`, else the slug's folder, else the shortest
 * folder starting with it (`anal_virginity` → `anal_virginity_remastered`).
 */
export function sceneFolder(folders, slug, shortName) {
  const all = [...new Set(folders)];
  if (shortName && all.includes(shortName)) return shortName;
  const key = slugFolder(slug);
  if (all.includes(key)) return key;
  const longer = all.filter((f) => f.startsWith(`${key}_`)).sort((a, b) => a.length - b.length);
  return longer[0] || null;
}

/**
 * This scene's downloads in a `#__NUXT_DATA__` array (the raw array, as
 * parsed JSON): one per file, highest quality first, as
 * `{ quality: '8K', k: 8, width, file, folder, projection, size, bytes, expires, url }`
 * (`width` nominal, 960 px per K; `size` the site's label, "17.98 GB";
 * `bytes` from the scene's file list; `expires` epoch seconds; the last
 * three null when unknown). The scene's own list (`downLoadVideoListForFront`
 * of the object with this slug) when the payload has one, else the signed
 * links of the scene's folder among all strings.
 */
export function pickSceneLinks(flat, slug) {
  if (!Array.isArray(flat)) return [];
  const scene = findScene(reviveNuxt(flat), slug);
  const listed = (Array.isArray(scene?.downLoadVideoListForFront) ? scene.downLoadVideoListForFront : [])
    .map((entry) => ({ link: parseVideoLink(entry?.url), size: typeof entry?.size === 'string' ? entry.size : null }))
    .filter(({ link }) => link && !isExtra(link));
  let picked = listed;
  if (!picked.length) {
    const links = downloadLinks(flat);
    const folder = sceneFolder(links.map((l) => l.folder), slug, scene?.videoSettings?.videoShortName);
    picked = links.filter((l) => l.folder === folder).map((link) => ({ link, size: null }));
  }
  const bytesOf = fileSizesOf(scene);
  const byFile = new Map();
  for (const { link: l, size } of picked) {
    if (byFile.has(l.file)) continue;
    byFile.set(l.file, {
      quality: l.quality, k: l.k, width: l.k * 960, file: l.file, folder: l.folder, projection: l.projection,
      size, bytes: bytesOf[`${l.quality}_${l.projection}`] ?? null, expires: l.expires, url: l.url,
    });
  }
  return [...byFile.values()].sort((a, b) => b.k - a.k || a.projection.localeCompare(b.projection));
}

/** The highest K label a VR video may have (VR_MAX_HEIGHT 4320 → 8K). */
export const VR_MAX_K = VR_MAX_HEIGHT / 540;

/**
 * The download to take: the highest quality within the VR caps
 * (≤ VR_MAX_K, fps ≤ VR_MAX_FPS when known), stereo 180 before mono 360.
 * null when none fits.
 */
export function chooseLink(links, { fps = null } = {}) {
  if (fps != null && fps > VR_MAX_FPS) return null;
  const fits = (links || []).filter((l) => l.k <= VR_MAX_K);
  const mono = (l) => /mono/i.test(l.projection);
  return fits.sort((a, b) => mono(a) - mono(b) || b.k - a.k)[0] || null;
}

const decodeEntities = (text) => String(text)
  .replace(/&#x([0-9a-f]+);/gi, (_, h) => String.fromCodePoint(parseInt(h, 16)))
  .replace(/&#(\d+);/g, (_, d) => String.fromCodePoint(Number(d)))
  .replace(/&quot;/g, '"').replace(/&apos;|&#39;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&nbsp;/g, ' ')
  .replace(/&amp;/g, '&');

const textOf = (html) => decodeEntities(String(html).replace(/<!--[\s\S]*?-->/g, '').replace(/<[^>]*>/g, ' ')).replace(/\s+/g, ' ').trim();

/** `00:35:23` / `35:23` → seconds; null when unreadable. */
export function parseDuration(text) {
  const m = String(text || '').match(/^(?:(\d+):)?(\d{1,2}):(\d{2})$/);
  return m ? Number(m[1] || 0) * 3600 + Number(m[2]) * 60 + Number(m[3]) : null;
}

/** The value spans after a `<span>Label:</span>` line of the page's info list. */
function infoValues(html, label) {
  const start = html.search(new RegExp(`<span[^>]*>\\s*${label}:\\s*</span>`, 'i'));
  if (start < 0) return [];
  const rest = html.slice(start);
  const end = rest.search(/class="single-video-info__(?!list-item__line)/i);
  const item = rest.slice(rest.indexOf('</span>') + 7, end > 0 ? end : 2000)
    .replace(/<div class="single-video-info__list-item__line"[^>]*><\/div>/i, '').replace(/<[^>]*$/, '');
  const spans = [...item.matchAll(/<span[^>]*>([\s\S]*?)<\/span>/gi)].map((m) => textOf(m[1])).filter(Boolean);
  return spans.length ? spans : [textOf(item)].filter(Boolean);
}

/** The `#__NUXT_DATA__` array of a page's HTML; null when absent or broken. */
export function nuxtDataOf(html) {
  const m = String(html || '').match(/<script\b[^>]*\bid="__NUXT_DATA__"[^>]*>([\s\S]*?)<\/script>/i);
  if (!m) return null;
  try { return JSON.parse(m[1]); } catch { return null; }
}

/**
 * What the public (logged-out) scene page tells about a scene, for scan time:
 * `{ slug, title, performers, duration (s), resolutions ['2K', …], fps,
 * degree ('180'|'360'), format ('3D'…), folder (CDN folder name),
 * qualities ['8K_180x180_3dh', …], fileSizes { qualityKey: bytes } }`.
 * The page's data block wins; the visible info list fills what it lacks.
 * null for a missing scene ("Nothing found").
 */
export function parseScenePage(html, slug) {
  const page = String(html || '');
  const scene = findScene(reviveNuxt(nuxtDataOf(page) || []), slug);
  const ogTitle = decodeEntities(page.match(/<meta[^>]*property="og:title"[^>]*content="([^"]*)"/i)?.[1] || '');
  const infoDuration = parseDuration(infoValues(page, 'Duration')[0]);
  if (!scene && infoDuration == null) return null;
  const settings = scene?.videoSettings || {};
  const qualities = Array.isArray(settings.downloadsQuality) ? settings.downloadsQuality.filter((q) => typeof q === 'string') : [];
  const fileSizes = fileSizesOf(scene);
  const resolutions = infoValues(page, 'Resolution');
  const performers = Array.isArray(scene?.models)
    ? scene.models.map((m) => m?.title).filter((t) => typeof t === 'string' && t)
    : [];
  const fps = Number(infoValues(page, 'FPS')[0]);
  return {
    slug,
    title: (typeof scene?.title === 'string' && scene.title) || ogTitle.replace(/\s+VR Porn Video:.*$/i, '').trim() || null,
    performers,
    duration: Number.isFinite(settings.duration) && settings.duration > 0 ? settings.duration : infoDuration,
    resolutions: resolutions.length ? resolutions : [...new Set(qualities.map((q) => q.split('_')[0]))],
    fps: Number.isFinite(fps) && fps > 0 ? fps : null,
    degree: infoValues(page, 'Degree')[0] || null,
    format: infoValues(page, 'Format')[0] || null,
    folder: typeof settings.videoShortName === 'string' ? settings.videoShortName : null,
    qualities,
    fileSizes,
  };
}

/** The shown form of a signed link: everything but the token. */
export const redactUrl = (url) => String(url).replace(/(\?secure=)[^,&]*/, '$1<redacted>');
