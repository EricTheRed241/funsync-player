/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// scripts/scene-probe.mjs — video facts for duplicate checks. VR comes from
// the backend's stereo check (vr-stereo-cache.json) when it has looked at the
// file, else from the name and the picture's shape. The probe cache is filled
// in advance, so no ffprobe runs here.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { libraryVideos, readStereoCache, stereoVr } from '../../scripts/scene-probe.mjs';

let tmp;
beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'scene-probe-')); });
afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

const stereoEntry = (stat, layout) => ({ size: stat.size, mtimeNs: Math.round(stat.mtimeMs * 1e6), layout, version: 1 });

describe('stereoVr', () => {
  it('answers from a current entry only', () => {
    const stat = { size: 10, mtimeMs: 1_700_000_000_123.5 };
    expect(stereoVr({ '/v.mp4': stereoEntry(stat, 'sbs') }, '/v.mp4', stat)).toBe(true);
    expect(stereoVr({ '/v.mp4': stereoEntry(stat, 'mono') }, '/v.mp4', stat)).toBe(false);
    expect(stereoVr({ '/v.mp4': stereoEntry(stat, null) }, '/v.mp4', stat)).toBeUndefined(); // undecided
    expect(stereoVr({ '/v.mp4': stereoEntry(stat, 'sbs') }, '/v.mp4', { ...stat, size: 11 })).toBeUndefined(); // changed since
    expect(stereoVr({ '/v.mp4': { ...stereoEntry(stat, 'sbs'), version: 2 } }, '/v.mp4', stat)).toBeUndefined();
    expect(stereoVr({}, '/v.mp4', stat)).toBeUndefined();
    // The app may have registered the other Unicode spelling of the path.
    expect(stereoVr({ '/Ne\u0302mesis.mp4': stereoEntry(stat, 'sbs') }, '/Nêmesis.mp4', stat)).toBe(true);
  });
});

describe('libraryVideos', () => {
  it('takes the stereo check over the name and the shape, and falls back to them without an entry', async () => {
    const dir = path.join(tmp, 'Lib', 'Artist', 'Scene');
    fs.mkdirSync(dir, { recursive: true });
    const files = {
      'TheChillPanda - VR PMV - Doja Cat.mp4': [1920, 1080], // named VR, one picture
      'DOOMSCROLL (5K60 21).mp4': [4320, 2160], // VR-shaped, one picture
      'Haley Heads Will Roll.mp4': [3840, 1920], // two eyes
      'Unchecked 2to1.mp4': [3840, 1920], // not looked at: the shape decides
      'Unchecked VR.mp4': [1920, 1080], // not looked at: the name decides
      '._Haley Heads Will Roll.mp4': [3840, 1920],
    };
    const probe = {};
    const stereo = {};
    for (const [name, [width, height]] of Object.entries(files)) {
      const f = path.join(dir, name);
      fs.writeFileSync(f, name);
      const stat = fs.statSync(f);
      probe[f] = { duration: 200, width, height, fps: 60, bitrate: 1, size: stat.size, mtimeMs: stat.mtimeMs };
    }
    const at = (name) => path.join(dir, name);
    const stat = (name) => fs.statSync(at(name));
    stereo[at('TheChillPanda - VR PMV - Doja Cat.mp4')] = stereoEntry(stat('TheChillPanda - VR PMV - Doja Cat.mp4'), 'mono');
    stereo[at('DOOMSCROLL (5K60 21).mp4')] = stereoEntry(stat('DOOMSCROLL (5K60 21).mp4'), 'mono');
    stereo[at('Haley Heads Will Roll.mp4')] = stereoEntry(stat('Haley Heads Will Roll.mp4'), 'sbs');
    fs.writeFileSync(path.join(tmp, 'vr-stereo-cache.json'), JSON.stringify({ version: 1, entries: stereo }));

    const videos = await libraryVideos([path.join(tmp, 'Lib')], probe, readStereoCache(tmp));
    const vr = Object.fromEntries(videos.map((v) => [path.basename(v.video), v.vr]));
    expect(vr).toEqual({
      'TheChillPanda - VR PMV - Doja Cat.mp4': false,
      'DOOMSCROLL (5K60 21).mp4': false,
      'Haley Heads Will Roll.mp4': true,
      'Unchecked 2to1.mp4': true,
      'Unchecked VR.mp4': true,
    });
    expect(readStereoCache(path.join(tmp, 'none'))).toEqual({});
  });
});
