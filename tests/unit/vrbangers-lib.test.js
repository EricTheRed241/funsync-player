/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// VRBangers member downloads (scripts/vrbangers-lib.mjs): which signed links
// of a scene page's __NUXT_DATA__ belong to the scene, which quality is taken,
// the credentials file and the public scene page. No vrbangers.com.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it, expect } from 'vitest';
import {
  parseCredentials, reviveNuxt, findScene, parseVideoLink, sceneFolder, pickSceneLinks, chooseLink, VR_MAX_K,
  parseDuration, parseScenePage, nuxtDataOf, redactUrl, sceneUrl,
} from '../../scripts/vrbangers-lib.mjs';
import { VR_MAX_HEIGHT } from '../../renderer/js/content-preferences.js';

const FIXTURE = path.join(path.dirname(fileURLToPath(import.meta.url)), 'fixtures', 'vrbangers-scene.html');

// Flatten values the way Nuxt's devalue does: every object, array and
// primitive is an entry, containers hold entry indices; the root is wrapped
// in ["ShallowReactive", 1]. Equal primitives share one entry.
function devalue(root) {
  const out = [['ShallowReactive', 1]];
  const seen = new Map();
  const enc = (v) => {
    const prim = v === null || typeof v !== 'object';
    if (prim && seen.has(v)) return seen.get(v);
    const i = out.length;
    out.push(null);
    if (prim) seen.set(v, i);
    if (Array.isArray(v)) out[i] = v.map(enc);
    else if (!prim) out[i] = Object.fromEntries(Object.entries(v).map(([k, x]) => [k, enc(x)]));
    else out[i] = v;
    return i;
  };
  enc(root);
  return out;
}

const EXP = 1794260760;
const mov = (folder, q, proj = '180x180_3dh', exp = EXP) => `https://mov.vrbangers.com/vrb_${folder}/VRBANGERS_${folder}_${q}_${proj}.mp4?secure=tok${q}==,${exp}`;
const entry = (folder, q, size = '1 GB', proj = '180x180_3dh', exp = EXP) => ({ size, quality: `${q}_${proj}`, label: q, url: mov(folder, q, proj, exp) });
const scene = (slug, folder, extra = {}) => ({ slug, title: slug, videoSettings: { duration: 3141, videoShortName: folder }, ...extra });

// A member's scene page: the page's own scene (folder ≠ slug) with its list,
// a recommendation card of the same scene (older expiry, fewer fields), other
// scenes' links, the old non-remastered folder, trailers and MISC extras.
function memberPage({ withList = true } = {}) {
  const AV = 'anal_virginity_remastered';
  const own = scene('anal-virginity', AV, {
    id: 'x', categories: [], models: [{ title: 'Riley Reid' }],
    videoFiles: [{ qualityKey: '8K_180x180_3dh', fileSize: 19301406842 }, { qualityKey: '2K_180x180_3dh', fileSize: 3214156404 }],
    trailer: `https://trailers.vrbangers.com/vrb_${AV}/VRBANGERS_${AV}_trailer_4K_180x180_3dh.mp4?secure=t==,${EXP}&s=1`,
    toy: `https://mov.vrbangers.com/vrb_${AV}/MISC/VRBANGERS_${AV}_toy.funscript`,
    preview: `https://mov.vrbangers.com/vrb_${AV}/MISC/VRBANGERS_${AV}_250p.mp4`,
    ...(withList ? {
      downLoadVideoListForFront: [entry(AV, '2K', '2.99 GB'), entry(AV, '4K', '4.8 GB'), entry(AV, '6K'), entry(AV, '8K', '17.98 GB'), entry(AV, '2K', '2.99 GB')],
    } : { strayLinks: ['2K', '4K', '6K', '8K'].map((q) => mov(AV, q)) }),
  });
  const card = scene('anal-virginity', AV, withList ? { downLoadVideoListForFront: [entry(AV, '8K', '17.98 GB', '180x180_3dh', EXP + 99)] } : {});
  const other = scene('wet-and-wild-in-malibu', 'wet_and_wild_in_malibu', {
    downLoadVideoListForFront: ['2K', '4K', '8K', '12K'].map((q) => entry('wet_and_wild_in_malibu', q)),
  });
  const oldCut = scene('anal-virginity-original', 'anal_virginity', { downLoadVideoListForFront: [entry('anal_virginity', '4K')] });
  return devalue({ state: { videos: { video: own, banners: [card, other, oldCut] } }, path: '/video/anal-virginity/' });
}

describe('parseCredentials', () => {
  it('reads Username / password lines among other text', () => {
    const text = 'browser element to find video: __NUXT__.state.videos.video.downLoadVideoListForFront\n\nUsername: someone+vrb@example.com\npassword: s3cret!pw\n';
    expect(parseCredentials(text)).toEqual({ user: 'someone+vrb@example.com', password: 's3cret!pw' });
  });
  it('tolerates case, spacing and CRLF', () => {
    expect(parseCredentials('USERNAME :a@b.c\r\nPassword:   pw\r\n')).toEqual({ user: 'a@b.c', password: 'pw' });
  });
  it('is null when a field is missing or empty', () => {
    expect(parseCredentials('Username: a@b.c\n')).toBeNull();
    expect(parseCredentials('Username: a@b.c\npassword:\n')).toBeNull();
    expect(parseCredentials('')).toBeNull();
    expect(parseCredentials(undefined)).toBeNull();
  });
});

describe('reviveNuxt', () => {
  it('unwraps Nuxt wrappers, resolves indices and keeps shared objects shared', () => {
    const flat = [['Reactive', 1], { a: 2, b: 3, c: 3, d: 5, e: -1 }, 'x', { n: 4 }, 7, ['Set', 2, 4]];
    const root = reviveNuxt(flat);
    expect(root.a).toBe('x');
    expect(root.b).toEqual({ n: 7 });
    expect(root.b).toBe(root.c);
    expect(root.d).toEqual(['x', 7]);
    expect(root.e).toBeUndefined();
  });
  it('survives cycles', () => {
    const root = reviveNuxt([{ self: 0, v: 1 }, 'ok']);
    expect(root.self).toBe(root);
  });
  it('is undefined for an empty or non-array payload', () => {
    expect(reviveNuxt([])).toBeUndefined();
    expect(reviveNuxt(null)).toBeUndefined();
  });
});

describe('findScene', () => {
  it("takes the page's copy (download list, more fields) over a recommendation card", () => {
    const found = findScene(reviveNuxt(memberPage()), 'anal-virginity');
    expect(found.id).toBe('x');
  });
  it('is null when no object has the slug', () => {
    expect(findScene(reviveNuxt(memberPage()), 'one-last-bang')).toBeNull();
  });
});

describe('parseVideoLink', () => {
  it('splits a signed download link', () => {
    expect(parseVideoLink(mov('one_last_bang', '4K'))).toEqual({
      folder: 'one_last_bang', file: 'VRBANGERS_one_last_bang_4K_180x180_3dh.mp4', quality: '4K', k: 4,
      projection: '180x180_3dh', expires: EXP, url: mov('one_last_bang', '4K'),
    });
    expect(parseVideoLink(mov('x', '2K', 'mono360')).projection).toBe('mono360');
  });
  it('rejects trailers, MISC files, other hosts and unsigned links', () => {
    expect(parseVideoLink(`https://mov.vrbangers.com/vrb_x/VRBANGERS_x_trailer_4K_180x180_3dh.mp4?secure=a==,${EXP}`)).toBeNull();
    expect(parseVideoLink('https://mov.vrbangers.com/vrb_x/MISC/VRBANGERS_x_250p.mp4')).toBeNull();
    expect(parseVideoLink(`https://trailers.vrbangers.com/vrb_x/VRBANGERS_x_4K_180x180_3dh.mp4?secure=a==,${EXP}`)).toBeNull();
    expect(parseVideoLink('https://mov.vrbangers.com/vrb_x/VRBANGERS_x_4K_180x180_3dh.mp4')).toBeNull();
    expect(parseVideoLink(42)).toBeNull();
  });
});

describe('sceneFolder', () => {
  const folders = ['anal_virginity', 'anal_virginity_remastered', 'anal_virginity_remastered_part_2', 'one_last_bang'];
  it("prefers the scene's videoShortName", () => {
    expect(sceneFolder(folders, 'anal-virginity', 'anal_virginity_remastered')).toBe('anal_virginity_remastered');
  });
  it('then the slug itself, then the shortest folder starting with it', () => {
    expect(sceneFolder(folders, 'anal-virginity')).toBe('anal_virginity');
    expect(sceneFolder(folders.slice(1), 'anal-virginity')).toBe('anal_virginity_remastered');
    expect(sceneFolder(folders, 'one-last-bang', 'gone')).toBe('one_last_bang');
  });
  it('is null when nothing matches (no substring guesses)', () => {
    expect(sceneFolder(['last_bang'], 'one-last-bang')).toBeNull();
    expect(sceneFolder(['one_last_bangers'], 'one-last-bang')).toBeNull();
  });
});

describe('pickSceneLinks', () => {
  it("returns only the scene's own downloads from its list, best first, once each", () => {
    const links = pickSceneLinks(memberPage(), 'anal-virginity');
    expect(links.map((l) => l.file)).toEqual([
      'VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.mp4',
      'VRBANGERS_anal_virginity_remastered_6K_180x180_3dh.mp4',
      'VRBANGERS_anal_virginity_remastered_4K_180x180_3dh.mp4',
      'VRBANGERS_anal_virginity_remastered_2K_180x180_3dh.mp4',
    ]);
    expect(links[0]).toEqual({
      quality: '8K', k: 8, width: 7680, file: 'VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.mp4',
      folder: 'anal_virginity_remastered', projection: '180x180_3dh', size: '17.98 GB', bytes: 19301406842,
      expires: EXP, url: mov('anal_virginity_remastered', '8K'),
    });
    expect(links[1].bytes).toBeNull();
  });
  it('falls back to the folder of the scene among all signed strings when the list is missing', () => {
    const links = pickSceneLinks(memberPage({ withList: false }), 'anal-virginity');
    expect(links.map((l) => l.quality)).toEqual(['8K', '6K', '4K', '2K']);
    expect(new Set(links.map((l) => l.folder))).toEqual(new Set(['anal_virginity_remastered']));
    expect(links.every((l) => l.size === null)).toBe(true);
  });
  it('falls back to slug folder matching when the payload has no scene object', () => {
    const flat = devalue({ links: [mov('one_last_bang', '2K'), mov('one_last_bang', '4K'), mov('wet_and_wild_in_malibu', '8K'), `https://mov.vrbangers.com/vrb_one_last_bang/VRBANGERS_one_last_bang_trailer_4K_180x180_3dh.mp4?secure=a==,${EXP}`] });
    expect(pickSceneLinks(flat, 'one-last-bang').map((l) => l.quality)).toEqual(['4K', '2K']);
    const remastered = devalue({ links: [mov('anal_virginity_remastered', '8K'), mov('wet_and_wild_in_malibu', '8K')] });
    expect(pickSceneLinks(remastered, 'anal-virginity').map((l) => l.file)).toEqual(['VRBANGERS_anal_virginity_remastered_8K_180x180_3dh.mp4']);
  });
  it('is empty for a logged-out page, another scene only, or no payload', () => {
    expect(pickSceneLinks(devalue({ s: scene('one-last-bang', 'one_last_bang') }), 'one-last-bang')).toEqual([]);
    expect(pickSceneLinks(memberPage(), 'one-last-bang')).toEqual([]);
    expect(pickSceneLinks(null, 'x')).toEqual([]);
  });
});

describe('chooseLink', () => {
  const link = (k, projection = '180x180_3dh') => ({ quality: `${k}K`, k, projection, file: `${k}K_${projection}` });
  it('reads the cap from content-preferences (VR_MAX_HEIGHT 4320 → 8K)', () => {
    expect(VR_MAX_K).toBe(VR_MAX_HEIGHT / 540);
  });
  it(`takes the highest quality within the cap (12K / 10K skipped)`, () => {
    expect(chooseLink([link(2), link(12), link(8), link(10), link(6)]).k).toBe(Math.min(8, VR_MAX_K));
    expect(chooseLink([link(2), link(3), link(4)]).k).toBe(4);
  });
  it('takes stereo 180 over a higher mono 360', () => {
    expect(chooseLink([link(4, 'mono360'), link(2)]).file).toBe('2K_180x180_3dh');
    expect(chooseLink([link(4, 'mono360'), link(2, 'mono360')]).file).toBe('4K_mono360');
  });
  it('is null when nothing fits, or the fps is over the cap', () => {
    expect(chooseLink([link(12)])).toBeNull();
    expect(chooseLink([])).toBeNull();
    expect(chooseLink([link(4)], { fps: 120 })).toBeNull();
    expect(chooseLink([link(4)], { fps: 60 }).k).toBe(4);
  });
});

describe('public scene page', () => {
  const html = fs.readFileSync(FIXTURE, 'utf8');
  it('reads the scene facts (folder ≠ slug for a remaster)', () => {
    expect(parseScenePage(html, 'anal-virginity')).toEqual({
      slug: 'anal-virginity',
      title: 'Anal Virginity (Remastered)',
      performers: ['Riley Reid'],
      duration: 3141,
      resolutions: ['2K', '4K', '6K', '8K'],
      fps: 60,
      degree: '180',
      format: '3D',
      folder: 'anal_virginity_remastered',
      qualities: ['2K_180x180_3dh', '4K_180x180_3dh', '6K_180x180_3dh', '8K_180x180_3dh'],
      fileSizes: {
        '2K_180x180_3dh': 3214156404, '4K_180x180_3dh': 5152418518, '6K_180x180_3dh': 11836374888, '8K_180x180_3dh': 19301406842,
      },
    });
  });
  it('falls back to the visible info list without the data block', () => {
    const bare = html.replace(/<script[^>]*__NUXT_DATA__[\s\S]*?<\/script>/, '');
    expect(nuxtDataOf(bare)).toBeNull();
    const facts = parseScenePage(bare, 'anal-virginity');
    expect(facts).toMatchObject({ duration: 3141, resolutions: ['2K', '4K', '6K', '8K'], degree: '180', format: '3D', folder: null, performers: [] });
    expect(facts.title).toMatch(/^Anal Virginity/);
  });
  it('is null for a missing scene page', () => {
    expect(parseScenePage('<html><title>Nothing found for gentlemans-wish</title></html>', 'gentlemans-wish')).toBeNull();
  });
  it('parses durations', () => {
    expect(parseDuration('00:52:21')).toBe(3141);
    expect(parseDuration('35:23')).toBe(2123);
    expect(parseDuration('soon')).toBeNull();
  });
});

describe('helpers', () => {
  it('redacts the token, keeps the file and expiry', () => {
    expect(redactUrl(mov('x', '8K'))).toBe(`https://mov.vrbangers.com/vrb_x/VRBANGERS_x_8K_180x180_3dh.mp4?secure=<redacted>,${EXP}`);
  });
  it('builds scene URLs', () => {
    expect(sceneUrl('one-last-bang')).toBe('https://vrbangers.com/video/one-last-bang/');
  });
});
