/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// VR follows the video: the registration carries the VR Format projections
// and post tags, the library asks the backend for its VR decisions
// (`GET /api/media/vr-flags`), and those feed the VR filter and the VR
// category; still-unknown videos (`pending`) are asked for again later.
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { Library } from '../../renderer/components/library.js';
import { AUTO_CATEGORIES_KEY } from '../../renderer/js/auto-categories.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';
import { isVRVideo, isVRAutoDetected, setDetectedFlags } from '../../renderer/js/vr-detect.js';

function makeLibrary(store = {}) {
  const lib = Object.create(Library.prototype);
  lib._videos = [{ path: '/v/a.mp4', name: 'a' }, { path: '/v/b.mp4', name: 'b' }];
  const videoCategories = { '/v/flat.mp4': ['vr-id'] };
  lib._settings = {
    get: (k) => store[k],
    set: (k, v) => { store[k] = v; },
    getCategories: () => [{ id: 'vr-id', name: 'VR' }],
    assignCategories: vi.fn((pairs) => {
      for (const [p, id] of pairs) videoCategories[p] = [...(videoCategories[p] || []), id];
    }),
    unassignCategory: vi.fn((p, id) => { videoCategories[p] = (videoCategories[p] || []).filter((x) => x !== id); }),
    _cache: { videoCategories },
  };
  return lib;
}

const registerBodies = () => globalThis.fetch.mock.calls
  .filter(([url]) => url.endsWith('/api/media/register'))
  .map(([, init]) => JSON.parse(init.body));

describe('Library VR from the backend', () => {
  let flagReplies;

  beforeEach(() => {
    flagReplies = [];
    globalThis.fetch = vi.fn((url) => {
      if (url.endsWith('/api/media/vr-flags')) {
        const body = flagReplies.shift();
        return Promise.resolve({ ok: true, json: () => Promise.resolve(body) });
      }
      return Promise.resolve({ ok: true });
    });
    globalThis.window = { funsync: { getCustomThumbsDir: vi.fn(() => Promise.resolve('/dir')) } };
  });

  afterEach(() => {
    vi.useRealTimers();
    setDetectedFlags(null);
    delete globalThis.fetch;
    delete globalThis.window;
  });

  it('registers the VR Format projections and post tags of the registered videos', async () => {
    const lib = makeLibrary({
      'library.vrFormat': { '/v/a.mp4': { projection: 'flat', source: 'manual' }, '/v/gone.mp4': { projection: 'sbs-half' } },
      [SCRIPT_TAGS_KEY]: { '/v/b.mp4': { postId: 1, tags: ['creampie', 'osr2'] }, '/v/a.mp4': { postId: 2, tags: [] } },
    });
    expect(await lib._registerWithBackend()).toBe(true);
    const [body] = registerBodies();
    expect(body.vrFormat).toEqual({ '/v/a.mp4': { projection: 'flat', source: 'manual' } });
    expect(body.scriptTags).toEqual({ '/v/b.mp4': ['creampie', 'osr2'] });
  });

  it('feeds the flags to the VR filter and the VR category, and asks again while some are pending', async () => {
    vi.useFakeTimers();
    const store = { [AUTO_CATEGORIES_KEY]: { created: ['vr'], assigned: { '/v/flat.mp4': ['vr'] } } };
    const lib = makeLibrary(store);
    flagReplies.push(
      { flags: { '/v/a.mp4': true, '/v/flat.mp4': false }, pending: 1 },
      { flags: { '/v/a.mp4': true, '/v/flat.mp4': false, '/v/b.mp4': true }, pending: 0 },
    );
    lib._registerWithBackend();
    await lib._syncVRFromBackend({ fresh: true });

    // The backend's decision beats the filename heuristic; the override still beats both.
    expect(isVRAutoDetected('/v/a.mp4')).toBe(true);
    expect(isVRVideo({ path: '/v/a.mp4' })).toBe(true);
    expect(isVRVideo({ path: '/v/b.mp4' })).toBe(false);
    expect(lib._settings.assignCategories).toHaveBeenCalledWith([['/v/a.mp4', 'vr-id']]);
    expect(lib._settings.unassignCategory).toHaveBeenCalledWith('/v/flat.mp4', 'vr-id');
    expect(store[AUTO_CATEGORIES_KEY].assigned).toEqual({ '/v/a.mp4': ['vr'] });

    // One video was still pending: asked again later, and only then decided.
    await vi.advanceTimersByTimeAsync(30_000);
    expect(isVRVideo({ path: '/v/b.mp4' })).toBe(true);
    expect(lib._settings.assignCategories).toHaveBeenLastCalledWith([['/v/b.mp4', 'vr-id']]);
    const flagCalls = () => globalThis.fetch.mock.calls.filter(([url]) => url.endsWith('/vr-flags')).length;
    expect(flagCalls()).toBe(2);
    await vi.advanceTimersByTimeAsync(120_000);
    expect(flagCalls()).toBe(2);
  });

  it('asks for no flags when the registration never landed', async () => {
    globalThis.fetch = vi.fn(() => Promise.resolve({ ok: false, status: 503 }));
    const lib = makeLibrary();
    lib._registration = Promise.resolve(false);
    await lib._syncVRFromBackend({ fresh: true });
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });
});
