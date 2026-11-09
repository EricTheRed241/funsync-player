// VRBangers scenes (vrbangers.com) as a video host for npm run
// eroscripts:scout (scripts/eroscripts-scout.mjs): a Free Scripts post that
// links `https://vrbangers.com/video/<slug>/` gets that scene's video from
// the user's paid account.
//
// Scan / --add: the public scene page (plain fetch, no login;
// scripts/vrbangers-browser.mjs scenePublic) becomes one pseudo video file:
// the best quality within the VR caps, named like the CDN file
// (`VRBANGERS_<folder>_8K_180x180_3dh.mp4`), with its size and length, so it
// pairs with the post's scripts like any other video (planWithStreams checks
// the length). --fetch: one headless Chrome per pass (openVrbangers), logged
// in once, reads the scene's signed links right before each download (they
// expire after 2–4 h) and streams the planned file with Chrome's user agent
// (the link needs no cookie). Page loads go at least VRBANGERS_INTERVAL_MS
// apart. The password, cookies and link tokens are never printed.

import fs from 'node:fs';
import { Readable } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { chooseLink, sceneUrl } from './vrbangers-lib.mjs';
import { durationMatches, pacedFetch } from './stream-hosts.mjs';

export const VRBANGERS_HOST = 'vrbangers';
/** Least time between two vrbangers.com page reads. */
export const VRBANGERS_INTERVAL_MS = 1000;

// ── Links ───────────────────────────────────────────────────────────────

const SCENE_PAGE = /^https?:\/\/(?:www\.)?vrbangers\.com\/video\/([a-z0-9][a-z0-9-]*)\/?(?:[?#]|$)/i;

/**
 * The VRBangers scene pages among a post's links, as
 * `{slug, url: 'https://vrbangers.com/video/<slug>/'}`, one per slug, in link
 * order. Model, category and other site pages aren't scenes.
 */
export function vrbangersLinks(hrefs) {
  const bySlug = new Map();
  for (const href of hrefs || []) {
    const m = SCENE_PAGE.exec(String(href).trim());
    const slug = m?.[1].toLowerCase();
    if (slug && !bySlug.has(slug)) bySlug.set(slug, { slug, url: sceneUrl(slug) });
  }
  return [...bySlug.values()];
}

/**
 * The CDN folders named by a post's files in the site's spelling
 * (`VRBANGERS_<folder>[_<Q>_<projection>][.<axis>].funscript|.mp4`; quality
 * and projection dropped), lower case, in file order: `VRBANGERS_anal_
 * virginity_remastered_8K_180x180_3dh.funscript` → `anal_virginity_remastered`.
 */
export function vrbangersFolders(names) {
  const out = [];
  for (const name of names || []) {
    const m = /^VRBANGERS_([^.]+)/i.exec(String(name).trim());
    if (!m) continue;
    const folder = m[1].toLowerCase()
      .replace(/_(?:\d+k|hd|sd|fhd|uhd|trailer)(?:_.*)?$/, '')
      .replace(/_(?:180x180|360x180|180|360|mono360|mono180|3dh|3dv|lr|tb|sbs)(?:_.*)?$/, '');
    if (/^[a-z0-9_]+$/.test(folder) && !out.includes(folder)) out.push(folder);
  }
  return out;
}

/** A title part as a site slug: `A Gentleman's Wish` → `a-gentlemans-wish`. */
const slugOf = (text) => String(text).normalize('NFKD').replace(/\p{M}+/gu, '').toLowerCase()
  .replace(/['’`]/g, '').replace(/&/g, ' and ').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');

/**
 * Slugs a post title naming VRBangers may hold, in title order: its ` - ` /
 * `:` / `|` parts without bracket groups, emoji codes and the studio name
 * (`VRBangers - Harmony Wonder - Boned in Ukraine` → `harmony-wonder`,
 * `boned-in-ukraine`); parts listing several people (`,` / `&`) go last.
 * [] when the title doesn't name VRBangers.
 */
export function vrbangersTitleSlugs(title) {
  const t = String(title || '');
  if (!/vr\s*bangers/i.test(t)) return [];
  const parts = t.replace(/:[a-z0-9_+-]+:/gi, ' ')
    .replace(/\[[^\]]*\]|\([^)]*\)|（[^）]*）|【[^】]*】|\{[^}]*\}/g, ' ')
    .split(/\s*[:|｜]\s*|\s+[-–—]\s+/)
    .map((p) => p.trim())
    .filter((p) => /\p{L}/u.test(p) && !/vr\s*bangers/i.test(p));
  const ordered = [...parts.filter((p) => !/[,&]/.test(p)), ...parts.filter((p) => /[,&]/.test(p))];
  return [...new Set(ordered.map(slugOf).filter((s) => s.length >= 3))];
}

/**
 * Where a post's VRBangers scenes may be, best evidence first, each lead a
 * list of slugs to try until one is a scene: a `vrbangers.com/video/<slug>/`
 * link (one lead each); a CDN folder its files name (`folder`: the page
 * must use it; `anal_virginity_remastered` → `anal-virginity-remastered`,
 * then `anal-virginity`); and, tried only when no earlier lead found a
 * scene (`fallback`), the parts of a title naming VRBangers. A slug is in
 * one lead only.
 *
 * @returns {{from: 'link'|'file'|'title', slugs: string[], folder?: string, fallback?: boolean}[]}
 */
export function vrbangersLeads({ hrefs = [], names = [], title = '' } = {}) {
  const leads = [];
  const seen = new Set();
  const add = (lead) => {
    const slugs = [...new Set(lead.slugs)].filter((s) => s && !seen.has(s));
    for (const s of slugs) seen.add(s);
    if (slugs.length) leads.push({ ...lead, slugs });
  };
  for (const { slug } of vrbangersLinks(hrefs)) add({ from: 'link', slugs: [slug] });
  for (const folder of vrbangersFolders(names)) {
    const slug = folder.replace(/_/g, '-');
    add({ from: 'file', folder, slugs: [slug, slug.replace(/-remaster(?:ed)?$/, '')] });
  }
  add({ from: 'title', slugs: vrbangersTitleSlugs(title), fallback: true });
  return leads;
}

// ── Pseudo file ─────────────────────────────────────────────────────────

/**
 * The downloads a public scene page announces, as chooseLink takes them:
 * `qualities` (`8K_180x180_3dh` …), else its Resolution list with the
 * projection its Degree implies (180 → `180x180_3dh`, 360 → `mono360`).
 */
function announced(facts) {
  const keys = facts.qualities?.length
    ? facts.qualities
    : (facts.resolutions || []).map((r) => `${r}_${String(facts.degree) === '360' ? 'mono360' : '180x180_3dh'}`);
  return keys.map((key) => {
    const m = /^(\d+)K_([a-z0-9_]+)$/i.exec(key);
    return m && { key, quality: `${m[1]}K`, k: Number(m[1]), projection: m[2] };
  }).filter(Boolean);
}

/**
 * A public scene page (parseScenePage facts) as the scout's one video file:
 * the quality chooseLink takes (≤ 8K, ≤ 60 fps, stereo 180 before mono 360),
 * `{name: 'VRBANGERS_<folder>_<Q>_<projection>.mp4', size: <bytes>|0, host:
 * 'vrbangers', url: <scene page>, slug, duration: <s>|null, quality,
 * projection, title, performers}`. null when no quality fits the caps.
 */
export function vrbangersFile(url, facts) {
  const best = chooseLink(announced(facts), { fps: facts.fps ?? null });
  if (!best) return null;
  const folder = facts.folder || String(facts.slug).toLowerCase().replace(/-/g, '_');
  return {
    name: `VRBANGERS_${folder}_${best.quality}_${best.projection}.mp4`,
    size: facts.fileSizes?.[best.key] || 0,
    host: VRBANGERS_HOST,
    url,
    slug: facts.slug,
    duration: facts.duration ?? null,
    quality: best.quality,
    projection: best.projection,
    title: facts.title || null,
    performers: facts.performers || [],
  };
}

// ── Session ─────────────────────────────────────────────────────────────

const clock = (ms) => `${Math.floor(ms / 60_000)}:${String(Math.round(ms / 1000) % 60).padStart(2, '0')}`;
const lazyBrowser = () => import('./vrbangers-browser.mjs');

/**
 * VRBangers for the scout. Public reads and downloads go through one
 * pacedFetch (≥ VRBANGERS_INTERVAL_MS apart); Chrome (`open`, default
 * openVrbangers) starts on the first member read and is logged in once
 * (`ensureLoggedIn`: a LoginError, exit code 3, ends the run).
 *
 * - `filesFor(leads)`: the scenes vrbangersLeads point at, one pseudo video
 *   file each (vrbangersFile) → `{files, missed: [{lead, why}]}`. A lead
 *   tries its slugs in order until one is a scene; a file lead's scene must
 *   use its folder, a title lead's scene title must be the slug tried, and a
 *   fallback lead is skipped once an earlier lead found a scene.
 * - `fresh(file)`: the scene's signed links, read now: the planned file,
 *   else the best one within the caps → `{link, userAgent}`.
 * - `probe(file)`: fresh, then a 1-byte Range request → `{link, status, total}`.
 * - `download(file, tmp, {scriptMs, videoSeconds})`: fresh, stream into
 *   `tmp`, then the size (the site's byte count) and the length (ffprobe
 *   through `videoSeconds`, durationMatches with the main script, else the
 *   page's length) must agree. Returns the link taken.
 * - `run(fn)`: `fn()`, then `close()`, also when it throws.
 * - `close()`: quit Chrome if it was started.
 */
export function createVrbangers({
  open = async (o) => (await lazyBrowser()).openVrbangers(o),
  scenePublic = async (slug, o) => (await lazyBrowser()).scenePublic(slug, o),
  fetch = globalThis.fetch, intervalMs = VRBANGERS_INTERVAL_MS, now, sleep,
  userData, credentialsFile, log = () => {},
} = {}) {
  const get = pacedFetch({ fetch, intervalMs, now, sleep, label: VRBANGERS_HOST });
  let session = null;
  const browser = () => {
    session ||= (async () => {
      const b = await open({ userData, credentialsFile, log });
      try {
        await b.ensureLoggedIn();
      } catch (err) {
        await b.close();
        throw err;
      }
      return b;
    })();
    return session;
  };
  const host = {
    async filesFor(leads) {
      const files = [];
      const missed = [];
      for (const lead of leads) {
        if (lead.fallback && files.length) continue;
        let why = 'no such scene';
        let file = null;
        for (const slug of lead.slugs) {
          let facts;
          try { facts = await scenePublic(slug, { fetchImpl: get }); } catch (err) { why = err.message; continue; }
          if (!facts) continue;
          const titled = !facts.title || slugOf(facts.title.replace(/\([^)]*\)|\[[^\]]*\]/g, ' ')) === slug;
          if (lead.folder && facts.folder && facts.folder !== lead.folder) { why = `${slug} is the scene of folder ${facts.folder}`; continue; }
          // A guess (title, or a file name when the page doesn't say its folder) must be the page's own title.
          if ((lead.from === 'title' || (lead.folder && !facts.folder)) && !titled) { why = `${slug} is "${facts.title}"`; continue; }
          file = vrbangersFile(sceneUrl(slug), facts);
          why = file ? '' : `${slug}: no quality within the VR caps`;
          break;
        }
        if (file && !files.some((f) => f.slug === file.slug)) files.push(file);
        else if (!file) missed.push({ lead, why });
      }
      return { files, missed };
    },
    async fresh(file) {
      const b = await browser();
      const links = await b.sceneLinks(file.slug);
      const link = links.find((l) => l.file === file.name) || chooseLink(links);
      if (!link) throw new Error(`vrbangers: no download within the VR caps for ${file.slug} (${links.length} listed; is the account's plan active?)`);
      return { link, userAgent: b.userAgent };
    },
    async probe(file) {
      const { link, userAgent } = await host.fresh(file);
      const resp = await get(link.url, { headers: { 'User-Agent': userAgent, Range: 'bytes=0-0' }, signal: AbortSignal.timeout(60_000) });
      await resp.body?.cancel?.().catch(() => {});
      const total = Number((/\/(\d+)\s*$/.exec(resp.headers.get('content-range') || '') || [])[1]) || 0;
      return { link, status: resp.status, total };
    },
    async download(file, tmp, { scriptMs = null, videoSeconds } = {}) {
      if (scriptMs && file.duration && !durationMatches(file.duration, scriptMs)) {
        throw new Error(`vrbangers: video ${clock(file.duration * 1000)}, script ${clock(scriptMs)}; not this scene's video`);
      }
      const { link, userAgent } = await host.fresh(file);
      const resp = await get(link.url, { headers: { 'User-Agent': userAgent } });
      if (!resp.ok || !resp.body) {
        await resp.body?.cancel?.().catch(() => {});
        throw new Error(`vrbangers: HTTP ${resp.status} for ${link.file}`);
      }
      await pipeline(Readable.fromWeb(resp.body), fs.createWriteStream(tmp));
      const want = link.bytes || (link.file === file.name ? file.size : 0) || Number(resp.headers.get('content-length')) || 0;
      const size = fs.statSync(tmp).size;
      if (want && size !== want) throw new Error(`vrbangers: size ${size} ≠ ${want} for ${link.file}`);
      const got = videoSeconds(tmp);
      if (!got) throw new Error(`vrbangers: ${link.file} isn't a readable video (ffprobe)`);
      // The main script must fit the download; without one the page's length and the file's must fit each other either way.
      const fits = scriptMs ? durationMatches(got, scriptMs) : !file.duration || durationMatches(got, file.duration * 1000) || durationMatches(file.duration, got * 1000);
      if (!fits) throw new Error(`vrbangers: downloaded video ${clock(got * 1000)}, ${scriptMs ? `script ${clock(scriptMs)}` : `page ${clock(file.duration * 1000)}`}; not this scene's video`);
      return link;
    },
    async run(fn) {
      try {
        return await fn();
      } finally {
        await host.close();
      }
    },
    async close() {
      const s = session;
      session = null;
      if (!s) return;
      let b;
      try { b = await s; } catch { return; }
      await b.close();
    },
  };
  return host;
}
