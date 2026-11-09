// Desktop thumbnail requests (thumb-request.js): a backend 404 is remembered
// until the next scan and skips the <video> fallback; the fallback seeks
// where the backend does, past the studio ident.
import { describe, it, expect, vi, beforeEach } from 'vitest';

import {
  requestBackendThumb, forgetMissingThumbs, isThumbMissing, fallbackSeekSeconds, captureVideoThumb,
} from '../../renderer/js/thumb-request.js';

const V = '/Volumes/2TB/GVP/_incoming/A/Scene.mp4';
const OPTS = { seekPct: 0.1, exact: false };

beforeEach(() => forgetMissingThumbs());

describe('backend 404 memory', () => {
  it('asks once, then answers from memory until the next scan', async () => {
    const api = { generateSingleThumbnail: vi.fn(async () => ({ missing: true })) };

    expect(await requestBackendThumb(api, V, OPTS)).toEqual({ missing: true });
    expect(await requestBackendThumb(api, V, OPTS)).toEqual({ missing: true });
    expect(api.generateSingleThumbnail).toHaveBeenCalledTimes(1);
    expect(isThumbMissing(V)).toBe(true);

    forgetMissingThumbs();   // a rescan
    api.generateSingleThumbnail.mockResolvedValueOnce({ dataUrl: 'data:image/jpeg;base64,AA', duration: 5 });
    expect(await requestBackendThumb(api, V, OPTS)).toEqual({ dataUrl: 'data:image/jpeg;base64,AA', duration: 5 });
    expect(isThumbMissing(V)).toBe(false);
  });

  it('other failures are not remembered', async () => {
    const api = { generateSingleThumbnail: vi.fn(async () => null) };
    expect(await requestBackendThumb(api, V, OPTS)).toBeNull();
    api.generateSingleThumbnail.mockRejectedValueOnce(new Error('ipc'));
    expect(await requestBackendThumb(api, V, OPTS)).toBeNull();
    expect(api.generateSingleThumbnail).toHaveBeenCalledTimes(2);
    expect(isThumbMissing(V)).toBe(false);
  });

  it('skips the <video> fallback after a 404', async () => {
    const api = { generateSingleThumbnail: vi.fn(async () => ({ missing: true })) };
    const create = vi.spyOn(document, 'createElement');
    expect(await captureVideoThumb(api, V, OPTS)).toBeNull();
    expect(create).not.toHaveBeenCalledWith('video');
    create.mockRestore();
  });

  it('returns the backend frame without touching the fallback', async () => {
    const api = { generateSingleThumbnail: vi.fn(async () => ({ dataUrl: 'data:x', duration: 3 })) };
    const create = vi.spyOn(document, 'createElement');
    expect(await captureVideoThumb(api, V, OPTS)).toEqual({ dataUrl: 'data:x', duration: 3 });
    expect(create).not.toHaveBeenCalledWith('video');
    create.mockRestore();
  });
});

describe('fallbackSeekSeconds', () => {
  it('uses the backend rule: max(10 s, 10%) past a minute, 10% below', () => {
    expect(fallbackSeekSeconds(1800)).toBe(180);
    expect(fallbackSeekSeconds(90)).toBe(10);
    expect(fallbackSeekSeconds(30)).toBe(3);
  });

  it('honours a pinned frame exactly', () => {
    expect(fallbackSeekSeconds(1800, 0.002)).toBeCloseTo(3.6);
  });

  it('no duration → the start', () => {
    expect(fallbackSeekSeconds(NaN)).toBe(0);
    expect(fallbackSeekSeconds(0)).toBe(0);
  });
});
