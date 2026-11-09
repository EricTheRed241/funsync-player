/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run dedupe:scenes — which library videos are the same video, and which
// copy stays (scripts/scene-dedupe-lib.mjs). Frames are synthetic pictures
// handed in by an injected function; no ffmpeg here.
import { describe, it, expect } from 'vitest';
import {
  titleWords, titlesMatch, findDuplicates, compareScenes, compareScripts, firstDuplicate, scriptSpeed, framesMatch, frameHash,
  pictureBox, hashDistance, fold, burstVerdict, FRAME_W, FRAME_H, BURST_OFFSETS_S,
} from '../../scripts/scene-dedupe-lib.mjs';

const match = (a, b, artistA = '', artistB = '') => titlesMatch(titleWords(a, artistA), titleWords(b, artistB));

// A scene as the CLI builds it; flat 1080p60 EroScripts by default.
let n = 0;
const scene = (title, extra = {}) => ({
  video: `/lib/${title}/${title}-${n++}.mp4`, dir: `/lib/${title}`, duration: 200, width: 1920, height: 1080, fps: 60, size: 5e8,
  source: 'eroscripts', handmadeAxes: 0, passthrough: false, speed: 200, scriptHash: null, words: titleWords(title, extra.artist || ''), ...extra,
});
const framesSame = async () => true;
const framesDiffer = async () => false;
const kept = async (scenes, frames = framesSame) => (await findDuplicates(scenes, frames)).map((g) => [g.keep.video, g.remove.map((r) => r.reason)]);

describe('titles', () => {
  it('match across encodes, re-downloads and labels, but not across characters', () => {
    expect(match('Oppositeodd - Entire Portfolio Part 1 Compilation - 4k120fps (LewdGazer, 1-20)', 'Oppositeodd - Entire Portfolio Part 1 Compilation - 1080p60fps (LewdGazer, 1-20)', 'Oppositeodd', 'Oppositeodd')).toBe(true);
    expect(match("Who's Ready For Tomorrow PMV (VivaPMV) (2)", "Who's Ready For Tomorrow PMV (VivaPMV)")).toBe(true);
    expect(match('maechoon - Yvonne - She Ridin On Me Passthrough_180_3DH', 'maechoon - Yvonne - She Ridin On Me_180_3DH')).toBe(true);
    expect(match('Agapilord - Primitive', 'Agapilord - Primitive League PMV', 'Agapilord', 'Agapilord')).toBe(true);
    expect(match('Sound Edit - Miku on the Couch (Maku)', 'Terminal - Miku on the Couch (Maku)', 'Sound Edit', 'Terminal')).toBe(true);
    expect(match('AidenHet - Rina - Lesson In Restraint Part 1', 'AidenHet - Yixuan - Lesson In Restraint Part 1')).toBe(false);
    expect(match('CM 048 - Cyber Sex_joni', 'CM 048 - Cyber Sex_eve')).toBe(false);
    expect(match('Part 1', 'Part 2')).toBe(false);
  });

  it('drop every quality and variant word of the shared list', () => {
    expect(match('Baxtyr - SK3L3T0K. DARK ELF LINK (1080p60)', 'Baxtyr - SK3L3T0K. DARK ELF LINK (4K60)')).toBe(true);
    expect(match('Gebby - Rondoudou Blacked II (1080)', 'Gebby - Rondoudou Blacked II')).toBe(true);
    expect(match('Scene QHD', 'Scene FHD 720')).toBe(true);
    expect(match('Demoness Boss Performance Review_alt', 'Demoness Boss Performance Review')).toBe(true);
  });

  it('fold Unicode: a decomposed (NFD) name equals the composed one, styled letters equal plain ones', () => {
    const nfd = 'maechoon - Nicole - NE\u0302MESIS';
    expect(nfd).not.toBe(nfd.normalize('NFC'));
    expect([...titleWords(nfd)]).toEqual([...titleWords(nfd.normalize('NFC'))]);
    expect(match(nfd, 'maechoon - Nicole - NEMESIS')).toBe(true);
    expect(fold('𝓣𝓪𝓷𝓪 #5')).toBe('tana #5');
  });

  it('strip a trailing (n) only when the rest is the same, so a numbered series never matches', () => {
    const kr = (k) => `FruitPunch96 - Cock Hero - Overwatch King's Row Story Mode Part 1 (${k})`;
    expect(match(kr(1), kr(17))).toBe(false);
    expect(match(kr(3), kr(5))).toBe(false);
    expect(match(kr(2), kr(2))).toBe(true);
    expect(match('Dance (2)', 'Dance')).toBe(true);
    expect(match('Dance (2)', 'Dance Extended')).toBe(false);
  });

  it('need a distinctive word: genre words alone match nothing', () => {
    expect(match('newnew - VAM', 'Other - VAM')).toBe(false);
    expect(match('Sulbas VR Compilation', 'Compilation')).toBe(false);
    expect(match('Fap Hero', 'Fap Hero')).toBe(false);
    expect(match('Fap Hero - Fantasy', 'Fantasy')).toBe(true);
  });
});

describe('findDuplicates', () => {
  it('groups the same video, keeps the biggest file, and leaves different pictures apart', async () => {
    const hd = scene('Hip Swing 1080p');
    const uhd = scene('Hip Swing 4k', { width: 3840, height: 2160, size: 2e9 });
    expect(await kept([hd, uhd])).toEqual([[uhd.video, ['bigger video']]]);
    const rina = scene('Rina - Lesson In Restraint');
    const yixuan = scene('Yixuan - Lesson In Restraint');
    expect(await kept([rina, yixuan], framesDiffer)).toEqual([]);
  });

  it('needs the same picture or the same script, and the same projection', async () => {
    expect(await kept([scene('Dance'), scene('Dance')], framesDiffer)).toEqual([]);
    const a = scene('Dance', { scriptHash: 'h', size: 6e8 });
    const b = scene('Dance', { scriptHash: 'h' });
    expect(await kept([a, b], framesDiffer)).toEqual([[a.video, ['bigger video']]]);
    expect(await kept([scene('Dance'), scene('Dance', { width: 3840, height: 1920 })])).toEqual([]); // 2:1 4K is VR
    expect(await kept([scene('Dance'), scene('Dance', { width: 1080, height: 1920 })])).toEqual([]); // portrait
  });

  it('takes the stereo check over the name and shape: one picture in a VR-shaped frame is flat', async () => {
    expect(await kept([scene('Dance'), scene('Dance', { width: 3840, height: 1920, vr: false })])).toHaveLength(1);
    expect(await kept([scene('Dance VR'), scene('Dance', { vr: false })])).toHaveLength(1);
    expect(await kept([scene('Dance'), scene('Dance', { vr: true })])).toEqual([]);
  });

  it('compares matching titles up to ±max(3 s, 1 %), there by frames only', async () => {
    const calls = [];
    const frames = async (a, b, opts) => { calls.push(opts); return true; };
    expect(await kept([scene('Dance'), scene('Dance', { duration: 203 })], frames)).toHaveLength(1);
    expect(await kept([scene('Dance'), scene('Dance', { duration: 203.01 })], frames)).toEqual([]);
    expect(await kept([scene('Eva', { duration: 1300.8 }), scene('Eva', { duration: 1313.8 })], frames)).toHaveLength(1);
    expect(await kept([scene('Eva', { duration: 1300.8 }), scene('Eva', { duration: 1314 })], frames)).toEqual([]);
    expect(calls).toEqual([{ strict: false }, { strict: false }]);
    // The same script proves it only within ±1.5 s.
    expect(await kept([scene('Dance', { scriptHash: 'h' }), scene('Dance', { scriptHash: 'h', duration: 201.5 })], framesDiffer)).toHaveLength(1);
    expect(await kept([scene('Dance', { scriptHash: 'h' }), scene('Dance', { scriptHash: 'h', duration: 202 })], framesDiffer)).toEqual([]);
    // Other titles: ±1.5 s only.
    calls.length = 0;
    expect(await kept([scene('Skrev - Zhu Yuan And Jane Doe'), scene('Unusual Apprehension Process', { duration: 202 })], frames)).toEqual([]);
    expect(calls).toEqual([]);
  });

  it('treats Extreme / Hard script variants as one title and re-uploads under another name strictly', async () => {
    const seen = [];
    const frames = async (a, b, opts) => { seen.push(opts); return true; };
    expect(await kept([scene('Goshujin VS Dorei-Ai (1v1 Extreme)'), scene('Goshujin VS Dorei-Ai (1v1 Hard)')], frames)).toHaveLength(1);
    expect(await kept([scene('Skrev - Zhu Yuan And Jane Doe'), scene("Jane Doe's and Zhu Yuan Unusual Apprehension Process")], frames)).toHaveLength(1);
    expect(seen).toEqual([{ strict: false }, { strict: true, swap: false }]);
  });

  it('merges titles one word apart each only when the frames match everywhere (swap check)', async () => {
    const swapApart = async (a, b, opts) => !opts.swap;
    // One animation, another character: same script, close frames, still two scenes.
    expect(await kept([scene('Haley Heads Will Roll', { scriptHash: 'h' }), scene('Tess Heads Will Roll', { scriptHash: 'h' })], swapApart)).toEqual([]);
    expect(await kept([scene('CM 048 - Cyber Sex_joni'), scene('CM 048 - Cyber Sex_eve')], swapApart)).toEqual([]);
    // A label, not a character: one video when the frames say so.
    expect(await kept([scene('Somato - Queen Marika 2', { duration: 147.9 }), scene('Somato - Queen Marika - Sub', { duration: 148.6 })], framesSame)).toHaveLength(1);
    expect(await kept([scene('Somato - Queen Marika 2', { duration: 147.9 }), scene('Somato - Queen Marika - Sub', { duration: 148.6 })], framesDiffer)).toEqual([]);
  });

  it('removes only copies that matched the kept video themselves, never through a chain', async () => {
    const a = scene('Chain A', { size: 3e9 });
    const b = scene('Chain B', { size: 2e9 });
    const c = scene('Chain C', { size: 1e9 });
    const pairs = new Set([`${a.video}|${b.video}`, `${b.video}|${a.video}`, `${b.video}|${c.video}`, `${c.video}|${b.video}`]);
    const frames = async (x, y) => pairs.has(`${x.video}|${y.video}`);
    const groups = await findDuplicates([a, b, c], frames);
    expect(groups.map((g) => [g.keep, g.remove.map((r) => r.scene)])).toEqual([[a, [b]]]);
  });

  it('keeps the biggest video with the fastest script, wherever that script is', async () => {
    const bigSlow = scene('X 4k', { size: 3e9, speed: 150 });
    const smallFast = scene('X 1080p', { size: 4e8, speed: 300 });
    const [g] = await findDuplicates([bigSlow, smallFast], framesSame);
    expect(g.keep).toBe(bigSlow);
    expect(g.script).toBe(smallFast);
    expect(g.scriptReason).toBe('faster script');
    expect(g.remove.map((r) => r.scene)).toEqual([smallFast]);
  });

  it('takes hand-made axes over a faster script, and leaves the kept scripts alone on a tie', async () => {
    const big = scene('Y 4k', { size: 3e9, speed: 400 });
    const handmade = scene('Y 1080p', { size: 4e8, speed: 200, handmadeAxes: 3 });
    expect((await findDuplicates([big, handmade], framesSame))[0].script).toBe(handmade);
    const a = scene('Z 4k', { size: 3e9 });
    const b = scene('Z 1080p');
    expect((await findDuplicates([a, b], framesSame))[0]).toMatchObject({ keep: a, script: a, scriptReason: null });
  });
});

describe('which copy stays', () => {
  it('keeps the video within the VR caps (8K, 60 fps) before the bigger file', () => {
    const sixK60 = scene('Z', { width: 5760, height: 2880, fps: 60, size: 2e9 });
    const twelveK90 = scene('Z', { width: 11776, height: 5888, fps: 90, size: 9e9 });
    expect(compareScenes(sixK60, twelveK90)).toEqual({ order: expect.any(Number), reason: 'within VR caps' });
    expect(compareScenes(sixK60, twelveK90).order).toBeLessThan(0);
    // Flat has no caps: the bigger file wins.
    expect(compareScenes(scene('F', { fps: 120, size: 2e9 }), scene('F', { width: 3840, height: 2160 })).order).toBeLessThan(0);
  });

  it('orders scripts by hand-made axes, then speed', () => {
    expect(compareScripts(scene('S', { speed: 250 }), scene('S', { speed: 200 })).reason).toBe('faster script');
    expect(compareScripts(scene('S', { handmadeAxes: 1 }), scene('S', { speed: 400 })).reason).toBe('hand-made axes');
  });

  it('measures script speed as travel per second', () => {
    expect(scriptSpeed([{ at: 0, pos: 0 }, { at: 500, pos: 100 }, { at: 1000, pos: 0 }])).toBe(200);
    expect(scriptSpeed([{ at: 0, pos: 50 }])).toBe(0);
  });
});

// ── Frames ──────────────────────────────────────────────────────────────

// A picture: 12×12 blocks of grey (30–220) from `seed`, drawn into `box` of a
// FRAME_W×FRAME_H frame that is black elsewhere (letterbox / pillarbox).
// `face`: one block brightened (another character's hair).
function picture(seed, { box = { x: 0, y: 0, w: FRAME_W, h: FRAME_H }, face = false } = {}) {
  let s = seed * 9301 + 49297;
  const rand = () => { s = (s * 233280 + 49297) % 1000003; return s / 1000003; };
  const blocks = Array.from({ length: 144 }, () => 30 + Math.floor(rand() * 190));
  if (face) blocks[5 * 12 + 6] = blocks[5 * 12 + 6] > 125 ? blocks[5 * 12 + 6] - 90 : blocks[5 * 12 + 6] + 90;
  const px = new Uint8Array(FRAME_W * FRAME_H);
  for (let y = box.y; y < box.y + box.h; y++) {
    for (let x = box.x; x < box.x + box.w; x++) {
      px[y * FRAME_W + x] = blocks[Math.floor(((y - box.y) * 12) / box.h) * 12 + Math.floor(((x - box.x) * 12) / box.w)];
    }
  }
  return px;
}
const black = () => new Uint8Array(FRAME_W * FRAME_H);

// A video whose picture changes every half second (a fast-cut PMV); `intro`
// seconds of other pictures come first.
const video = (duration, { seed = 1, intro = 0, ...opts } = {}) => ({
  video: `v${n++}`, duration,
  at: (t) => (t < intro ? frameHash(picture(seed * 1e5 + 99_000 + Math.floor(t * 2), opts)) : frameHash(picture(seed * 1e5 + Math.floor((t - intro) * 2), opts))),
});
const burstAt = async (v, at) => BURST_OFFSETS_S.map((o) => v.at(at + o + 0.01));

describe('frame hashes', () => {
  it('crop black bars before hashing, so a letterboxed or pillarboxed copy hashes like the full picture', () => {
    const lb = picture(7, { box: { x: 0, y: 9, w: FRAME_W, h: 54 } });
    expect(pictureBox(lb, FRAME_W, FRAME_H)).toEqual({ x: 0, y: 9, w: FRAME_W, h: 54 });
    expect(hashDistance(frameHash(lb).hash, frameHash(picture(7)).hash)).toBeLessThanOrEqual(8);
    const pb = picture(7, { box: { x: 16, y: 0, w: 96, h: FRAME_H } });
    expect(hashDistance(frameHash(pb).hash, frameHash(picture(7)).hash)).toBeLessThanOrEqual(8);
    expect(hashDistance(frameHash(picture(7)).hash, frameHash(picture(8)).hash)).toBeGreaterThan(30);
    // A black frame isn't cropped to nothing, and shows no picture.
    expect(pictureBox(black(), FRAME_W, FRAME_H)).toEqual({ x: 0, y: 0, w: FRAME_W, h: FRAME_H });
    expect(frameHash(black()).detail).toBe(0);
  });
});

describe('framesMatch', () => {
  it('finds the same picture when one copy has an intro the other lacks (aligned at the end)', async () => {
    expect(await framesMatch(video(300), video(305, { intro: 5 }), burstAt)).toBe(true);
    expect(await framesMatch(video(300), video(305, { seed: 2 }), burstAt)).toBe(false);
  });

  it('searches ±1 s around each sample, so a small offset still matches', async () => {
    expect(await framesMatch(video(300), video(300.4, { intro: 0.4 }), burstAt, { strict: true })).toBe(true);
  });

  it('compares a letterboxed copy with the full picture', async () => {
    expect(await framesMatch(video(300), video(300, { box: { x: 0, y: 9, w: FRAME_W, h: 54 } }), burstAt, { strict: true })).toBe(true);
  });

  it('strict: every sample must show a picture; loose: blanks are skipped', async () => {
    const fades = (v) => ({ ...v, at: (t) => (t < 150 ? frameHash(black()) : v.at(t)) });
    const a = video(300);
    const b = video(300);
    expect(await framesMatch(fades(a), fades(b), burstAt)).toBe(true);
    expect(await framesMatch(fades(a), fades(b), burstAt, { strict: true })).toBe(false);
    const blank = { video: 'x', duration: 300, at: () => frameHash(black()) };
    expect(await framesMatch(blank, { ...blank, video: 'y' }, burstAt)).toBe(false);
  });

  it('swap: a changed face fails, an identical picture passes', async () => {
    const a = video(300);
    const face = video(300, { face: true });
    expect(await framesMatch(a, face, burstAt, { strict: true })).toBe(true);
    expect(await framesMatch(a, face, burstAt, { swap: true })).toBe(false);
    expect(await framesMatch(a, video(300), burstAt, { swap: true })).toBe(true);
  });

  it('aligns on pictures only: two cuts through black never make different pictures match', () => {
    const blank = frameHash(black());
    expect(burstVerdict([blank, frameHash(picture(1)), blank], [blank, frameHash(picture(2)), blank]).verdict).toBe('different');
    expect(burstVerdict([blank, frameHash(picture(1)), blank], [frameHash(picture(1)), frameHash(picture(2)), blank]).verdict).toBe('same');
  });

  it('treats frames that could not be read as unknown', async () => {
    const a = video(300);
    const unreadable = { ...video(300), video: 'broken' };
    const failing = async (v, at) => (v.video === 'broken' ? null : burstAt(v, at));
    expect(await framesMatch(a, unreadable, failing)).toBe(false);
    expect(await framesMatch(a, unreadable, failing, { strict: true })).toBe(false);
  });
});

describe('firstDuplicate (scouts, before a download)', () => {
  it('finds a library video with the same title words, length and projection', () => {
    const library = [{ words: titleWords('Agapilord - Sexual', 'Agapilord'), duration: 190, width: 1920, height: 1080 }];
    expect(firstDuplicate({ words: titleWords('Agapilord - HMV - SEXUAL'), duration: 190.8, vr: false }, library)).toBe(library[0]);
    expect(firstDuplicate({ words: titleWords('Agapilord - HMV - SEXUAL'), duration: 195, vr: false }, library)).toBeNull();
    expect(firstDuplicate({ words: titleWords('Agapilord - HMV - SEXUAL'), duration: 190, vr: true }, library)).toBeNull();
  });

  it('skips the orientation test when the candidate has no picture size (FapTap)', () => {
    const portrait = [{ words: titleWords('JuliaBlack - Piper Presley Onlyfans POV Creampie', 'JuliaBlack'), duration: 300, width: 1080, height: 1920 }];
    expect(firstDuplicate({ words: titleWords('Piper Presley Onlyfans POV Creampie'), duration: 300.5, vr: false }, portrait)).toBe(portrait[0]);
    expect(firstDuplicate({ words: titleWords('Piper Presley Onlyfans POV Creampie'), duration: 300.5, vr: false, width: 1920, height: 1080 }, portrait)).toBeNull();
  });
});
