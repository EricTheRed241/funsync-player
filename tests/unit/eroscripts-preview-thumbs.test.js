// EroScripts post previews as library thumbnails (renderer side).
//
// The stored entry is a user-upload-shaped poster plus `source`, `topicId`,
// the image URL and its match distance to the video. Automatic imports
// (script download, batch) only fill an empty slot, and only with an image
// the main process matched to the video: a poster the user uploaded or a
// frame they pinned is never replaced. Revalidation removes automatic
// EroScripts posters that don't show their video, and nothing else.
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  importPostPreview, fetchPreviewsBatch, planPreviewBatch, revalidatePreviews, usePreviewCandidate,
  makePreviewEntry, isPreviewEntry, isRevalidatable, CUSTOM_THUMBS_KEY, PREVIEW_EVENT,
} from '../../renderer/js/eroscripts-preview.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';
import { customThumbImagePath } from '../../renderer/components/library.js';
import { EroScriptsPanel } from '../../renderer/components/eroscripts-panel.js';
import { eventBus } from '../../renderer/js/event-bus.js';
import * as thumbCache from '../../renderer/js/thumbnail-cache.js';

const VIDEO = '/lib/Scene/My Video.mp4';
const IMG = 'https://eroscripts-discourse.eroscripts.com/original/4X/a/b/c/abc.webp';
const JPG = '/ud/custom-thumbs/x-eroscripts.jpg';

function makeSettings(init = {}) {
  const store = { ...init };
  return { store, get: (k) => store[k], set: (k, v) => { store[k] = v; } };
}

const matched = { status: 'imported', path: JPG, dataUrl: 'data:image/jpeg;base64,AA', url: IMG, matchScore: 4 };

function makeApi({ match = matched, imported = { path: JPG, dataUrl: 'data:,' }, validate = { accepted: true, distance: 3 } } = {}) {
  return {
    eroscriptsMatchPreview: vi.fn().mockResolvedValue(match),
    eroscriptsImportPreview: vi.fn().mockResolvedValue(imported),
    eroscriptsValidatePoster: vi.fn().mockResolvedValue(validate),
    deleteEroScriptsThumbnail: vi.fn().mockResolvedValue(true),
  };
}

beforeEach(() => thumbCache.clear());

describe('preview entries', () => {
  it('are posters every thumbnail surface already resolves, tagged with their post and match', () => {
    const entry = makePreviewEntry(JPG, 311478, { imageUrl: IMG, matchScore: 6 });
    expect(entry).toEqual({ type: 'image', imagePath: JPG, source: 'eroscripts', topicId: 311478, imageUrl: IMG, matchScore: 6 });
    expect(customThumbImagePath(entry)).toBe(JPG);
    expect(isPreviewEntry(entry)).toBe(true);
    expect(isPreviewEntry({ type: 'image', imagePath: '/u.jpg' })).toBe(false);
    expect(isPreviewEntry({ seekPct: 0.3 })).toBe(false);
  });

  it('only automatic EroScripts posters are revalidated', () => {
    expect(isRevalidatable(makePreviewEntry(JPG, 1))).toBe(true);
    expect(isRevalidatable(makePreviewEntry(JPG, 1, { forced: true }))).toBe(false);
    expect(isRevalidatable({ type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' })).toBe(false);
    expect(isRevalidatable({ type: 'image', imagePath: '/x.jpg', source: 'user' })).toBe(false);
    expect(isRevalidatable({ seekPct: 0.4 })).toBe(false);
  });
});

describe('importPostPreview', () => {
  it('imports the matching image, with its score and URL, and seeds the tile cache', async () => {
    const settings = makeSettings();
    const api = makeApi();
    const seen = vi.fn();
    const off = eventBus.on(PREVIEW_EVENT, seen);
    const res = await importPostPreview({ settings, api, videoPath: VIDEO, topic: { id: 7, preview: IMG } });
    off?.();
    expect(res.status).toBe('imported');
    expect(api.eroscriptsMatchPreview).toHaveBeenCalledWith(VIDEO, 7, [IMG]);
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toEqual(makePreviewEntry(JPG, 7, { imageUrl: IMG, matchScore: 4 }));
    expect(thumbCache.get(VIDEO, 0)).toBe('data:image/jpeg;base64,AA');
    expect(seen).toHaveBeenCalledWith({ videoPath: VIDEO, dataUrl: 'data:image/jpeg;base64,AA' });
  });

  it('leaves the generated frame when no image matches the video', async () => {
    const settings = makeSettings();
    const api = makeApi({ match: { status: 'no-match', closest: 22 } });
    const res = await importPostPreview({ settings, api, videoPath: VIDEO, topic: { id: 7 } });
    expect(res).toEqual({ status: 'no-match', closest: 22 });
    expect(settings.store[CUSTOM_THUMBS_KEY]).toBeUndefined();
    expect(thumbCache.get(VIDEO, 0)).toBeNull();
  });

  it.each([
    ['an uploaded poster', { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' }],
    ['a pinned frame', { seekPct: 0.42 }],
    ['an earlier preview', makePreviewEntry('/ud/custom-thumbs/old.jpg', 3)],
  ])('never replaces %s automatically', async (_label, existing) => {
    const settings = makeSettings({ [CUSTOM_THUMBS_KEY]: { [VIDEO]: existing } });
    const api = makeApi();
    const res = await importPostPreview({ settings, api, videoPath: VIDEO, topic: { id: 7, preview: IMG } });
    expect(res.status).toBe('skipped');
    expect(api.eroscriptsMatchPreview).not.toHaveBeenCalled();
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toBe(existing);
  });

  it('keeps a thumbnail the user chose while matching was running', async () => {
    const settings = makeSettings();
    const mine = { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' };
    const api = makeApi();
    api.eroscriptsMatchPreview.mockImplementation(async () => {
      settings.set(CUSTOM_THUMBS_KEY, { [VIDEO]: mine });
      return matched;
    });
    const res = await importPostPreview({ settings, api, videoPath: VIDEO, topic: { id: 7 } });
    expect(res.status).toBe('skipped');
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toBe(mine);
  });

  it('reports a post without an image, and a failure, without writing', async () => {
    const settings = makeSettings();
    expect((await importPostPreview({ settings, api: makeApi({ match: { status: 'no-image' } }), videoPath: VIDEO, topic: { id: 5 } })).status).toBe('no-image');
    expect((await importPostPreview({ settings, api: makeApi({ match: { status: 'failed' } }), videoPath: VIDEO, topic: { id: 5 } })).status).toBe('failed');
    expect(settings.store[CUSTOM_THUMBS_KEY]).toBeUndefined();
  });
});

describe('usePreviewCandidate (manual override)', () => {
  it('sets the picked image whatever its score, replacing anything, marked as the user\'s', async () => {
    const settings = makeSettings({ [CUSTOM_THUMBS_KEY]: { [VIDEO]: { seekPct: 0.42 } } });
    const api = makeApi();
    const res = await usePreviewCandidate({ settings, api, videoPath: VIDEO, topicId: 9, candidate: { url: IMG, distance: 25 } });
    expect(res.status).toBe('imported');
    expect(api.eroscriptsImportPreview).toHaveBeenCalledWith(VIDEO, IMG);
    const entry = settings.store[CUSTOM_THUMBS_KEY][VIDEO];
    expect(entry).toEqual(makePreviewEntry(JPG, 9, { imageUrl: IMG, matchScore: 25, forced: true }));
    expect(isRevalidatable(entry)).toBe(false);
  });
});

describe('revalidation', () => {
  const paths = ['/v/bad.mp4', '/v/good.mp4', '/v/upload.mp4', '/v/pinned.mp4', '/v/forced.mp4', '/v/offline.mp4'];
  const videos = paths.map((path) => ({ path }));
  const upload = { type: 'image', imagePath: '/ud/custom-thumbs/u.jpg' };
  const pinned = { seekPct: 0.3 };
  const forced = makePreviewEntry('/ud/custom-thumbs/f-eroscripts.jpg', 4, { forced: true });

  function setup() {
    const settings = makeSettings({
      [CUSTOM_THUMBS_KEY]: {
        '/v/bad.mp4': makePreviewEntry('/ud/custom-thumbs/bad-eroscripts.jpg', 1),
        '/v/good.mp4': makePreviewEntry('/ud/custom-thumbs/good-eroscripts.jpg', 2),
        '/v/upload.mp4': upload,
        '/v/pinned.mp4': pinned,
        '/v/forced.mp4': forced,
        '/v/offline.mp4': makePreviewEntry('/ud/custom-thumbs/off-eroscripts.jpg', 3),
      },
    });
    const api = makeApi();
    api.eroscriptsValidatePoster.mockImplementation(async (videoPath) => {
      if (videoPath === '/v/bad.mp4') return { accepted: false, distance: 24, reason: 'does not match the video' };
      if (videoPath === '/v/offline.mp4') return { error: 'ENOENT' };
      return { accepted: true, distance: 5 };
    });
    return { settings, api };
  }

  it('removes failing EroScripts posters and their files, and touches nothing else', async () => {
    const { settings, api } = setup();
    const repaint = vi.fn();
    const off = eventBus.on(PREVIEW_EVENT, repaint);
    thumbCache.set('/v/bad.mp4', 0, 'data:old');
    const summary = await revalidatePreviews({ settings, api, videos });
    off?.();

    expect(summary).toEqual({ checked: 3, kept: 1, removed: 1, unchecked: 1 });
    expect(api.eroscriptsValidatePoster.mock.calls.map((c) => c[0])).toEqual(['/v/bad.mp4', '/v/good.mp4', '/v/offline.mp4']);
    const map = settings.store[CUSTOM_THUMBS_KEY];
    expect(map['/v/bad.mp4']).toBeUndefined();
    expect(api.deleteEroScriptsThumbnail).toHaveBeenCalledTimes(1);
    expect(api.deleteEroScriptsThumbnail).toHaveBeenCalledWith('/ud/custom-thumbs/bad-eroscripts.jpg');
    expect(thumbCache.get('/v/bad.mp4', 0)).toBeNull();
    expect(repaint).toHaveBeenCalledWith({ videoPath: '/v/bad.mp4', dataUrl: null });
    expect(map['/v/good.mp4'].matchScore).toBe(5);
    expect(map['/v/upload.mp4']).toBe(upload);
    expect(map['/v/pinned.mp4']).toBe(pinned);
    expect(map['/v/forced.mp4']).toBe(forced);
    expect(map['/v/offline.mp4'].imagePath).toBe('/ud/custom-thumbs/off-eroscripts.jpg');
  });

  it('keeps a poster the user replaced while it was being checked', async () => {
    const { settings, api } = setup();
    const mine = { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' };
    api.eroscriptsValidatePoster.mockImplementation(async (videoPath) => {
      if (videoPath === '/v/bad.mp4') {
        settings.set(CUSTOM_THUMBS_KEY, { ...settings.get(CUSTOM_THUMBS_KEY), '/v/bad.mp4': mine });
        return { accepted: false, distance: 30 };
      }
      return { accepted: true, distance: 2 };
    });
    await revalidatePreviews({ settings, api, videos: [{ path: '/v/bad.mp4' }] });
    expect(settings.store[CUSTOM_THUMBS_KEY]['/v/bad.mp4']).toBe(mine);
    expect(api.deleteEroScriptsThumbnail).not.toHaveBeenCalled();
  });
});

describe('auto-import on a panel script download', () => {
  const topic = { id: 311478, tags: ['multi-axis'], url: 'https://discuss.eroscripts.com/t/x/311478', preview: IMG };

  function makePanel(init) {
    const settings = makeSettings({ 'library.sources': [{ path: '/lib', enabled: true }], ...init });
    const panel = new EroScriptsPanel({ settings });
    panel.onScriptDownloaded = vi.fn();
    return { panel, settings };
  }

  beforeEach(() => {
    document.body.innerHTML = `<div id="app"><div id="player-container" data-video-path="${VIDEO}"></div></div>`;
    window.funsync.eroscriptsDownload.mockReset().mockResolvedValue({ success: true });
    window.funsync.filesExist.mockImplementation(async (list) => list.map(() => false));
    window.funsync.eroscriptsMatchPreview.mockReset().mockResolvedValue(matched);
  });

  it('imports the matching post image when the video has no custom thumbnail', async () => {
    const { panel, settings } = makePanel();
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null, topic);
    await vi.waitFor(() => expect(settings.store[CUSTOM_THUMBS_KEY]?.[VIDEO]).toBeTruthy());
    expect(window.funsync.eroscriptsMatchPreview).toHaveBeenCalledWith(VIDEO, 311478, [IMG]);
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toEqual(makePreviewEntry(JPG, 311478, { imageUrl: IMG, matchScore: 4 }));
    expect(settings.store[SCRIPT_TAGS_KEY][VIDEO].topicId).toBe(311478);
  });

  it('leaves an existing custom thumbnail alone', async () => {
    const mine = { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' };
    const { panel, settings } = makePanel({ [CUSTOM_THUMBS_KEY]: { [VIDEO]: mine } });
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null, topic);
    await new Promise((r) => setTimeout(r, 0));
    expect(window.funsync.eroscriptsMatchPreview).not.toHaveBeenCalled();
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toBe(mine);
  });

  it('does nothing when the download failed', async () => {
    window.funsync.eroscriptsDownload.mockResolvedValue({ success: false, error: 'x' });
    const { panel } = makePanel();
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null, topic);
    await new Promise((r) => setTimeout(r, 0));
    expect(window.funsync.eroscriptsMatchPreview).not.toHaveBeenCalled();
  });
});

describe('batch fetch', () => {
  const videos = ['/v/a.mp4', '/v/b.mp4', '/v/c.mp4', '/v/d.mp4', '/v/e.mp4'].map((path) => ({ path }));
  const tags = {
    '/v/a.mp4': { topicId: 1 },
    '/v/b.mp4': { topicId: 1 }, // same post as a
    '/v/c.mp4': { topicId: 2 },
    '/v/d.mp4': { tags: [] }, // no topic id
    // e: never linked
  };

  it('plans only videos with a known topic and no custom thumbnail', () => {
    const settings = makeSettings({
      [SCRIPT_TAGS_KEY]: tags,
      [CUSTOM_THUMBS_KEY]: { '/v/c.mp4': { seekPct: 0.2 } },
    });
    const { known, pending } = planPreviewBatch(settings, videos);
    expect(known).toBe(3);
    expect(pending.map((p) => p.videoPath)).toEqual(['/v/a.mp4', '/v/b.mp4']);
  });

  it('passes a recorded previewUrl along as a candidate', () => {
    const settings = makeSettings({ [SCRIPT_TAGS_KEY]: { '/v/a.mp4': { topicId: 1, previewUrl: IMG } } });
    expect(planPreviewBatch(settings, videos).pending[0].topic).toEqual({ id: 1, previewUrl: IMG });
  });

  it('revalidates first, then fills empty slots with matching images, spacing new posts', async () => {
    const settings = makeSettings({
      [SCRIPT_TAGS_KEY]: tags,
      [CUSTOM_THUMBS_KEY]: { '/v/c.mp4': makePreviewEntry('/ud/custom-thumbs/c-eroscripts.jpg', 2) },
    });
    const api = makeApi({ validate: { accepted: false, distance: 27 } });
    api.eroscriptsMatchPreview.mockImplementation(async (videoPath, topicId) => {
      if (topicId === 2) return { status: 'no-match', closest: 19 };
      return videoPath === '/v/b.mp4' ? { status: 'no-image' } : matched;
    });
    const sleep = vi.fn().mockResolvedValue();
    const progress = vi.fn();
    const summary = await fetchPreviewsBatch({ settings, api, videos, sleep, onProgress: progress });

    expect(summary).toEqual({
      known: 3, total: 3, imported: 1, skipped: 0, noMatch: 1, noImage: 1, failed: 0,
      revalidated: { checked: 1, kept: 0, removed: 1, unchecked: 0 },
    });
    expect(api.eroscriptsValidatePoster).toHaveBeenCalledWith('/v/c.mp4', '/ud/custom-thumbs/c-eroscripts.jpg');
    expect(api.eroscriptsMatchPreview.mock.calls.map((c) => c[0])).toEqual(['/v/a.mp4', '/v/b.mp4', '/v/c.mp4']);
    expect(Object.keys(settings.store[CUSTOM_THUMBS_KEY])).toEqual(['/v/a.mp4']);
    // One pause, before the second post; the second video of post 1 needs no new forum request.
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(sleep.mock.calls[0][0]).toBeGreaterThanOrEqual(500);
    expect(progress).toHaveBeenCalledWith('revalidate', 1, 1);
    expect(progress).toHaveBeenLastCalledWith('fetch', 3, 3);
  });

  it('can skip revalidation', async () => {
    const settings = makeSettings({ [CUSTOM_THUMBS_KEY]: { '/v/c.mp4': makePreviewEntry('/x-eroscripts.jpg', 2) } });
    const api = makeApi();
    const summary = await fetchPreviewsBatch({ settings, api, videos, revalidate: false, sleep: async () => {} });
    expect(summary.revalidated).toBeNull();
    expect(api.eroscriptsValidatePoster).not.toHaveBeenCalled();
  });
});
