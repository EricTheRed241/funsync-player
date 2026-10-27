// FapTap thumbnails (renderer side, renderer/js/eroscripts-preview.js):
// which FapTap Discovery scenes get FapTap's thumbnail after a scan, the
// entry it leaves, and that a no-match (or a thumbnail the user reset) is
// not fetched again. Uploads, pinned frames and hand-picked previews are
// never replaced.
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  planFaptapThumbnails, importFaptapThumbnail, fetchFaptapThumbnails, makeFaptapEntry, makePreviewEntry,
  isRevalidatable, CUSTOM_THUMBS_KEY, FAPTAP_ATTEMPTS_KEY, PREVIEW_EVENT,
} from '../../renderer/js/eroscripts-preview.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';
import { FAPTAP_SOURCE, SCOUT_SOURCE } from '../../renderer/js/eroscripts-links.js';
import { customThumbImagePath } from '../../renderer/components/library.js';
import { eventBus } from '../../renderer/js/event-bus.js';
import * as thumbCache from '../../renderer/js/thumbnail-cache.js';

const URL1 = 'https://faptap.net/api/assets/thumbnails/11.webp';
const URL2 = 'https://vz-abc.b-cdn.net/0f1e/thumbnail.jpg';
const JPG = '/ud/custom-thumbs/x-faptap.jpg';

const faptapRecord = (id, previewUrl) => ({
  tags: [], topicId: null, topicUrl: null, faptapId: id, faptapUrl: `https://faptap.net/v/${id}`, source: FAPTAP_SOURCE, savedAt: 1,
  ...(previewUrl ? { previewUrl } : {}),
});

function makeSettings(init = {}) {
  const store = { ...init };
  return { store, get: (k) => store[k], set: (k, v) => { store[k] = v; } };
}

const imported = { status: 'imported', path: JPG, dataUrl: 'data:image/jpeg;base64,AA', distance: 6 };
const makeApi = (res = imported) => ({ faptapImportThumbnail: vi.fn().mockResolvedValue(res) });
const videos = (...paths) => paths.map((p) => ({ path: p }));

beforeEach(() => thumbCache.clear());

describe('planFaptapThumbnails', () => {
  it('covers FapTap scenes with a thumbnail URL and no custom thumbnail, and nothing else', () => {
    const settings = makeSettings({
      [SCRIPT_TAGS_KEY]: {
        '/new.mp4': faptapRecord('1', URL1),
        '/nourl.mp4': faptapRecord('2'),
        '/post.mp4': { tags: [], topicId: 7, previewUrl: 'https://eroscripts-discourse.eroscripts.com/x.webp', source: SCOUT_SOURCE },
        '/upload.mp4': faptapRecord('3', URL1),
        '/pinned.mp4': faptapRecord('4', URL1),
        '/picked.mp4': faptapRecord('5', URL1),
        '/es.mp4': faptapRecord('6', URL1),
        '/forced.mp4': faptapRecord('7', URL1),
        '/same.mp4': faptapRecord('8', URL1),
        '/moved.mp4': faptapRecord('9', URL2),
      },
      [CUSTOM_THUMBS_KEY]: {
        '/upload.mp4': { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' },
        '/pinned.mp4': { seekPct: 0.4 },
        '/picked.mp4': makePreviewEntry('/p.jpg', 7, { forced: true }),
        '/es.mp4': makePreviewEntry('/p.jpg', 7),
        '/forced.mp4': { ...makeFaptapEntry(JPG, '7', { imageUrl: URL2 }), forced: true },
        '/same.mp4': makeFaptapEntry(JPG, '8', { imageUrl: URL1 }),
        '/moved.mp4': makeFaptapEntry(JPG, '9', { imageUrl: URL1 }),
      },
    });
    const all = videos('/new.mp4', '/nourl.mp4', '/post.mp4', '/upload.mp4', '/pinned.mp4', '/picked.mp4',
      '/es.mp4', '/forced.mp4', '/same.mp4', '/moved.mp4', '/unknown.mp4');
    expect(planFaptapThumbnails(settings, all)).toEqual([
      { videoPath: '/new.mp4', faptapId: '1', url: URL1 },
      // An earlier FapTap thumbnail gives way to a new URL.
      { videoPath: '/moved.mp4', faptapId: '9', url: URL2 },
    ]);
  });

  it('skips a URL already tried (no match, or imported then reset), but tries a new one', () => {
    const settings = makeSettings({
      [SCRIPT_TAGS_KEY]: { '/nm.mp4': faptapRecord('1', URL1), '/reset.mp4': faptapRecord('2', URL1), '/new.mp4': faptapRecord('3', URL2) },
      [FAPTAP_ATTEMPTS_KEY]: {
        '/nm.mp4': { url: URL1, status: 'no-match', distance: 20 },
        '/reset.mp4': { url: URL1, status: 'imported' },
        '/new.mp4': { url: URL1, status: 'no-match', distance: 20 },
      },
    });
    expect(planFaptapThumbnails(settings, videos('/nm.mp4', '/reset.mp4', '/new.mp4')).map((p) => p.videoPath)).toEqual(['/new.mp4']);
  });
});

describe('importFaptapThumbnail', () => {
  it('records a FapTap poster every thumbnail surface resolves, and remembers the attempt', async () => {
    const settings = makeSettings();
    const api = makeApi();
    const seen = vi.fn();
    const off = eventBus.on(PREVIEW_EVENT, seen);
    const res = await importFaptapThumbnail({ settings, api, videoPath: '/a.mp4', faptapId: '11', url: URL1 });
    off?.();
    expect(res).toMatchObject({ status: 'imported', matchScore: 6 });
    expect(api.faptapImportThumbnail).toHaveBeenCalledWith('/a.mp4', URL1);
    const entry = settings.store[CUSTOM_THUMBS_KEY]['/a.mp4'];
    expect(entry).toEqual({ type: 'image', imagePath: JPG, source: 'faptap', faptapId: '11', imageUrl: URL1, matchScore: 6 });
    expect(customThumbImagePath(entry)).toBe(JPG);
    expect(isRevalidatable(entry)).toBe(true);
    expect(settings.store[FAPTAP_ATTEMPTS_KEY]['/a.mp4']).toEqual({ url: URL1, status: 'imported' });
    expect(thumbCache.get('/a.mp4', 0)).toBe(imported.dataUrl);
    expect(seen).toHaveBeenCalledWith({ videoPath: '/a.mp4', dataUrl: imported.dataUrl });
  });

  it('remembers a no-match without writing a thumbnail; a failure is neither written nor remembered', async () => {
    const settings = makeSettings();
    const nm = await importFaptapThumbnail({ settings, api: makeApi({ status: 'no-match', distance: 21 }), videoPath: '/a.mp4', faptapId: '1', url: URL1 });
    expect(nm).toEqual({ status: 'no-match', distance: 21 });
    expect(settings.store[FAPTAP_ATTEMPTS_KEY]['/a.mp4']).toEqual({ url: URL1, status: 'no-match', distance: 21 });
    const failed = await importFaptapThumbnail({ settings, api: makeApi({ status: 'failed', error: 'x' }), videoPath: '/b.mp4', faptapId: '2', url: URL1 });
    expect(failed.status).toBe('failed');
    expect(settings.store[FAPTAP_ATTEMPTS_KEY]['/b.mp4']).toBeUndefined();
    expect(settings.store[CUSTOM_THUMBS_KEY]).toBeUndefined();
  });

  it('keeps a thumbnail the user chose while matching was running', async () => {
    const settings = makeSettings();
    const mine = { seekPct: 0.3 };
    const api = makeApi();
    api.faptapImportThumbnail.mockImplementation(async () => {
      settings.set(CUSTOM_THUMBS_KEY, { '/a.mp4': mine });
      return imported;
    });
    expect((await importFaptapThumbnail({ settings, api, videoPath: '/a.mp4', faptapId: '1', url: URL1 })).status).toBe('skipped');
    expect(settings.store[CUSTOM_THUMBS_KEY]['/a.mp4']).toBe(mine);
  });
});

describe('fetchFaptapThumbnails', () => {
  const twoScenes = () => makeSettings({
    [SCRIPT_TAGS_KEY]: { '/a.mp4': faptapRecord('1', URL1), '/b.mp4': faptapRecord('2', URL2) },
  });

  it('goes through the queue one video at a time and does not retry a no-match', async () => {
    const settings = twoScenes();
    const api = makeApi();
    api.faptapImportThumbnail
      .mockResolvedValueOnce(imported)
      .mockResolvedValueOnce({ status: 'no-match', distance: 30 });
    const sleep = vi.fn(async () => {});
    const r = await fetchFaptapThumbnails({ settings, api, videos: videos('/a.mp4', '/b.mp4'), sleep });
    expect(r).toEqual({ total: 2, imported: 1, noMatch: 1, skipped: 0, failed: 0, stopped: false });
    expect(sleep).toHaveBeenCalledTimes(2);

    const again = await fetchFaptapThumbnails({ settings, api, videos: videos('/a.mp4', '/b.mp4'), sleep });
    expect(again.total).toBe(0);
    expect(api.faptapImportThumbnail).toHaveBeenCalledTimes(2);
  });

  it('stops before the next video once aborted', async () => {
    const settings = twoScenes();
    const ctrl = new AbortController();
    const api = makeApi();
    api.faptapImportThumbnail.mockImplementation(async () => { ctrl.abort(); return imported; });
    const r = await fetchFaptapThumbnails({ settings, api, videos: videos('/a.mp4', '/b.mp4'), signal: ctrl.signal, sleep: async () => {} });
    expect(r).toMatchObject({ total: 2, imported: 1, stopped: true });
    expect(api.faptapImportThumbnail).toHaveBeenCalledTimes(1);
  });
});
