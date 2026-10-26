/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// electron/library-scan.js is the scan-directory pairing, shared with CLI
// tools (npm run eroscripts:sync). Pins that it runs outside Electron on a
// real folder tree and pairs the way the app does.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const { scanLibrary } = await import('../../electron/library-scan.js');

let tmp;
beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'lib-scan-')); });
afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

function touch(rel) {
  const p = path.join(tmp, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, '');
  return p;
}

describe('scanLibrary', () => {
  it('pairs videos with scripts, variants and axis companions across sources', async () => {
    touch('A/Creator/Scene/Scene_hard.mp4');
    touch('A/Creator/Scene/Scene.mp4');
    touch('A/Creator/Scene/Scene.funscript');
    touch('A/Creator/Scene/Scene_hard.funscript');
    touch('A/Creator/Scene/Scene_hard.twist.funscript');
    touch('A/Creator/Scene/._Scene.funscript');
    touch('B/Other/Clip/Clip.mp4');
    touch('B/Other/Clip/Clip (Soft).funscript');
    touch('B/Other/Clip/Clip.funscript');

    const r = await scanLibrary([path.join(tmp, 'A'), path.join(tmp, 'B')], { [path.join(tmp, 'B')]: 'Bee' });
    const byName = Object.fromEntries(r.videos.map((v) => [v.name, v]));
    expect(byName['Scene_hard.mp4'].funscriptPath).toBe(path.join(tmp, 'A/Creator/Scene/Scene_hard.funscript'));
    expect(byName['Scene.mp4'].funscriptPath).toBe(path.join(tmp, 'A/Creator/Scene/Scene.funscript'));
    expect(byName['Clip.mp4'].variants.map((v) => v.label)).toEqual(['Default', 'Soft']);
    expect(byName['Clip.mp4'].sourceName).toBe('Bee');
    expect(r.unmatchedFunscripts).toEqual([]);
    expect(r.allFunscripts.some((f) => f.name.startsWith('._'))).toBe(false);
    expect(r.failedPaths).toEqual([]);
  });

  it("attaches a Discovery scene's post from its eroscripts.json, matching the recorded file", async () => {
    touch('In/Artist/Scene/Artist - Scene.mp4');
    touch('In/Artist/Pack/One.mp4');
    touch('In/Artist/Pack/Two.mp4');
    touch('In/Other/Plain.mp4');
    fs.writeFileSync(path.join(tmp, 'In/Artist/Scene/eroscripts.json'), JSON.stringify({ topicId: 7, url: 'https://x/t/7', tags: ['vr', 'creampie'], video: '3.mp4' }));
    fs.writeFileSync(path.join(tmp, 'In/Artist/Pack/eroscripts.json'), JSON.stringify({ topicId: 8, tags: [], file: 'two.MP4' }));

    const byName = Object.fromEntries((await scanLibrary(path.join(tmp, 'In'))).videos.map((v) => [v.name, v]));
    expect(byName['Artist - Scene.mp4'].eroscripts).toEqual({ topicId: 7, url: 'https://x/t/7', tags: ['vr', 'creampie'] });
    expect(byName['Two.mp4'].eroscripts).toEqual({ topicId: 8, url: null, tags: [] });
    expect(byName['One.mp4'].eroscripts).toBeUndefined();
    expect(byName['Plain.mp4'].eroscripts).toBeUndefined();
  });

  it('reports a missing source instead of throwing', async () => {
    const r = await scanLibrary([path.join(tmp, 'missing')]);
    expect(r.failedPaths).toEqual([path.join(tmp, 'missing')]);
    expect(r.videos).toEqual([]);
  });
});
