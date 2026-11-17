/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Pornhub links as video files for npm run eroscripts:scout
// (scripts/pornhub-host.mjs). yt-dlp is never run: the runner is a fake.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, it, expect } from 'vitest';
import {
  pornhubLinks, pornhubViewkey, pornhubFormat, parsePornhubInfo, pornhubFile, pornhubFits, ytDlpError, downloadArgs,
  createPornhub, YtDlpMissingError, PORNHUB_HOST,
} from '../../scripts/pornhub-host.mjs';
import { planWithStreams, sceneSidecar, STREAM_HOSTS } from '../../scripts/eroscripts-scout-lib.mjs';

const URL = 'https://www.pornhub.com/view_video.php?viewkey=6745abc123';

const INFO = {
  title: 'Ambie Bambiii - Sunday Ride 1080p 60fps',
  duration: 600,
  height: 1080,
  formats: [
    { format_id: 'a', vcodec: 'none', acodec: 'mp4a' },
    { format_id: '480p', vcodec: 'avc1', height: 480 },
    { format_id: '1080p', vcodec: 'avc1', height: 1080 },
    { format_id: '2160p', vcodec: 'avc1', height: 2160 },
  ],
};

describe('pornhubLinks', () => {
  it('takes view_video links on any subdomain, one per viewkey, canonical', () => {
    expect(pornhubLinks([
      'https://de.pornhub.com/view_video.php?viewkey=6745abc123&t=30#x',
      'https://www.pornhub.com/view_video.php?viewkey=6745abc123',
      'http://pornhub.com/view_video.php?pkey=1&viewkey=ph5f00_b-1',
      'https://www.pornhub.org/view_video.php?viewkey=zz9',
      'https://www.pornhub.com/model/ambie-bambiii',
      'https://www.pornhub.com/video/search?search=x',
      'https://notpornhub.com.evil/view_video.php?viewkey=1',
    ])).toEqual([
      URL,
      'https://www.pornhub.com/view_video.php?viewkey=ph5f00_b-1',
      'https://www.pornhub.com/view_video.php?viewkey=zz9',
    ]);
    expect(pornhubViewkey('https://rt.pornhub.com/view_video.php?viewkey=abc')).toBe('abc');
    expect(pornhubViewkey('https://example.com/view_video.php?viewkey=abc')).toBeNull();
  });
});

describe('pornhubFormat', () => {
  it('caps video and single-file fallback at the height', () => {
    expect(pornhubFormat(2160)).toBe('bv*[height<=2160]+ba/b[height<=2160]');
    expect(() => pornhubFormat(0)).toThrow();
  });
  it('download args merge into an mp4 in the work folder', () => {
    const args = downloadArgs(URL, 1080, '/s/x.part.d');
    expect(args).toEqual(expect.arrayContaining(['--no-playlist', '--merge-output-format', 'mp4', '-f', 'bv*[height<=1080]+ba/b[height<=1080]', '/s/x.part.d/video.%(ext)s', URL]));
  });
});

describe('parsePornhubInfo', () => {
  it('reads title, length and the highest height within the cap', () => {
    expect(parsePornhubInfo(JSON.stringify(INFO), 2160)).toEqual({ title: INFO.title, durationSec: 600, height: 2160 });
    expect(parsePornhubInfo(INFO, 1440).height).toBe(1080);
  });
  it('falls back to the top-level height without formats', () => {
    expect(parsePornhubInfo({ title: 't', duration: 0, height: 720 }, 2160)).toEqual({ title: 't', durationSec: null, height: 720 });
  });
  it('a playlist, live stream or nothing within the cap is gone', () => {
    expect(() => parsePornhubInfo({ _type: 'playlist' }, 2160)).toThrow(expect.objectContaining({ status: 404 }));
    expect(() => parsePornhubInfo({ ...INFO, is_live: true }, 2160)).toThrow(expect.objectContaining({ status: 404 }));
    expect(() => parsePornhubInfo({ formats: [{ vcodec: 'avc1', height: 2160 }] }, 1080)).toThrow(/no video format up to 1080p/);
  });
  it('a disabled or removed video fails like a dead link; others may be retried', () => {
    expect(ytDlpError('WARNING: x\nERROR: [PornHub] 6745abc123: This video has been disabled', URL)).toMatchObject({ status: 404, message: expect.stringMatching(/disabled/) });
    expect(ytDlpError('ERROR: [PornHub] 6745abc123: Video 6745abc123 has been removed', URL).status).toBe(404);
    expect(ytDlpError('ERROR: Unable to download webpage: HTTP Error 503', URL).status).toBeUndefined();
  });
});

describe('pornhubFile and the length check', () => {
  it('is one pseudo mp4 named by title and height', () => {
    const f = pornhubFile(URL, parsePornhubInfo(INFO, 1080));
    expect(f).toEqual({
      name: 'Ambie Bambiii - Sunday Ride 1080p.mp4', size: 0, host: PORNHUB_HOST, url: URL, viewkey: '6745abc123',
      title: INFO.title, height: 1080, duration: 600,
    });
    expect(pornhubFile(URL, { title: '', durationSec: null, height: 720 }).name).toBe('pornhub 6745abc123 720p.mp4');
  });
  it('fits like the other stream hosts; unknown lengths pass', () => {
    expect(pornhubFits(600, 590_000)).toBe(true);
    expect(pornhubFits(600, 30_000)).toBe(false);
    expect(pornhubFits(600, 700_000)).toBe(false);
    expect(pornhubFits(null, 30_000)).toBe(true);
    expect(pornhubFits(600, null)).toBe(true);
  });
  it('is a stream host: a script that does not fit drops it', async () => {
    expect(STREAM_HOSTS.has(PORNHUB_HOST)).toBe(true);
    const video = pornhubFile(URL, parsePornhubInfo(INFO, 1080));
    const scripts = [{ name: 'Ambie Bambiii - Sunday Ride.funscript', host: 'forum', url: 'u' }];
    const fit = await planWithStreams({ scripts, videos: [video], scriptMs: async () => 595_000 });
    expect(fit.scenes.map((s) => s.video.host)).toEqual([PORNHUB_HOST]);
    const teaser = await planWithStreams({ scripts, videos: [video], scriptMs: async () => 20_000 });
    expect(teaser.scenes).toEqual([]);
    expect(teaser.dropped).toEqual([{ url: URL, durationSec: 600, scriptMs: 20_000 }]);
  });
  it('the sidecar names the source', () => {
    const video = pornhubFile(URL, parsePornhubInfo(INFO, 1080));
    const c = { id: 348508, url: 'https://discuss.eroscripts.com/t/348508', title: 'T', author: 'a', tags: [], scenes: [{}] };
    expect(sceneSidecar(c, video, 'x.mp4', 'now')).toMatchObject({
      video: video.name, pornhub: { viewkey: '6745abc123', url: URL, title: INFO.title, height: 1080 },
    });
  });
});

describe('createPornhub (fake yt-dlp)', () => {
  const runner = (calls, { info = INFO, infoCode = 0, dlCode = 0, writes = true } = {}) => async (args) => {
    calls.push(args);
    if (args[0] === '-J') return infoCode ? { code: infoCode, stdout: '', stderr: 'ERROR: [PornHub] x: This video has been disabled' } : { code: 0, stdout: JSON.stringify(info), stderr: '' };
    if (dlCode) return { code: dlCode, stdout: '', stderr: 'ERROR: boom' };
    const out = args[args.indexOf('-o') + 1].replace('%(ext)s', 'mp4');
    if (writes) fs.writeFileSync(out, 'video');
    return { code: 0, stdout: '', stderr: '' };
  };
  const tmpPart = () => path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ph-')), 'scene.mp4.part');

  it('files reads the metadata within the cap', async () => {
    const calls = [];
    const ph = createPornhub({ run: runner(calls) });
    expect(await ph.files(URL, { cap: 1080 })).toEqual([expect.objectContaining({ height: 1080, duration: 600, host: PORNHUB_HOST })]);
    expect(calls).toEqual([['-J', '--no-playlist', '--no-warnings', URL]]);
  });
  it('a disabled video throws as gone', async () => {
    const ph = createPornhub({ run: runner([], { infoCode: 1 }) });
    await expect(ph.files(URL, { cap: 1080 })).rejects.toMatchObject({ status: 404 });
  });
  it('yt-dlp missing surfaces as YtDlpMissingError', async () => {
    const ph = createPornhub({ run: async () => { throw new YtDlpMissingError(); } });
    await expect(ph.files(URL, { cap: 1080 })).rejects.toBeInstanceOf(YtDlpMissingError);
  });
  it('downloads the capped format into the .part and checks the length', async () => {
    const calls = [];
    const tmp = tmpPart();
    const ph = createPornhub({ run: runner(calls) });
    await ph.download({ url: URL, duration: 600 }, tmp, { cap: 1080, scriptMs: 595_000, videoSeconds: () => 600 });
    expect(fs.readFileSync(tmp, 'utf8')).toBe('video');
    expect(fs.existsSync(`${tmp}.d`)).toBe(false);
    expect(calls[1]).toEqual(expect.arrayContaining(['-f', 'bv*[height<=1080]+ba/b[height<=1080]', '--merge-output-format', 'mp4']));
  });
  it('refuses a video the script does not fit, before and after the download', async () => {
    const calls = [];
    const ph = createPornhub({ run: runner(calls) });
    await expect(ph.download({ url: URL }, tmpPart(), { cap: 1080, scriptMs: 20_000 })).rejects.toThrow(/not this scene's video/);
    expect(calls).toHaveLength(1);
    await expect(ph.download({ url: URL }, tmpPart(), { cap: 1080, scriptMs: 595_000, videoSeconds: () => 30 })).rejects.toThrow(/downloaded video/);
  });
  it('a failed or empty download leaves nothing behind', async () => {
    const tmp = tmpPart();
    await expect(createPornhub({ run: runner([], { dlCode: 1 }) }).download({ url: URL }, tmp, { cap: 1080 })).rejects.toThrow(/boom/);
    await expect(createPornhub({ run: runner([], { writes: false }) }).download({ url: URL }, tmp, { cap: 1080 })).rejects.toThrow(/no mp4/);
    expect(fs.existsSync(tmp) || fs.existsSync(`${tmp}.d`)).toBe(false);
  });
});
