/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run dedupe:scenes — which library videos are the same video, and which
// copy stays (scripts/scene-dedupe-lib.mjs). Frames are compared by an
// injected function; no ffmpeg here.
import { describe, it, expect } from 'vitest';
import { titleWords, titlesMatch, findDuplicates, compareScenes, compareScripts, firstDuplicate, scriptSpeed, framesMatch } from '../../scripts/scene-dedupe-lib.mjs';

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

  it('needs the same picture or the same script, the same length, and the same projection', async () => {
    expect(await kept([scene('Dance'), scene('Dance')], framesDiffer)).toEqual([]);
    const a = scene('Dance', { scriptHash: 'h', size: 6e8 });
    const b = scene('Dance', { scriptHash: 'h' });
    expect(await kept([a, b], framesDiffer)).toEqual([[a.video, ['bigger video']]]);
    expect(await kept([scene('Dance'), scene('Dance', { duration: 203 })])).toEqual([]);
    expect(await kept([scene('Dance'), scene('Dance', { width: 3840, height: 1920 })])).toEqual([]); // 2:1 4K is VR
    expect(await kept([scene('Dance'), scene('Dance', { width: 1080, height: 1920 })])).toEqual([]); // portrait
  });

  it('treats Extreme / Hard script variants as one title, re-uploads under another name strictly, character swaps never', async () => {
    const seen = [];
    const frames = async (a, b, opts) => { seen.push(opts.strict); return true; };
    expect(await kept([scene('Goshujin VS Dorei-Ai (1v1 Extreme)'), scene('Goshujin VS Dorei-Ai (1v1 Hard)')], frames)).toHaveLength(1);
    expect(await kept([scene('Skrev - Zhu Yuan And Jane Doe'), scene("Jane Doe's and Zhu Yuan Unusual Apprehension Process")], frames)).toHaveLength(1);
    expect(seen).toEqual([false, true]);
    // One animation, another character: same frames, same script, still two scenes.
    expect(await kept([scene('Haley Heads Will Roll', { scriptHash: 'h' }), scene('Tess Heads Will Roll', { scriptHash: 'h' })], frames)).toEqual([]);
    expect(await kept([scene('CM 048 - Cyber Sex_joni'), scene('CM 048 - Cyber Sex_eve')], frames)).toEqual([]);
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

describe('framesMatch', () => {
  const frame = (fill, spread = 0) => Uint8Array.from({ length: 48 }, (_, i) => fill + (i % 2) * spread);
  it('loose: the mean difference decides', () => {
    expect(framesMatch([frame(100, 40), frame(10)], [frame(104, 40), frame(12)])).toBe(true);
    expect(framesMatch([frame(100, 40)], [frame(140, 40)])).toBe(false);
  });
  it('strict: every frame must match and show a picture, so blank or faded frames prove nothing', () => {
    expect(framesMatch([frame(100, 40), frame(50, 30)], [frame(102, 40), frame(51, 30)], { strict: true })).toBe(true);
    expect(framesMatch([frame(0), frame(0)], [frame(0), frame(0)], { strict: true })).toBe(false);
    expect(framesMatch([frame(100, 40), frame(50, 30)], [frame(100, 40), frame(80, 30)], { strict: true })).toBe(false);
  });
});

describe('firstDuplicate (scouts, before a download)', () => {
  it('finds a library video with the same title words, length and projection', () => {
    const library = [{ words: titleWords('Agapilord - Sexual', 'Agapilord'), duration: 190, width: 1920, height: 1080 }];
    expect(firstDuplicate({ words: titleWords('Agapilord - HMV - SEXUAL'), duration: 190.8, vr: false }, library)).toBe(library[0]);
    expect(firstDuplicate({ words: titleWords('Agapilord - HMV - SEXUAL'), duration: 195, vr: false }, library)).toBeNull();
    expect(firstDuplicate({ words: titleWords('Agapilord - HMV - SEXUAL'), duration: 190, vr: true }, library)).toBeNull();
  });
});
