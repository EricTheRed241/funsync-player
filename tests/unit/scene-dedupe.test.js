/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run dedupe:scenes — which library videos are the same video, and which
// copy stays (scripts/scene-dedupe-lib.mjs). Frames are compared by an
// injected function; no ffmpeg here.
import { describe, it, expect } from 'vitest';
import { titleWords, titlesMatch, findDuplicates, compareScenes, firstDuplicate, scriptSpeed } from '../../scripts/scene-dedupe-lib.mjs';

const match = (a, b, artistA = '', artistB = '') => titlesMatch(titleWords(a, artistA), titleWords(b, artistB));

// A scene as the CLI builds it; flat 1080p60 EroScripts by default.
let n = 0;
const scene = (title, extra = {}) => ({
  video: `/lib/${title}/${title}-${n++}.mp4`, dir: `/lib/${title}`, duration: 200, width: 1920, height: 1080, fps: 60, bitrate: 5e6,
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
  it('groups the same video, keeps the higher resolution, and leaves character variants of one animation apart', async () => {
    const hd = scene('Hip Swing 1080p');
    const uhd = scene('Hip Swing 4k', { width: 3840, height: 2160 });
    const rina = scene('Rina - Lesson In Restraint');
    const yixuan = scene('Yixuan - Lesson In Restraint');
    expect(await kept([hd, uhd, rina, yixuan])).toEqual([[uhd.video, ['higher resolution']]]);
  });

  it('needs the same picture or the same script, the same length, and the same projection', async () => {
    expect(await kept([scene('Dance'), scene('Dance')], framesDiffer)).toEqual([]);
    const a = scene('Dance', { scriptHash: 'h' });
    const b = scene('Dance', { scriptHash: 'h', speed: 100 });
    expect(await kept([a, b], framesDiffer)).toEqual([[a.video, ['faster script']]]);
    expect(await kept([scene('Dance'), scene('Dance', { duration: 203 })])).toEqual([]);
    expect(await kept([scene('Dance'), scene('Dance', { width: 3840, height: 1920 })])).toEqual([]); // 2:1 4K is VR
    expect(await kept([scene('Dance'), scene('Dance', { width: 1080, height: 1920 })])).toEqual([]); // portrait
  });
});

describe('which copy stays', () => {
  const order = (a, b) => compareScenes(a, b);

  it('takes EroScripts over FapTap and hand-made axes over generated before any video quality', () => {
    const faptap4k = scene('X', { source: 'faptap', width: 3840, height: 2160 });
    const eroHd = scene('X');
    expect(order(eroHd, faptap4k)).toEqual({ order: expect.any(Number), reason: 'EroScripts over FapTap' });
    expect(order(eroHd, faptap4k).order).toBeLessThan(0);
    const generated4k = scene('X', { width: 3840, height: 2160 });
    const handmadeHd = scene('X', { handmadeAxes: 3 });
    expect(order(handmadeHd, generated4k).order).toBeLessThan(0);
  });

  it('takes passthrough, then stays within the VR caps (8K, 60 fps), then the highest quality, then the faster script', () => {
    const vr = { width: 7680, height: 3840, fps: 60 };
    expect(order(scene('Y', { ...vr, passthrough: true }), scene('Y', { ...vr, width: 8192, height: 4096 })).reason).toBe('passthrough');
    const sixK90 = scene('Z', { width: 5888, height: 2944, fps: 90 });
    const twelveK90 = scene('Z', { width: 11776, height: 5888, fps: 90 });
    expect(order(sixK90, twelveK90)).toEqual({ order: expect.any(Number), reason: 'within VR caps' });
    expect(order(sixK90, twelveK90).order).toBeLessThan(0);
    // Flat has no fps cap: 4K 120 fps beats 1080p 60 fps.
    expect(order(scene('F', { width: 3840, height: 2160, fps: 120 }), scene('F')).order).toBeLessThan(0);
    expect(order(scene('S', { speed: 250 }), scene('S', { speed: 200 })).reason).toBe('faster script');
  });

  it('measures script speed as travel per second', () => {
    expect(scriptSpeed([{ at: 0, pos: 0 }, { at: 500, pos: 100 }, { at: 1000, pos: 0 }])).toBe(200);
    expect(scriptSpeed([{ at: 0, pos: 50 }])).toBe(0);
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
