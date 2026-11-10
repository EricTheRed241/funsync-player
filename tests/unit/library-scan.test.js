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

  it("attaches a FapTap scene from its faptap.json, matching the recorded file", async () => {
    touch('In/Zoot/Yoga/Zoot - YOGA TIME with Bulma!.mp4');
    touch('In/Zoot/Yoga/Zoot - YOGA TIME with Bulma!.funscript');
    touch('In/Zoot/Other/Other.mp4');
    touch('In/Zoot/NoId/NoId.mp4');
    fs.writeFileSync(path.join(tmp, 'In/Zoot/Yoga/faptap.json'), JSON.stringify({
      faptapId: '2114528228239478785', url: 'https://faptap.net/v/2114528228239478785', title: 'YOGA TIME with Bulma!',
      author: 'Zoot', tags: ['animation', 'riding', '', 3], file: 'zoot - yoga time with bulma!.MP4',
    }));
    fs.writeFileSync(path.join(tmp, 'In/Zoot/Other/faptap.json'), JSON.stringify({ faptapId: '5', tags: ['riding'], file: 'Elsewhere.mp4' }));
    fs.writeFileSync(path.join(tmp, 'In/Zoot/NoId/faptap.json'), JSON.stringify({ tags: ['riding'] }));

    const byName = Object.fromEntries((await scanLibrary(path.join(tmp, 'In'))).videos.map((v) => [v.name, v]));
    const yoga = byName['Zoot - YOGA TIME with Bulma!.mp4'];
    expect(yoga.faptap).toEqual({ faptapId: '2114528228239478785', url: 'https://faptap.net/v/2114528228239478785', tags: ['animation', 'riding'] });
    expect(yoga.eroscripts).toBeUndefined();
    expect(byName['Other.mp4'].faptap).toBeUndefined();
    expect(byName['NoId.mp4'].faptap).toBeUndefined();
  });

  it("attaches FapTap's thumbnail URL only from FapTap's own hosts, over https", async () => {
    const urls = {
      Webp: 'https://faptap.net/api/assets/thumbnails/1.webp',
      Bunny: 'https://vz-5d2e7f1a-b3c.b-cdn.net/0a1b2c3d/thumbnail.jpg',
      Http: 'http://faptap.net/api/assets/thumbnails/3.webp',
      Elsewhere: 'https://evil.example/thumbnail.jpg',
      LookAlike: 'https://faptap.net.evil.example/x.webp',
      OtherZone: 'https://cdn.b-cdn.net/thumbnail.jpg',
      Port: 'https://faptap.net:8443/api/assets/thumbnails/7.webp',
      NotString: 42,
    };
    for (const [name, thumbnailUrl] of Object.entries(urls)) {
      touch(`In/A/${name}/${name}.mp4`);
      fs.writeFileSync(path.join(tmp, `In/A/${name}/faptap.json`), JSON.stringify({ faptapId: name, tags: [], thumbnailUrl }));
    }
    const byName = Object.fromEntries((await scanLibrary(path.join(tmp, 'In'))).videos.map((v) => [v.name, v]));
    expect(byName['Webp.mp4'].faptap.thumbnailUrl).toBe(urls.Webp);
    expect(byName['Bunny.mp4'].faptap.thumbnailUrl).toBe(urls.Bunny);
    for (const name of ['Http', 'Elsewhere', 'LookAlike', 'OtherZone', 'Port', 'NotString']) {
      expect(byName[`${name}.mp4`].faptap).toEqual({ faptapId: name, url: null, tags: [] });
    }
  });

  it("attaches a rule34video cover from eroscripts.json only when it is exactly a cover link", async () => {
    const urls = {
      Cover: 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg',
      Small: 'https://rule34video.com/contents/videos_screenshots/0/42/preview.jpg',
      Http: 'http://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg',
      Www: 'https://www.rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg',
      LookAlike: 'https://rule34video.com.evil.example/contents/videos_screenshots/3065000/3065491/preview.jpg',
      Port: 'https://rule34video.com:8443/contents/videos_screenshots/3065000/3065491/preview.jpg',
      Creds: 'https://u@rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg',
      Query: 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg?x=1',
      WrongFolder: 'https://rule34video.com/contents/videos_screenshots/3064000/3065491/preview.jpg',
      OtherFile: 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/1.jpg',
      Traversal: 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/../preview.jpg',
      Zero: 'https://rule34video.com/contents/videos_screenshots/0/0/preview.jpg',
      Padded: 'https://rule34video.com/contents/videos_screenshots/3065000/03065491/preview.jpg',
      NotString: 42,
    };
    for (const [name, posterUrl] of Object.entries(urls)) {
      touch(`In/A/${name}/${name}.mp4`);
      fs.writeFileSync(path.join(tmp, `In/A/${name}/eroscripts.json`), JSON.stringify({ topicId: 9, tags: [], posterUrl }));
    }
    const byName = Object.fromEntries((await scanLibrary(path.join(tmp, 'In'))).videos.map((v) => [v.name, v]));
    expect(byName['Cover.mp4'].eroscripts).toEqual({ topicId: 9, url: null, tags: [], posterUrl: urls.Cover });
    expect(byName['Small.mp4'].eroscripts.posterUrl).toBe(urls.Small);
    for (const name of Object.keys(urls).filter((n) => n !== 'Cover' && n !== 'Small')) {
      expect(byName[`${name}.mp4`].eroscripts).toEqual({ topicId: 9, url: null, tags: [] });
    }
  });

  it("reports each video's size on disk (the Overview's capacity total)", async () => {
    fs.writeFileSync(touch('In/Artist/Scene/Big.mp4'), Buffer.alloc(4096));
    touch('In/Artist/Scene/Empty.mp4');
    const byName = Object.fromEntries((await scanLibrary(path.join(tmp, 'In'))).videos.map((v) => [v.name, v]));
    expect(byName['Big.mp4'].size).toBe(4096);
    expect(byName['Empty.mp4'].size).toBe(0);
  });

  it('reports a missing source instead of throwing', async () => {
    const r = await scanLibrary([path.join(tmp, 'missing')]);
    expect(r.failedPaths).toEqual([path.join(tmp, 'missing')]);
    expect(r.videos).toEqual([]);
  });

  it('logs overlapping-source duplicates and skipped AppleDouble files as separate counts', async () => {
    touch('A/Creator/Scene/Scene.mp4');
    touch('A/Creator/Scene/Scene.funscript');
    touch('A/Creator/Scene/._Scene.mp4');
    touch('A/Creator/Scene/._Scene.funscript');
    const lines = [];
    const log = { info: (m) => lines.push(m), warn: (m) => lines.push(m) };
    await scanLibrary([path.join(tmp, 'A'), path.join(tmp, 'A/Creator')], {}, { log });
    // Creator/, Scene/ and Scene's two files are 4 entries; the second source lists 3 of them again.
    expect(lines[0]).toMatch(/Scanned 4 unique entries \(3 duplicates dropped from overlapping sources, 4 AppleDouble \._ files skipped\)/);
    lines.length = 0;
    await scanLibrary([path.join(tmp, 'A')], {}, { log });
    expect(lines[0]).toMatch(/Scanned 4 entries \(2 AppleDouble \._ files skipped\)/);
  });
});
