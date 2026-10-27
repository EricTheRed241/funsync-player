/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The custom-thumbs dir in the backend registration.
//
// Regression (2026-10-27): the first registration after a start was sent
// before the `get-custom-thumbs-dir` IPC answered, so it carried no
// `customThumbsDir`. The backend then served generated frames in place of
// every uploaded poster, EroScripts preview and FapTap thumbnail until some
// later change re-registered the library, and HereSphere keeps the first
// thumbnail it gets.
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { Library } from '../../renderer/components/library.js';

const DIR = '/u/funsync-player/custom-thumbs';

function makeLibrary() {
  const lib = Object.create(Library.prototype);
  lib._videos = [{ path: '/v/a.mp4', name: 'a' }];
  const store = { 'library.customThumbnails': { '/v/a.mp4': { type: 'image', imagePath: `${DIR}/x.jpg` } } };
  lib._settings = { get: (k) => store[k] };
  return lib;
}

const bodies = () => globalThis.fetch.mock.calls.map(([, init]) => JSON.parse(init.body));

describe('Library._registerWithBackend custom-thumbs dir', () => {
  let resolveDir;

  beforeEach(() => {
    globalThis.fetch = vi.fn(() => Promise.resolve({ ok: true }));
    globalThis.window = {
      funsync: { getCustomThumbsDir: vi.fn(() => new Promise((r) => { resolveDir = r; })) },
    };
  });

  afterEach(() => {
    delete globalThis.fetch;
    delete globalThis.window;
  });

  it('sends the first registration only once the dir is known, with the dir', async () => {
    const lib = makeLibrary();
    lib._registerWithBackend();
    await Promise.resolve();
    expect(globalThis.fetch).not.toHaveBeenCalled();

    resolveDir(DIR);
    await vi.waitFor(() => expect(globalThis.fetch).toHaveBeenCalledTimes(1));
    expect(bodies()[0].customThumbsDir).toBe(DIR);
    expect(bodies()[0].customThumbnails['/v/a.mp4'].imagePath).toBe(`${DIR}/x.jpg`);
  });

  it('asks for the dir once and sends it with every registration, in order', async () => {
    const lib = makeLibrary();
    lib._registerWithBackend();
    lib._videos = [...lib._videos, { path: '/v/b.mp4', name: 'b' }];
    lib._registerWithBackend();
    resolveDir(DIR);
    await vi.waitFor(() => expect(globalThis.fetch).toHaveBeenCalledTimes(2));
    expect(window.funsync.getCustomThumbsDir).toHaveBeenCalledTimes(1);
    expect(bodies().map((b) => [b.customThumbsDir, b.videos.length])).toEqual([[DIR, 1], [DIR, 2]]);
  });

  it('still registers the library when the dir lookup fails', async () => {
    window.funsync.getCustomThumbsDir = vi.fn(() => Promise.reject(new Error('ipc down')));
    const lib = makeLibrary();
    lib._registerWithBackend();
    await vi.waitFor(() => expect(globalThis.fetch).toHaveBeenCalledTimes(1));
    expect(bodies()[0].videos).toHaveLength(1);
    expect(bodies()[0]).not.toHaveProperty('customThumbsDir');
  });
});
