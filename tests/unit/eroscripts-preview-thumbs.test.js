// EroScripts post previews as library thumbnails (renderer side).
//
// The stored entry is a user-upload-shaped poster plus `source` and
// `topicId`. Automatic imports (script download, batch) only fill an empty
// slot: a poster the user uploaded or a frame they pinned is never
// replaced. The kebab action replaces on purpose (`force`).
import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
  importPostPreview, fetchPreviewsBatch, planPreviewBatch, resolvePreviewUrl,
  makePreviewEntry, isPreviewEntry, CUSTOM_THUMBS_KEY, PREVIEW_EVENT,
} from '../../renderer/js/eroscripts-preview.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';
import { customThumbImagePath } from '../../renderer/components/library.js';
import { EroScriptsPanel } from '../../renderer/components/eroscripts-panel.js';
import { eventBus } from '../../renderer/js/event-bus.js';
import * as thumbCache from '../../renderer/js/thumbnail-cache.js';

const VIDEO = '/lib/Scene/My Video.mp4';
const IMG = 'https://eroscripts-discourse.eroscripts.com/original/4X/a/b/c/abc.webp';

function makeSettings(init = {}) {
  const store = { ...init };
  return { store, get: (k) => store[k], set: (k, v) => { store[k] = v; } };
}

function makeApi({ imported = { path: '/ud/custom-thumbs/x-eroscripts.jpg', dataUrl: 'data:image/jpeg;base64,AA' }, topicImage = IMG } = {}) {
  return {
    eroscriptsImportPreview: vi.fn().mockResolvedValue(imported),
    eroscriptsTopicImage: vi.fn().mockResolvedValue(topicImage),
  };
}

beforeEach(() => thumbCache.clear());

describe('preview entries', () => {
  it('are posters every thumbnail surface already resolves, tagged with their post', () => {
    const entry = makePreviewEntry('/ud/custom-thumbs/x-eroscripts.jpg', 311478);
    expect(entry).toEqual({ type: 'image', imagePath: '/ud/custom-thumbs/x-eroscripts.jpg', source: 'eroscripts', topicId: 311478 });
    expect(customThumbImagePath(entry)).toBe('/ud/custom-thumbs/x-eroscripts.jpg');
    expect(isPreviewEntry(entry)).toBe(true);
    expect(isPreviewEntry({ type: 'image', imagePath: '/u.jpg' })).toBe(false);
    expect(isPreviewEntry({ seekPct: 0.3 })).toBe(false);
  });
});

describe('importPostPreview precedence', () => {
  it('fills an empty slot and seeds the tile cache', async () => {
    const settings = makeSettings();
    const api = makeApi();
    const seen = vi.fn();
    const off = eventBus.on(PREVIEW_EVENT, seen);
    const res = await importPostPreview({ settings, api, videoPath: VIDEO, topic: { id: 7, preview: IMG } });
    off?.();
    expect(res.status).toBe('imported');
    expect(api.eroscriptsImportPreview).toHaveBeenCalledWith(VIDEO, IMG);
    expect(api.eroscriptsTopicImage).not.toHaveBeenCalled();
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toEqual(makePreviewEntry('/ud/custom-thumbs/x-eroscripts.jpg', 7));
    expect(thumbCache.get(VIDEO, 0)).toBe('data:image/jpeg;base64,AA');
    expect(seen).toHaveBeenCalledWith({ videoPath: VIDEO, dataUrl: 'data:image/jpeg;base64,AA' });
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
    expect(api.eroscriptsImportPreview).not.toHaveBeenCalled();
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toBe(existing);
  });

  it('keeps a thumbnail the user chose while the download was running', async () => {
    const settings = makeSettings();
    const mine = { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' };
    const api = makeApi();
    api.eroscriptsImportPreview.mockImplementation(async () => {
      settings.set(CUSTOM_THUMBS_KEY, { [VIDEO]: mine });
      return { path: '/ud/custom-thumbs/x-eroscripts.jpg', dataUrl: 'data:,' };
    });
    const res = await importPostPreview({ settings, api, videoPath: VIDEO, topic: { id: 7, preview: IMG } });
    expect(res.status).toBe('skipped');
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toBe(mine);
  });

  it('replaces anything when the user asks for it (force)', async () => {
    const settings = makeSettings({ [CUSTOM_THUMBS_KEY]: { [VIDEO]: { seekPct: 0.42 } } });
    const res = await importPostPreview({ settings, api: makeApi(), videoPath: VIDEO, topic: { id: 9 }, force: true });
    expect(res.status).toBe('imported');
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO].topicId).toBe(9);
  });

  it('falls back to the topic image when the result carries none', async () => {
    const api = makeApi();
    expect(await resolvePreviewUrl(api, { id: 5 })).toBe(IMG);
    expect(api.eroscriptsTopicImage).toHaveBeenCalledWith(5);
  });

  it('reports a post without an image, and a failed download, without writing', async () => {
    const settings = makeSettings();
    expect((await importPostPreview({ settings, api: makeApi({ topicImage: null }), videoPath: VIDEO, topic: { id: 5 } })).status).toBe('no-image');
    expect((await importPostPreview({ settings, api: makeApi({ imported: null }), videoPath: VIDEO, topic: { id: 5 } })).status).toBe('failed');
    expect(settings.store[CUSTOM_THUMBS_KEY]).toBeUndefined();
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
    window.funsync.eroscriptsImportPreview.mockReset()
      .mockResolvedValue({ path: '/ud/custom-thumbs/x-eroscripts.jpg', dataUrl: 'data:,' });
  });

  it('imports the post preview when the video has no custom thumbnail', async () => {
    const { panel, settings } = makePanel();
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null, topic);
    await vi.waitFor(() => expect(settings.store[CUSTOM_THUMBS_KEY]?.[VIDEO]).toBeTruthy());
    expect(window.funsync.eroscriptsImportPreview).toHaveBeenCalledWith(VIDEO, IMG);
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toEqual(makePreviewEntry('/ud/custom-thumbs/x-eroscripts.jpg', 311478));
    expect(settings.store[SCRIPT_TAGS_KEY][VIDEO].topicId).toBe(311478);
  });

  it('leaves an existing custom thumbnail alone', async () => {
    const mine = { type: 'image', imagePath: '/ud/custom-thumbs/mine.jpg' };
    const { panel, settings } = makePanel({ [CUSTOM_THUMBS_KEY]: { [VIDEO]: mine } });
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null, topic);
    await new Promise((r) => setTimeout(r, 0));
    expect(window.funsync.eroscriptsImportPreview).not.toHaveBeenCalled();
    expect(settings.store[CUSTOM_THUMBS_KEY][VIDEO]).toBe(mine);
  });

  it('does nothing when the download failed', async () => {
    window.funsync.eroscriptsDownload.mockResolvedValue({ success: false, error: 'x' });
    const { panel } = makePanel();
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null, topic);
    await new Promise((r) => setTimeout(r, 0));
    expect(window.funsync.eroscriptsImportPreview).not.toHaveBeenCalled();
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

  it('uses a recorded previewUrl instead of fetching the topic', () => {
    const settings = makeSettings({ [SCRIPT_TAGS_KEY]: { '/v/a.mp4': { topicId: 1, previewUrl: IMG } } });
    expect(planPreviewBatch(settings, videos).pending[0].topic).toEqual({ id: 1, previewUrl: IMG });
  });

  it('skips videos without a topic, resolves each post once and spaces requests', async () => {
    const settings = makeSettings({ [SCRIPT_TAGS_KEY]: tags });
    const api = makeApi();
    api.eroscriptsTopicImage.mockImplementation(async (id) => (id === 2 ? null : IMG));
    const sleep = vi.fn().mockResolvedValue();
    const progress = vi.fn();
    const summary = await fetchPreviewsBatch({ settings, api, videos, sleep, onProgress: progress });

    expect(summary).toEqual({ known: 3, total: 3, imported: 2, skipped: 0, noImage: 1, failed: 0 });
    expect(api.eroscriptsTopicImage.mock.calls).toEqual([[1], [2]]);
    expect(api.eroscriptsImportPreview.mock.calls.map((c) => c[0])).toEqual(['/v/a.mp4', '/v/b.mp4']);
    expect(Object.keys(settings.store[CUSTOM_THUMBS_KEY]).sort()).toEqual(['/v/a.mp4', '/v/b.mp4']);
    expect(settings.store[CUSTOM_THUMBS_KEY]['/v/d.mp4']).toBeUndefined();
    expect(sleep).toHaveBeenCalledTimes(2);
    expect(sleep.mock.calls[0][0]).toBeGreaterThanOrEqual(500);
    expect(progress).toHaveBeenLastCalledWith(3, 3);
  });
});
