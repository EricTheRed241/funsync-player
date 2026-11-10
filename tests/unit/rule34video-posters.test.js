// rule34video covers (renderer side, renderer/js/eroscripts-preview.js):
// which scanned scenes get rule34video's cover after a scan, what it may
// replace (an automatic EroScripts preview or FapTap thumbnail, never an
// upload, a pinned frame or a hand-picked poster), and which attempts are
// remembered so a link isn't fetched on every scan.
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  planRule34videoPosters, importRule34videoPoster, fetchRule34videoPosters, canRule34videoImport,
  makeRule34videoEntry, makePreviewEntry, makeFaptapEntry, planPreviewBatch, planFaptapThumbnails, isRevalidatable,
  CUSTOM_THUMBS_KEY, RULE34VIDEO_ATTEMPTS_KEY, RULE34VIDEO_RETRY_MS, PREVIEW_EVENT,
} from '../../renderer/js/eroscripts-preview.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';
import { FAPTAP_SOURCE, SCOUT_SOURCE } from '../../renderer/js/eroscripts-links.js';
import { customThumbImagePath } from '../../renderer/components/library.js';
import { eventBus } from '../../renderer/js/event-bus.js';
import * as thumbCache from '../../renderer/js/thumbnail-cache.js';

const COVER1 = 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg';
const COVER2 = 'https://rule34video.com/contents/videos_screenshots/3069000/3069287/preview.jpg';
const JPG = '/ud/custom-thumbs/x-rule34video.jpg';
const NOW = 1_800_000_000_000;

function makeSettings(init = {}) {
  const store = { ...init };
  return { store, get: (k) => store[k], set: vi.fn((k, v) => { store[k] = v; }) };
}

const scene = (p, posterUrl, topicId = 7) => ({ path: p, eroscripts: { topicId, url: null, tags: [], ...(posterUrl ? { posterUrl } : {}) } });
const imported = { status: 'imported', path: JPG, dataUrl: 'data:image/jpeg;base64,AA' };
const makeApi = (res = imported) => ({ rule34videoImportPoster: vi.fn().mockResolvedValue(res), deleteEroScriptsThumbnail: vi.fn().mockResolvedValue(true) });

beforeEach(() => thumbCache.clear());

describe('canRule34videoImport', () => {
  it('fills an empty slot and replaces automatic posters, never the user’s', () => {
    expect(canRule34videoImport(undefined, COVER1)).toBe(true);
    expect(canRule34videoImport(makePreviewEntry('/p.jpg', 7), COVER1)).toBe(true);
    expect(canRule34videoImport(makeFaptapEntry('/f.jpg', '9', { imageUrl: 'https://faptap.net/x.webp' }), COVER1)).toBe(true);
    expect(canRule34videoImport(makeRule34videoEntry(JPG, 7, COVER2), COVER1)).toBe(true);

    expect(canRule34videoImport({ type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' }, COVER1)).toBe(false);
    expect(canRule34videoImport({ seekPct: 0.4 }, COVER1)).toBe(false);
    expect(canRule34videoImport(makePreviewEntry('/p.jpg', 7, { forced: true }), COVER1)).toBe(false);
    expect(canRule34videoImport({ ...makeFaptapEntry('/f.jpg', '9'), forced: true }, COVER1)).toBe(false);
    expect(canRule34videoImport(makeRule34videoEntry(JPG, 7, COVER1), COVER1)).toBe(false);
  });

  it('once set, the EroScripts and FapTap imports leave it alone and revalidation skips it', () => {
    const entry = makeRule34videoEntry(JPG, 7, COVER1);
    expect(isRevalidatable(entry)).toBe(false);
    const settings = makeSettings({
      [SCRIPT_TAGS_KEY]: {
        '/a.mp4': { tags: [], topicId: 7, source: SCOUT_SOURCE },
        '/b.mp4': { tags: [], topicId: null, faptapId: '1', source: FAPTAP_SOURCE, previewUrl: 'https://faptap.net/x.webp' },
      },
      [CUSTOM_THUMBS_KEY]: { '/a.mp4': entry, '/b.mp4': entry },
    });
    expect(planPreviewBatch(settings, [{ path: '/a.mp4' }]).pending).toEqual([]);
    expect(planFaptapThumbnails(settings, [{ path: '/b.mp4' }])).toEqual([]);
  });
});

describe('planRule34videoPosters', () => {
  it('covers scanned scenes with a cover whose slot the import may fill', () => {
    const settings = makeSettings({
      [CUSTOM_THUMBS_KEY]: {
        '/upload.mp4': { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' },
        '/pinned.mp4': { seekPct: 0.4 },
        '/picked.mp4': makePreviewEntry('/p.jpg', 7, { forced: true }),
        '/es.mp4': makePreviewEntry('/p.jpg', 7),
        '/ft.mp4': makeFaptapEntry('/f.jpg', '9'),
        '/same.mp4': makeRule34videoEntry(JPG, 7, COVER1),
      },
    });
    const videos = [
      scene('/new.mp4', COVER1), { path: '/plain.mp4' }, scene('/nocover.mp4'), scene('/upload.mp4', COVER1),
      scene('/pinned.mp4', COVER1), scene('/picked.mp4', COVER1), scene('/es.mp4', COVER1), scene('/ft.mp4', COVER1),
      scene('/same.mp4', COVER1),
    ];
    expect(planRule34videoPosters(settings, videos, NOW)).toEqual([
      { videoPath: '/new.mp4', topicId: 7, url: COVER1 },
      { videoPath: '/es.mp4', topicId: 7, url: COVER1 },
      { videoPath: '/ft.mp4', topicId: 7, url: COVER1 },
    ]);
  });

  it('skips a link already tried (imported then reset, unusable), retries a failure after a day, tries a new link', () => {
    const settings = makeSettings({
      [RULE34VIDEO_ATTEMPTS_KEY]: {
        '/reset.mp4': { url: COVER1, status: 'imported' },
        '/gone.mp4': { url: COVER1, status: 'unusable' },
        '/recent.mp4': { url: COVER1, status: 'failed', at: NOW - 60_000 },
        '/old.mp4': { url: COVER1, status: 'failed', at: NOW - RULE34VIDEO_RETRY_MS },
        '/new.mp4': { url: COVER2, status: 'unusable' },
      },
    });
    const videos = ['/reset.mp4', '/gone.mp4', '/recent.mp4', '/old.mp4', '/new.mp4'].map((p) => scene(p, COVER1));
    expect(planRule34videoPosters(settings, videos, NOW).map((p) => p.videoPath)).toEqual(['/old.mp4', '/new.mp4']);
  });
});

describe('importRule34videoPoster', () => {
  it('records a cover every thumbnail surface resolves, and remembers the attempt', async () => {
    const settings = makeSettings();
    const api = makeApi();
    const seen = vi.fn();
    const off = eventBus.on(PREVIEW_EVENT, seen);
    const res = await importRule34videoPoster({ settings, api, videoPath: '/a.mp4', topicId: 7, url: COVER1 });
    off?.();
    expect(res).toEqual({ status: 'imported', dataUrl: imported.dataUrl });
    expect(api.rule34videoImportPoster).toHaveBeenCalledWith('/a.mp4', COVER1);
    const entry = settings.store[CUSTOM_THUMBS_KEY]['/a.mp4'];
    expect(entry).toEqual({ type: 'image', imagePath: JPG, source: 'rule34video', topicId: 7, imageUrl: COVER1 });
    expect(customThumbImagePath(entry)).toBe(JPG);
    expect(settings.store[RULE34VIDEO_ATTEMPTS_KEY]['/a.mp4']).toEqual({ url: COVER1, status: 'imported' });
    expect(thumbCache.get('/a.mp4', 0)).toBe(imported.dataUrl);
    expect(seen).toHaveBeenCalledWith({ videoPath: '/a.mp4', dataUrl: imported.dataUrl });
    expect(api.deleteEroScriptsThumbnail).not.toHaveBeenCalled();
  });

  it('replaces an automatic EroScripts preview and deletes its file', async () => {
    const settings = makeSettings({ [CUSTOM_THUMBS_KEY]: { '/a.mp4': makePreviewEntry('/ud/custom-thumbs/a-eroscripts.jpg', 7) } });
    const api = makeApi();
    expect((await importRule34videoPoster({ settings, api, videoPath: '/a.mp4', topicId: 7, url: COVER1 })).status).toBe('imported');
    expect(settings.store[CUSTOM_THUMBS_KEY]['/a.mp4'].source).toBe('rule34video');
    expect(api.deleteEroScriptsThumbnail).toHaveBeenCalledWith('/ud/custom-thumbs/a-eroscripts.jpg');
  });

  it('remembers an unusable link for good and a failure with its time', async () => {
    const settings = makeSettings();
    const unusable = await importRule34videoPoster({ settings, api: makeApi({ status: 'unusable', error: 'Image download failed (404)' }), videoPath: '/a.mp4', topicId: 7, url: COVER1 });
    expect(unusable.status).toBe('unusable');
    const failed = await importRule34videoPoster({
      settings, api: makeApi({ status: 'failed', error: 'fetch failed' }), videoPath: '/b.mp4', topicId: 7, url: COVER1, now: () => NOW,
    });
    expect(failed.status).toBe('failed');
    expect(settings.store[RULE34VIDEO_ATTEMPTS_KEY]).toEqual({
      '/a.mp4': { url: COVER1, status: 'unusable' },
      '/b.mp4': { url: COVER1, status: 'failed', at: NOW },
    });
    expect(settings.store[CUSTOM_THUMBS_KEY]).toBeUndefined();
  });

  it('leaves a poster the user picked while the download ran', async () => {
    const settings = makeSettings();
    const mine = { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' };
    const api = makeApi();
    api.rule34videoImportPoster.mockImplementation(async () => {
      settings.set(CUSTOM_THUMBS_KEY, { '/a.mp4': mine });
      return imported;
    });
    expect((await importRule34videoPoster({ settings, api, videoPath: '/a.mp4', topicId: 7, url: COVER1 })).status).toBe('skipped');
    expect(settings.store[CUSTOM_THUMBS_KEY]['/a.mp4']).toBe(mine);
  });
});

describe('fetchRule34videoPosters', () => {
  it('imports each planned cover once, spaced out, and tries only a passing failure again later', async () => {
    const settings = makeSettings();
    const api = makeApi();
    api.rule34videoImportPoster
      .mockResolvedValueOnce(imported)
      .mockResolvedValueOnce({ status: 'unusable', error: 'Not an image' })
      .mockResolvedValueOnce({ status: 'failed', error: 'Image download failed (503)' });
    const sleep = vi.fn(async () => {});
    const all = [scene('/a.mp4', COVER1), scene('/b.mp4', COVER2), scene('/c.mp4', COVER1)];
    const r = await fetchRule34videoPosters({ settings, api, videos: all, sleep, now: () => NOW });
    expect(r).toEqual({ total: 3, imported: 1, unusable: 1, skipped: 0, failed: 1, stopped: false });
    expect(sleep).toHaveBeenCalledTimes(3);

    // Next scan: nothing; a day later only the 503 is tried again.
    expect((await fetchRule34videoPosters({ settings, api, videos: all, sleep, now: () => NOW + 1000 })).total).toBe(0);
    const later = await fetchRule34videoPosters({ settings, api, videos: all, sleep, now: () => NOW + RULE34VIDEO_RETRY_MS });
    expect(later.total).toBe(1);
    expect(api.rule34videoImportPoster).toHaveBeenLastCalledWith('/c.mp4', COVER1);
  });

  it('stops before the next video once aborted', async () => {
    const settings = makeSettings();
    const ctrl = new AbortController();
    const api = makeApi();
    api.rule34videoImportPoster.mockImplementation(async () => { ctrl.abort(); return imported; });
    const r = await fetchRule34videoPosters({
      settings, api, videos: [scene('/a.mp4', COVER1), scene('/b.mp4', COVER2)], signal: ctrl.signal, sleep: async () => {},
    });
    expect(r).toMatchObject({ total: 2, imported: 1, stopped: true });
    expect(api.rule34videoImportPoster).toHaveBeenCalledTimes(1);
  });
});
