/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Frames for the swipe deck (scripts/scout-frames.mjs, npm run eroscripts:scout -- --frames):
// the one-eye crop per video name, the frame times, which free source each host gives (never
// SLR's account download or VRBangers' paid video), which waiting posts get frames (unswiped
// only) and in what order, the frames file (records, migration from the scan state and from
// folders on disk), the ffmpeg arguments (fake runner), and real ffmpeg runs on a generated
// test video through the local range proxy. No network: hosts are fakes.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { Readable } from 'node:stream';
import { afterEach, beforeAll, describe, it, expect } from 'vitest';
import {
  stereoLayout, layoutFromSize, frameFilter, frameTimes, frameArgs, imageArgs, probeArgs, parseProbe, imageKey, usableImages,
  imageCounts, framesQueue, frameSource, frameScene, pickVariant, minHeightFor, slrTrailer, vrbangersGallery, vrbangersPageImages,
  recordFrames, recordFramesFailed, recentlyFailed, postFramesDir, grabFrames, grabImages, startRangeProxy, httpSource, megaSource,
  readerFailure, postFrames, FrameError, FRAME_POINTS, FAILED_RETRY_MS, MAX_FRAME_WIDTH, SHARED_IMAGE_POSTS, FRAMES_WANTED,
  jpegSize, isBlankFrame, FRAMES_FILE, emptyFramesFile, normalizeFramesFile, readFramesFile, writeFramesFile, framesOnDisk, migrateFrames,
} from '../../scripts/scout-frames.mjs';
import { markTopic, emptyScoutState, dropFrameRecords } from '../../scripts/eroscripts-scout-lib.mjs';

const temps = [];
const tempDir = () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), 'scout-frames-'));
  temps.push(d);
  return d;
};
afterEach(() => { while (temps.length) fs.rmSync(temps.pop(), { recursive: true, force: true }); });

describe('stereoLayout: which part of the picture a frame keeps', () => {
  it.each([
    ['META-003-1-Airi Tsugumoto_180_3DH.mp4', true, 'sbs'],
    ['VRBANGERS_queen_of_sticks_8K_180x180_3dh.mp4', true, 'sbs'],
    ['Miss Lexa - Scene (Miss Lexa VR)_180_LR.mp4', true, 'sbs'],
    ['Scene_180_LR.mp4', false, 'sbs'],
    ['Scene SBS 4K.mp4', false, 'sbs'],
    ['JAV_MKX200.mp4', true, 'sbs'],
    ['JAV_FISHEYE190.mp4', true, 'sbs'],
    ['Scene side-by-side.mp4', false, 'sbs'],
    ['Scene_LR.mp4', true, 'sbs'],
    ['Scene_180_TB.mp4', true, 'tb'],
    ['Scene_360_TB.mp4', false, 'tb'],
    ['Scene_3dv.mp4', true, 'tb'],
    ['Scene OverUnder.mp4', false, 'tb'],
    ['Scene_TB.mp4', true, 'tb'],
    ['Scene_MONO360.mp4', true, 'mono'],
    ['Scene_MONO180_LR.mp4', true, 'mono'],
    ['Mona Cowgirl [Theobrobine] 1080p.mp4', false, 'flat'],
    // Short tokens are words in titles: only a VR video's name counts them.
    ["Je t'aime ou pas.mp4", false, 'flat'],
    ['Scene LR edit.mp4', false, 'flat'],
    // VR without a token: the frame size decides.
    ['Some VR scene.mp4', true, null],
  ])('%s (vr %s) → %s', (name, vr, layout) => {
    expect(stereoLayout(name, { vr })).toBe(layout);
  });

  it('the frame size decides for VR without a token', () => {
    expect(layoutFromSize(5760, 2880)).toBe('sbs');
    expect(layoutFromSize(2048, 4096)).toBe('tb');
    expect(layoutFromSize(4096, 4096)).toBe('mono');
    expect(layoutFromSize(0, 0)).toBe('mono');
  });

  it('crops the left half or the top half, and caps the width', () => {
    expect(frameFilter('sbs')).toBe(`crop=iw/2:ih:0:0,scale='min(${MAX_FRAME_WIDTH},iw)':-2`);
    expect(frameFilter('tb')).toBe(`crop=iw:ih/2:0:0,scale='min(${MAX_FRAME_WIDTH},iw)':-2`);
    expect(frameFilter('mono')).toBe(`scale='min(${MAX_FRAME_WIDTH},iw)':-2`);
    expect(frameFilter('flat')).toBe(frameFilter('mono'));
  });
});

describe('frameTimes', () => {
  it('takes 25, 50 and 75 % of the length, in seconds', () => {
    expect(FRAME_POINTS).toEqual([25, 50, 75]);
    expect(frameTimes(400_000)).toEqual([100, 200, 300]);
    expect(frameTimes(91_400)).toEqual([22.85, 45.7, 68.55]);
    expect(frameTimes(1001)).toEqual([0.25, 0.501, 0.751]);
  });
  it('none without a length', () => {
    expect(frameTimes(0)).toEqual([]);
    expect(frameTimes(null)).toEqual([]);
    expect(frameTimes(NaN)).toEqual([]);
  });
});

describe('ffmpeg arguments', () => {
  it('seeks before opening the input and takes one keyframe', () => {
    const args = frameArgs({ input: 'http://127.0.0.1:9/video', seekSec: 45.7, out: '/t/25.jpg.tmp', layout: 'sbs', http: true });
    const at = (a) => args.indexOf(a);
    expect(at('-ss')).toBeGreaterThan(-1);
    expect(at('-ss')).toBeLessThan(at('-i'));
    expect(at('-noaccurate_seek')).toBeLessThan(at('-i'));
    expect(args[at('-seekable') + 1]).toBe('1');
    expect(at('-seekable')).toBeLessThan(at('-i'));
    expect(args[at('-ss') + 1]).toBe('45.7');
    expect(args[at('-i') + 1]).toBe('http://127.0.0.1:9/video');
    expect(args[at('-frames:v') + 1]).toBe('1');
    expect(args[at('-vf') + 1]).toBe(frameFilter('sbs'));
    expect(args.slice(-3)).toEqual(['-f', 'mjpeg', '/t/25.jpg.tmp']);
    expect(args).toContain('-nostdin');
  });
  it('a local file has no -seekable; headers go as CRLF lines before the input', () => {
    const local = frameArgs({ input: '/v.mp4', seekSec: 1, out: '/o', layout: 'flat' });
    expect(local).not.toContain('-seekable');
    expect(local).not.toContain('-headers');
    const h = frameArgs({ input: 'https://x/v.mp4', seekSec: 1, out: '/o', layout: 'flat', http: true, headers: { Referer: 'https://r/', 'User-Agent': 'UA' } });
    expect(h[h.indexOf('-headers') + 1]).toBe('Referer: https://r/\r\nUser-Agent: UA\r\n');
    expect(h.indexOf('-headers')).toBeLessThan(h.indexOf('-i'));
  });
  it('stills are scaled, not cropped; ffprobe asks for length and size', () => {
    const img = imageArgs({ input: 'https://c/x.jpg', out: '/o' });
    expect(img[img.indexOf('-vf') + 1]).toBe(frameFilter('flat'));
    expect(img).not.toContain('-ss');
    const p = probeArgs({ input: 'http://127.0.0.1:1/video', http: true });
    expect(p).toContain('format=duration:stream=width,height');
    expect(p.indexOf('-seekable')).toBeLessThan(p.indexOf('http://127.0.0.1:1/video'));
    expect(parseProbe('{"streams":[{"width":3840,"height":1920}],"format":{"duration":"91.4"}}')).toEqual({ lengthMs: 91_400, width: 3840, height: 1920 });
    expect(parseProbe('garbage')).toEqual({ lengthMs: null, width: null, height: null });
  });
});

describe('frameSource: free sources only', () => {
  const post = (video, extra = {}) => ({ id: 1, vr: false, scenes: [{ base: 'b', scripts: [], video, durationMs: 120_000, ...extra }] });

  it('forum attachment: the file itself, the session for the forum only', () => {
    const p = frameSource(post({ name: 'Scene.mp4', host: 'forum', url: 'https://discuss.eroscripts.com/uploads/short-url/x.mp4' }));
    expect(p).toMatchObject({ kind: 'http', host: 'forum', forum: true, url: 'https://discuss.eroscripts.com/uploads/short-url/x.mp4', lengthMs: 120_000, layout: 'flat' });
  });
  it('pixeldrain: the file API without ?download', () => {
    const p = frameSource(post({ name: 'Scene.mp4', host: 'pixeldrain', url: 'https://pixeldrain.com/api/file/NAoQTdKz?download' }));
    expect(p).toMatchObject({ kind: 'http', host: 'pixeldrain', url: 'https://pixeldrain.com/api/file/NAoQTdKz' });
  });
  it('rule34video: the page (read again for a fresh link), its length', () => {
    const p = frameSource(post({ name: 'Mona 1080p.mp4', host: 'rule34video', url: 'https://rule34video.com/video/3066425/mona/', height: 1080, duration: 91 }));
    expect(p).toMatchObject({ kind: 'rule34video', url: 'https://rule34video.com/video/3066425/mona/', lengthMs: 91_000 });
  });
  it('MEGA: the file in its folder, the VR layout from the name', () => {
    const c = { id: 2, vr: true, scenes: [{ scripts: [], video: { name: 'META-003_180_3DH.mp4', host: 'mega', url: 'https://mega.nz/folder/X#k', remote: 'META-003_180_3DH.mp4', size: 7e8 }, durationMs: 427_918 }] };
    expect(frameSource(c)).toMatchObject({ kind: 'mega', remote: 'META-003_180_3DH.mp4', layout: 'sbs', lengthMs: 427_918 });
  });
  it('SexLikeReal: the scene trailer, never the account download; its length is probed', () => {
    const p = frameSource(post({ name: 'Miss Lexa - X (Miss Lexa VR)_180_LR.mp4', host: 'slr', slrSceneId: 52317, vr: true, url: 'https://www.sexlikereal.com/scenes/x-52317', duration: 223 }));
    expect(p).toMatchObject({ kind: 'slr-trailer', sceneId: 52317, layout: 'sbs', lengthMs: null });
  });
  it('VRBangers: the public page stills, never the paid video', () => {
    const p = frameSource(post({ name: 'VRBANGERS_q_8K_180x180_3dh.mp4', host: 'vrbangers', url: 'https://vrbangers.com/video/queen-of-sticks/', slug: 'queen-of-sticks' }));
    expect(p).toMatchObject({ kind: 'vrbangers-gallery', slug: 'queen-of-sticks' });
  });
  it('skips archives, posts without a video and unknown hosts', () => {
    expect(frameSource({ id: 3, scenes: [{ packed: true, video: { name: 'pack.rar', host: 'pixeldrain' } }] })).toEqual({ skip: 'archive' });
    expect(frameSource({ id: 3, scenes: [] })).toEqual({ skip: 'no video' });
    expect(frameSource(post({ name: 'x.mp4', host: 'gofile', url: 'https://gofile.io/d/x' }))).toEqual({ skip: 'gofile' });
  });
  it('the first scene with a video', () => {
    const c = { scenes: [{ packed: true, video: { name: 'a.rar', host: 'mega' } }, { video: { name: 'b.mp4', host: 'mega' } }, { video: { name: 'c.mp4', host: 'mega' } }] };
    expect(frameScene(c).video.name).toBe('b.mp4');
  });
});

describe('quality choice', () => {
  const v = (...hs) => hs.map((height) => ({ height, direct: `https://d/${height}.mp4` }));
  it('the lowest quality at least 480p, else the highest there is', () => {
    expect(pickVariant(v(1080, 360, 480, 720)).height).toBe(480);
    expect(pickVariant(v(240, 360)).height).toBe(360);
    expect(pickVariant([])).toBeNull();
    expect(pickVariant(v(480, 720, 1080), { minHeight: minHeightFor('sbs') }).height).toBe(720);
    expect(minHeightFor('flat')).toBe(480);
    expect(minHeightFor(null)).toBe(720);
  });

  const scene = {
    fullVideoLength: 223,
    trailerEncodings: [
      { name: 'h265', videoSources: [{ resolution: 720, height: 720, url: 'https://cdn/h265_720.mp4' }] },
      { name: 'h264', videoSources: [
        { resolution: 480, height: 480, url: 'https://cdn/h264_480.mp4' },
        { resolution: 720, height: 720, url: 'https://cdn/h264_720.mp4' },
        { resolution: 1080, height: 1080, url: 'https://cdn/h264_1080.mp4' },
      ] },
    ],
    videoPreview: 'https://cdn/preview/14x1/52317_300p.mp4',
  };
  it('SLR: an h264 trailer encoding, 720p for stereo, 480p flat', () => {
    expect(slrTrailer(scene)).toEqual({ url: 'https://cdn/h264_720.mp4', height: 720, kind: 'trailer' });
    expect(slrTrailer(scene, { minHeight: 480 }).url).toBe('https://cdn/h264_480.mp4');
  });
  it('SLR: the preview clip when there is no trailer; null with neither', () => {
    expect(slrTrailer({ trailerEncodings: [], videoPreview: scene.videoPreview })).toEqual({ url: scene.videoPreview, height: 300, kind: 'preview' });
    expect(slrTrailer({ trailerEncodings: [], videoPreview: null })).toBeNull();
    expect(slrTrailer(null)).toBeNull();
  });
});

describe('vrbangersGallery', () => {
  const img = (n) => ({
    permalink: `/uploads/${n}.jpg`,
    previews: [{ sizeAlias: 'XL', width: 2000, permalink: `/uploads/${n}_XL.jpg` }, { sizeAlias: 'L', width: 1400, permalink: `/uploads/${n}_L.jpg` }, { sizeAlias: 'M', width: 1000, permalink: `/uploads/${n}_M.jpg` }],
  });
  it('three stills spread over the gallery, the largest preview that fits', () => {
    const urls = vrbangersGallery({ galleryImages: Array.from({ length: 12 }, (_, i) => img(i)) });
    expect(urls).toEqual([3, 6, 9].map((n) => `https://content.vrbangers.com/uploads/${n}_M.jpg`));
  });
  it('a short gallery whole; without one, the poster', () => {
    expect(vrbangersGallery({ galleryImages: [img(0), img(1)] })).toHaveLength(2);
    expect(vrbangersGallery({ galleryImages: [], poster: { permalink: '/uploads/p.jpg', previews: [] } })).toEqual(['https://content.vrbangers.com/uploads/p.jpg']);
    expect(vrbangersGallery({})).toEqual([]);
  });
  it('a page without scene data has none', () => {
    expect(vrbangersPageImages('<html></html>', 'x')).toEqual([]);
  });
});

describe('which posts get frames', () => {
  const sha = (ch) => ch.repeat(40);
  const up = (key) => `https://cdn/original/4X/a/b/${key}.jpeg`;
  const NOW = Date.parse('2026-11-14T12:00:00Z');
  const cand = (id, score, extra = {}) => ({ id, score, scenes: [{ video: { name: `${id}.mp4`, host: 'mega' } }], ...extra });
  const state = () => ({
    topics: {
      // a banner shared by 3 posts: never counts
      1: { images: [up(sha('a'))] },
      2: { images: [up(sha('a')), up(sha('b'))] },
      3: { images: [up(sha('a')), up(sha('c')), up(sha('d')), up(sha('e'))] },
      4: { images: [] },
      5: { images: [up(sha('f')), up(sha('f')), 'http://insecure/x.jpg'] },
      6: { image: up(sha('g')) },
      // Where --frames kept its records before the frames file: ignored now.
      7: { images: [], framesFailed: { at: new Date(NOW).toISOString(), reason: 'old place' } },
      8: { images: [] },
      9: { images: [] },
      10: { images: [] },
      11: { images: [] },
      13: { images: [] },
    },
  });
  const framesFile = () => ({
    version: 1,
    topics: {
      7: { frames: { at: '2026-11-01T00:00:00Z', source: 'mega: 7.mp4', files: ['25.jpg'] } },
      8: { framesFailed: { at: new Date(NOW - FAILED_RETRY_MS + 3600_000).toISOString(), reason: 'pixeldrain: HTTP 403' } },
      9: { framesFailed: { at: new Date(NOW - FAILED_RETRY_MS - 1).toISOString(), reason: 'mega: bandwidth limit' } },
    },
  });

  it('usable images: https, one per upload, none 3+ posts share', () => {
    const s = state();
    const counts = imageCounts(s);
    expect(SHARED_IMAGE_POSTS).toBe(3);
    expect(counts.get(sha('a'))).toBe(3);
    expect(usableImages(s.topics[1], counts)).toEqual([]);
    expect(usableImages(s.topics[3], counts)).toHaveLength(3);
    expect(usableImages(s.topics[5], counts)).toEqual([up(sha('f'))]);
    expect(usableImages(s.topics[6], counts)).toEqual([up(sha('g'))]);
    expect(imageKey('https://cdn/x/notasha.png')).toBe('https://cdn/x/notasha.png');
  });

  it('picks unswiped posts with fewer than 3 images, no frames, no recent failure, not in the library', () => {
    expect(FRAMES_WANTED).toBe(3);
    const candidates = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13].map((id) => cand(id, id));
    const q = framesQueue(candidates, { state: state(), framesFile: framesFile(), swiped: new Set([10, 13]), inLibrary: (c) => c.id === 11, now: NOW });
    const ids = q.map((x) => x.c.id);
    // 3: 3 usable images; 7: has frames; 8: failed within 7 days; 10, 13: swiped (left or right); 11: in the library; 12: no state record.
    expect(ids.sort((a, b) => a - b)).toEqual([1, 2, 4, 5, 6, 9]);
  });

  it('a post swiped either way gets none; one not swiped does', () => {
    const candidates = [cand(4, 1), cand(8, 1), cand(10, 1)];
    const swiped = new Set([4, 10]);
    expect(framesQueue(candidates, { state: state(), swiped, now: NOW }).map((x) => x.c.id)).toEqual([8]);
  });

  it('only the frames file counts what was done, not the scan state', () => {
    // Without a frames file, 7's state record (the old place) doesn't keep it out, and 8 and 9 have no failure.
    const q = framesQueue([cand(7, 1), cand(8, 1), cand(9, 1)], { state: state(), now: NOW });
    expect(q.map((x) => x.c.id).sort((a, b) => a - b)).toEqual([7, 8, 9]);
  });

  it('order: no images at all first, then score, then newest; a swipe right no longer goes first', () => {
    const candidates = [cand(1, 50), cand(2, 1), cand(4, 10), cand(5, 99), cand(6, 99), cand(9, 10)];
    const q = framesQueue(candidates, { state: state(), framesFile: framesFile(), now: NOW });
    // No images (1, 4, 9: score 50, 10, 10 → 9 newer than 4); then 5, 6 and 2 (score 99, 99, 1; newest first).
    expect(q.map((x) => x.c.id)).toEqual([1, 9, 4, 6, 5, 2]);
    expect(q.find((x) => x.c.id === 5).images).toBe(1);
  });

  it('a candidate without a score sorts last among its kind', () => {
    const q = framesQueue([cand(4, undefined), cand(10, 1)], { state: state(), now: NOW });
    expect(q.map((x) => x.c.id)).toEqual([10, 4]);
  });
});

describe('the frames file', () => {
  const t = Date.parse('2026-11-14T00:00:00Z');

  it('a post holds frames or a failure, never both; the 7-day retry window', () => {
    const file = emptyFramesFile();
    recordFramesFailed(file, 5, 'pixeldrain: HTTP 403', t);
    expect(file.topics[5]).toEqual({ framesFailed: { at: '2026-11-14T00:00:00.000Z', reason: 'pixeldrain: HTTP 403' } });
    expect(recentlyFailed(file.topics[5], t + FAILED_RETRY_MS - 1)).toBe(true);
    expect(recentlyFailed(file.topics[5], t + FAILED_RETRY_MS)).toBe(false);
    recordFrames(file, 5, { source: 'mega: x.mp4', files: ['25.jpg', '75.jpg'], now: t + 1000 });
    expect(file).toEqual({ version: 1, topics: { 5: { frames: { at: '2026-11-14T00:00:01.000Z', source: 'mega: x.mp4', files: ['25.jpg', '75.jpg'] } } } });
    recordFramesFailed(file, 5, 'mega: gone', t + 2000);
    expect(file.topics[5]).toEqual({ framesFailed: { at: '2026-11-14T00:00:02.000Z', reason: 'mega: gone' } });
  });

  it('is read back as written (atomically, no temporary file left); missing or foreign reads as empty', () => {
    const ud = tempDir();
    expect(FRAMES_FILE).toBe('eroscripts-scout-frames.json');
    expect(readFramesFile(ud)).toEqual(emptyFramesFile());
    const file = emptyFramesFile();
    recordFrames(file, 7, { source: 'forum: a.mp4', files: ['50.jpg'], now: t });
    writeFramesFile(ud, file);
    expect(readFramesFile(ud)).toEqual(file);
    expect(fs.readdirSync(ud)).toEqual([FRAMES_FILE]);
    for (const raw of [null, { version: 2, topics: {} }, { version: 1, topics: [] }, { version: 1 }]) expect(normalizeFramesFile(raw)).toEqual(emptyFramesFile());
    fs.writeFileSync(path.join(ud, FRAMES_FILE), '{not json');
    expect(readFramesFile(ud)).toEqual(emptyFramesFile());
  });

  it('takes in the scan state\'s old records, then folders on disk without a record; the state stays as it was', () => {
    const ud = tempDir();
    const put = (id, names) => {
      const dir = postFramesDir(ud, id);
      fs.mkdirSync(dir, { recursive: true });
      for (const n of names) fs.writeFileSync(path.join(dir, n), 'jpeg');
    };
    put(1, ['25.jpg', '50.jpg', '75.jpg']); // also recorded in the state: the state's record wins
    put(4, ['75.jpg', '25.jpg', '50.jpg.123.tmp', 'x.jpg']); // only on disk: its frames, in point order
    put(5, []); // an empty folder: nothing
    put(6, ['50.jpg']); // the file already has a record
    fs.mkdirSync(path.join(ud, 'scout-frames', 'eroscripts', 'notes'), { recursive: true });
    fs.writeFileSync(path.join(ud, 'scout-frames', 'eroscripts', '._7'), 'x');
    const onDisk = framesOnDisk(ud);
    expect(onDisk.map((d) => [d.id, d.files])).toEqual(expect.arrayContaining([[1, ['25.jpg', '50.jpg', '75.jpg']], [4, ['25.jpg', '75.jpg']], [6, ['50.jpg']]]));
    expect(onDisk).toHaveLength(3);
    expect(Number.isFinite(Date.parse(onDisk[0].at))).toBe(true);

    const state = emptyScoutState();
    state.topics[1] = { verdict: 'candidate', frames: { at: 'a', source: 'mega: 1.mp4', files: ['25.jpg', '50.jpg', '75.jpg'] } };
    state.topics[2] = { verdict: 'candidate', framesFailed: { at: 'b', reason: 'pixeldrain: HTTP 403' } };
    state.topics[3] = { verdict: 'candidate', frames: { at: 'c', source: 's', files: ['25.jpg'] }, framesFailed: { at: 'd', reason: 'r' } };
    state.topics[6] = { verdict: 'candidate', framesFailed: { at: 'e', reason: 'old' } };
    state.topics[8] = { verdict: 'no-video' };
    const before = JSON.stringify(state);
    const file = emptyFramesFile();
    file.topics[6] = { frames: { at: 'f', source: 'forum: 6.mp4', files: ['50.jpg'] } };

    expect(migrateFrames(file, { state, onDisk })).toEqual({ fromState: 3, fromDisk: 1 });
    expect(file.topics).toEqual({
      1: { frames: { at: 'a', source: 'mega: 1.mp4', files: ['25.jpg', '50.jpg', '75.jpg'] } },
      2: { framesFailed: { at: 'b', reason: 'pixeldrain: HTTP 403' } },
      3: { frames: { at: 'c', source: 's', files: ['25.jpg'] } },
      4: { frames: { at: onDisk.find((d) => d.id === 4).at, source: '', files: ['25.jpg', '75.jpg'] } },
      6: { frames: { at: 'f', source: 'forum: 6.mp4', files: ['50.jpg'] } },
    });
    expect(JSON.stringify(state)).toBe(before);
    // A second run finds nothing new.
    expect(migrateFrames(file, { state, onDisk })).toEqual({ fromState: 0, fromDisk: 0 });
  });

  it('a new verdict no longer carries frames, and a saved state drops the old copies', () => {
    const state = emptyScoutState();
    state.topics[5] = { verdict: 'candidate', rules: 1, at: 'x', title: 'T', frames: { at: 'a', source: 's', files: ['25.jpg'] }, framesFailed: { at: 'b', reason: 'r' } };
    state.topics[6] = { verdict: 'candidate', rules: 1, at: 'x', title: 'U', framesFailed: { at: 'b', reason: 'r' } };
    markTopic(state, 5, 'candidate');
    expect(state.topics[5]).not.toHaveProperty('frames');
    expect(state.topics[5]).not.toHaveProperty('framesFailed');
    expect(state.topics[5].title).toBe('T');
    dropFrameRecords(state);
    expect(state.topics[6]).toEqual({ verdict: 'candidate', rules: 1, at: 'x', title: 'U' });
  });

  it('frames folder per post', () => {
    expect(postFramesDir('/ud', 348348)).toBe(path.join('/ud', 'scout-frames', 'eroscripts', '348348'));
  });
});

describe('grabFrames with a fake runner', () => {
  it('one ffmpeg per point, written through a temporary file; a failed one is left out', async () => {
    const dir = tempDir();
    fs.writeFileSync(path.join(dir, '50.jpg'), 'stale');
    const calls = [];
    const run = async (cmd, args, o) => {
      calls.push({ cmd, args, o });
      const out = args[args.length - 1];
      if (calls.length === 2) return { code: 1, stdout: '', stderr: 'Invalid data found\n', timedOut: false };
      fs.writeFileSync(out, 'jpeg');
      return { code: 0, stdout: '', stderr: '', timedOut: false };
    };
    const r = await grabFrames({ input: 'http://127.0.0.1:1/video', http: true, lengthMs: 200_000, layout: 'tb', dir, run, timeoutMs: 1234 });
    expect(r).toEqual({ files: ['25.jpg', '75.jpg'], errors: ['Invalid data found'] });
    expect(calls.map((c) => c.cmd)).toEqual(['ffmpeg', 'ffmpeg', 'ffmpeg']);
    expect(calls.map((c) => c.args[c.args.indexOf('-ss') + 1])).toEqual(['50', '100', '150']);
    expect(calls.every((c) => c.args[c.args.indexOf('-vf') + 1] === frameFilter('tb') && c.o.timeoutMs === 1234)).toBe(true);
    expect(calls[0].args.at(-1)).toMatch(/25\.jpg\.\d+\.tmp$/);
    expect(fs.readdirSync(dir).sort()).toEqual(['25.jpg', '75.jpg']);
  });
  /** A stand-in JPEG: a frame header of `width`×`height`, padded to `bytes`. */
  const fakeJpeg = (width, height, bytes) => {
    const b = Buffer.alloc(bytes);
    Buffer.from([0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, height >> 8, height & 255, width >> 8, width & 255]).copy(b);
    return b;
  };
  it('a blank frame is taken again 5 % later, then 5 % earlier; all blank, none kept', async () => {
    expect(jpegSize(fakeJpeg(1280, 720, 100))).toEqual({ width: 1280, height: 720 });
    expect(isBlankFrame(fakeJpeg(1280, 720, 5600))).toBe(true);
    expect(isBlankFrame(fakeJpeg(1280, 720, 30_000))).toBe(false);
    expect(isBlankFrame(Buffer.from('jpeg'))).toBe(false);
    const dir = tempDir();
    const seeks = [];
    const run = async (cmd, args) => {
      const t = Number(args[args.indexOf('-ss') + 1]);
      seeks.push(t);
      // 25 % is a black fade, 30 % isn't; 75 % and around it all black.
      fs.writeFileSync(args.at(-1), fakeJpeg(1280, 720, t === 25 || t >= 70 ? 4000 : 40_000));
      return { code: 0 };
    };
    const r = await grabFrames({ input: 'x', lengthMs: 100_000, layout: 'flat', dir, run });
    expect(seeks).toEqual([25, 30, 50, 75, 80, 70]);
    expect(r).toEqual({ files: ['25.jpg', '50.jpg'], errors: ['blank frame'] });
    expect(fs.readFileSync(path.join(dir, '25.jpg')).length).toBe(40_000);
  });
  it('a timeout is a reason', async () => {
    const dir = tempDir();
    const r = await grabFrames({ input: 'x', lengthMs: 4000, layout: 'flat', dir, run: async () => ({ code: -1, stderr: '', timedOut: true }) });
    expect(r).toEqual({ files: [], errors: ['timed out', 'timed out', 'timed out'] });
  });
  it('stills: one ffmpeg per image', async () => {
    const dir = tempDir();
    const seen = [];
    const run = async (cmd, args) => { seen.push(args[args.indexOf('-i') + 1]); fs.writeFileSync(args.at(-1), 'j'); return { code: 0 }; };
    const r = await grabImages({ urls: ['https://a/1.jpg', 'https://a/2.jpg', 'https://a/3.jpg', 'https://a/4.jpg'], dir, run });
    expect(r.files).toEqual(['25.jpg', '50.jpg', '75.jpg']);
    expect(seen).toEqual(['https://a/1.jpg', 'https://a/2.jpg', 'https://a/3.jpg']);
  });
});

/** A fake fetch serving `file` with range requests (HTTP 206), or `status` for every request. */
const fileFetch = (file, { status = null, log = [] } = {}) => async (url, init = {}) => {
  log.push({ url, headers: { ...(init.headers || {}) } });
  if (status) return new Response(null, { status });
  const size = fs.statSync(file).size;
  const m = /bytes=(\d+)-(\d*)/.exec(init.headers?.Range || '');
  const start = m ? Number(m[1]) : 0;
  const end = m && m[2] ? Math.min(Number(m[2]), size - 1) : size - 1;
  return new Response(Readable.toWeb(fs.createReadStream(file, { start, end })), {
    status: m ? 206 : 200,
    headers: { 'content-length': String(end - start + 1), ...(m ? { 'content-range': `bytes ${start}-${end}/${size}` } : {}) },
  });
};

describe('readers', () => {
  it('a pixeldrain 403 is its hotlink protection; MEGA bandwidth stops MEGA', async () => {
    await expect(httpSource({ url: 'https://pixeldrain.com/api/file/x', host: 'pixeldrain', fetch: fileFetch(null, { status: 403 }) }))
      .rejects.toMatchObject({ reason: expect.stringMatching(/^pixeldrain: HTTP 403 \(hotlink protection/) });
    const f = readerFailure('mega', new Error('Bandwidth limit reached: 1234 seconds until it resets'));
    expect(f).toBeInstanceOf(FrameError);
    expect(f.stopHost).toBe(true);
    expect(f.reason).toMatch(/^mega: bandwidth limit/);
    expect(readerFailure('mega', new Error('EAGAIN')).stopHost).toBe(false);
  });
  it('a host that ignores ranges is refused', async () => {
    const dir = tempDir();
    const f = path.join(dir, 'v.bin');
    fs.writeFileSync(f, Buffer.alloc(100));
    const noRange = async () => new Response('x'.repeat(100), { status: 200 });
    await expect(httpSource({ url: 'https://h/v', host: 'forum', fetch: noRange })).rejects.toMatchObject({ reason: 'forum: no range requests' });
  });
  it('the forum session goes to the forum host only, through redirects', async () => {
    const dir = tempDir();
    const f = path.join(dir, 'v.bin');
    fs.writeFileSync(f, Buffer.alloc(1000, 7));
    const log = [];
    const serve = fileFetch(f, { log });
    const fetch = async (url, init) => {
      if (url.startsWith('https://discuss.eroscripts.com/')) {
        log.push({ url, headers: { ...init.headers } });
        return new Response(null, { status: 302, headers: { location: 'https://cdn.example/v.mp4' } });
      }
      return serve(url, init);
    };
    const src = await httpSource({ url: 'https://discuss.eroscripts.com/uploads/short-url/x.mp4', host: 'forum', headers: { 'User-Agent': 'UA' }, sensitive: { Cookie: '_t=secret' }, fetch });
    expect(src.size).toBe(1000);
    const chunks = [];
    for await (const c of await src.open(10, 19)) chunks.push(c);
    expect(Buffer.concat(chunks)).toEqual(Buffer.alloc(10, 7));
    expect(log[0].headers.Cookie).toBe('_t=secret');
    expect(log.slice(1).every((l) => l.url === 'https://cdn.example/v.mp4' && !l.headers.Cookie && l.headers['User-Agent'] === 'UA')).toBe(true);
  });
  it('the range proxy serves ranges, counts bytes and stops at its cap', async () => {
    const data = Buffer.from(Array.from({ length: 5000 }, (_, i) => i % 251));
    const open = (s, e) => Readable.from([data.subarray(s, e + 1)]);
    const proxy = await startRangeProxy({ size: data.length, open, maxBytes: 3000 });
    try {
      const r = await fetch(proxy.url, { headers: { Range: 'bytes=100-199' } });
      expect(r.status).toBe(206);
      expect(r.headers.get('content-range')).toBe('bytes 100-199/5000');
      expect(Buffer.from(await r.arrayBuffer())).toEqual(data.subarray(100, 200));
      const head = await fetch(proxy.url, { method: 'HEAD' });
      expect(head.headers.get('content-length')).toBe('5000');
      expect((await fetch(proxy.url, { headers: { Range: 'bytes=9000-' } })).status).toBe(416);
      expect(proxy.bytes()).toBe(100);
      await fetch(proxy.url, { headers: { Range: 'bytes=0-' } }).then((x) => x.arrayBuffer()).catch(() => null);
      expect(proxy.capped()).toBe(true);
    } finally {
      await proxy.close();
    }
  });
});

describe('paid and quota-counted hosts are never read', () => {
  it('SLR: only the public scene details and its trailer', async () => {
    const dir = tempDir();
    const slr = {
      scene: async (id) => (id === 52317 ? { trailerEncodings: [], videoPreview: null } : null),
      download: () => { throw new Error('quota used'); },
      plan: () => { throw new Error('quota used'); },
      ready: () => { throw new Error('quota used'); },
    };
    const c = { id: 1, vr: true, scenes: [{ video: { name: 'X_180_LR.mp4', host: 'slr', slrSceneId: 52317 } }] };
    await expect(postFrames(c, dir, { slr, fetch: () => { throw new Error('no fetch'); }, run: async () => ({ code: 1 }) }))
      .rejects.toMatchObject({ reason: 'slr: no free trailer' });
  });
  it('VRBangers: the public page and its stills, no video', async () => {
    const dir = tempDir();
    const flat = [{ data: 1 }, { scene: 2 }, { slug: 3, videoSettings: 4, galleryImages: 5 }, 'queen-of-sticks', {}, [6], { permalink: 7, previews: 8 }, '/uploads/a.jpg', []];
    const html = `<script id="__NUXT_DATA__" type="application/json">${JSON.stringify(flat)}</script>`;
    const fetched = [];
    const fetch = async (url) => { fetched.push(url); return new Response(html, { status: 200 }); };
    const inputs = [];
    const run = async (cmd, args) => { inputs.push(args[args.indexOf('-i') + 1]); fs.writeFileSync(args.at(-1), 'j'); return { code: 0 }; };
    const c = { id: 1, vr: true, scenes: [{ video: { name: 'VRBANGERS_q_8K_180x180_3dh.mp4', host: 'vrbangers', url: 'https://vrbangers.com/video/queen-of-sticks/', slug: 'queen-of-sticks' } }] };
    const r = await postFrames(c, dir, { fetch, run });
    expect(fetched).toEqual(['https://vrbangers.com/video/queen-of-sticks/']);
    expect(inputs).toEqual(['https://content.vrbangers.com/uploads/a.jpg']);
    expect(r).toMatchObject({ files: ['25.jpg'], source: 'vrbangers: VRBANGERS_q_8K_180x180_3dh.mp4 (scene page stills)', bytes: null });
  });
  it('posts on other hosts are skipped', async () => {
    expect(await postFrames({ id: 1, scenes: [{ video: { name: 'x.mp4', host: 'gofile' } }] }, tempDir(), {})).toEqual({ skip: 'gofile' });
  });
});

const hasFfmpeg = spawnSync('ffmpeg', ['-version']).status === 0 && spawnSync('ffprobe', ['-version']).status === 0;

describe.skipIf(!hasFfmpeg)('real ffmpeg on a generated video', () => {
  let video;
  let work;
  beforeAll(() => {
    work = fs.mkdtempSync(path.join(os.tmpdir(), 'scout-frames-video-'));
    video = path.join(work, 'sbs.mp4');
    // 10 s side by side: the left eye red, the right eye blue (both grainy, as real frames are), black
    // from 2 to 2.9 s (the 25 % frame, 2.5 s, falls in it), a keyframe every second.
    const r = spawnSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', 'color=c=red:s=320x240:r=10:d=10,noise=alls=25:allf=t+u', '-f', 'lavfi', '-i', 'color=c=blue:s=320x240:r=10:d=10,noise=alls=25:allf=t+u',
      '-filter_complex', "hstack=inputs=2,drawbox=c=black:t=fill:enable='between(t,2,2.9)'", '-c:v', 'libx264', '-g', '10', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', video]);
    if (r.status !== 0) throw new Error(`test video: ${r.stderr}`);
    return () => fs.rmSync(work, { recursive: true, force: true });
  });

  /** Width, height and the centre pixel's RGB of a JPEG. */
  const look = (file) => {
    const p = parseProbe(spawnSync('ffprobe', probeArgs({ input: file }), { encoding: 'utf8' }).stdout);
    const px = spawnSync('ffmpeg', ['-v', 'error', '-i', file, '-vf', `format=rgb24,crop=1:1:${Math.floor(p.width / 2)}:${Math.floor(p.height / 2)}`, '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-'], { maxBuffer: 1 << 20 }).stdout;
    return { width: p.width, height: p.height, rgb: [...px] };
  };

  it('three frames of the left eye through the range proxy (pixeldrain fake)', async () => {
    const dir = path.join(work, 'pd');
    const log = [];
    const c = { id: 7, vr: true, scenes: [{ video: { name: 'Scene_180_LR.mp4', host: 'pixeldrain', url: 'https://pixeldrain.com/api/file/abc?download' } }] };
    const r = await postFrames(c, dir, { fetch: fileFetch(video, { log }) });
    expect(r.files).toEqual(['25.jpg', '50.jpg', '75.jpg']);
    expect(r.source).toBe('pixeldrain: Scene_180_LR.mp4');
    expect(r.bytes).toBeGreaterThan(0);
    expect(r.bytes).toBeLessThan(10 * fs.statSync(video).size);
    expect(log.every((l) => l.url === 'https://pixeldrain.com/api/file/abc')).toBe(true);
    for (const f of r.files) {
      const { width, height, rgb } = look(path.join(dir, f));
      expect([width, height]).toEqual([320, 240]);
      expect(rgb[0]).toBeGreaterThan(200);
      expect(rgb[2]).toBeLessThan(60);
      expect(jpegSize(fs.readFileSync(path.join(dir, f)))).toEqual({ width: 320, height: 240 });
    }
    expect(fs.readdirSync(dir).sort()).toEqual(['25.jpg', '50.jpg', '75.jpg']);
  });

  it('MEGA ranges; the length and layout come from ffprobe when unknown', async () => {
    const dir = path.join(work, 'mega');
    const ranges = [];
    const node = { name: 'clip.mp4', size: fs.statSync(video).size, download: ({ start, end }) => { ranges.push([start, end]); return fs.createReadStream(video, { start, end }); } };
    const File = { fromURL: () => ({ directory: true, children: [node], loadAttributes: async () => {} }) };
    // VR without a layout token, 2:1 frame: side by side, left eye kept.
    const c = { id: 8, vr: true, scenes: [{ video: { name: 'clip.mp4', host: 'mega', url: 'https://mega.nz/folder/x#k', remote: 'clip.mp4' } }] };
    const r = await postFrames(c, dir, { File });
    expect(r.files).toEqual(['25.jpg', '50.jpg', '75.jpg']);
    expect(ranges.length).toBeGreaterThan(0);
    expect(look(path.join(dir, '50.jpg')).width).toBe(320);
  });

  it('MEGA out of bandwidth: a failure that stops MEGA', async () => {
    const dir = path.join(work, 'mega-bw');
    const node = { name: 'clip.mp4', size: fs.statSync(video).size, download: () => {
      const s = new Readable({ read() {} });
      setImmediate(() => s.destroy(new Error('Bandwidth limit reached: 900 seconds until it resets')));
      return s;
    } };
    const File = { fromURL: () => ({ directory: false, ...node, loadAttributes: async () => {} }) };
    const c = { id: 9, vr: false, scenes: [{ video: { name: 'clip.mp4', host: 'mega', url: 'https://mega.nz/file/x#k' }, durationMs: 10_000 }] };
    const err = await postFrames(c, dir, { File }).catch((e) => e);
    expect(err).toBeInstanceOf(FrameError);
    expect(err.stopHost).toBe(true);
    expect(err.reason).toMatch(/^mega: bandwidth limit/);
  });

  it('a flat video keeps the whole picture; megaSource finds the file in its folder', async () => {
    const dir = path.join(work, 'flat');
    const node = { name: 'flat.mp4', size: fs.statSync(video).size, download: ({ start, end }) => fs.createReadStream(video, { start, end }) };
    const src = await megaSource({ url: 'u', remote: 'flat.mp4', File: { fromURL: () => ({ directory: true, children: [{ directory: true, children: [node] }], loadAttributes: async () => {} }) } });
    const proxy = await startRangeProxy({ size: src.size, open: src.open });
    try {
      const r = await grabFrames({ input: proxy.url, http: true, lengthMs: 10_000, layout: 'flat', dir });
      expect(r.files).toEqual(['25.jpg', '50.jpg', '75.jpg']);
      expect(look(path.join(dir, '25.jpg')).width).toBe(640);
    } finally {
      await proxy.close();
    }
    await expect(megaSource({ url: 'u', remote: 'gone.mp4', File: { fromURL: () => ({ directory: true, children: [], loadAttributes: async () => {} }) } }))
      .rejects.toMatchObject({ reason: 'mega: gone.mp4 is gone' });
  });
});
