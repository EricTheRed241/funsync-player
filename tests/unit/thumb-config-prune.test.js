/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Thumbnail config of videos that are gone is dropped after a complete scan
// (posters, FapTap attempts, cached durations, and their custom-thumbs
// files) — with the association rule: an unplugged drive or an unreachable
// share never loses anything.
import { describe, it, expect, vi, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { pruneThumbConfig } from '../../renderer/js/thumb-config-prune.js';

const { deleteVideoThumbs, thumbStem } = await import('../../electron/custom-thumbs.js');

const LIVE = '/Volumes/2TB/GVP/EroScripts/A/Scene/Scene.mp4';
const MOVED = '/Volumes/2TB/GVP/_incoming/A/Old Scene/Old Scene.mp4';
const UNPLUGGED = '/Volumes/Other/VR/B/Scene/Scene.mp4';
const SHARE = '\\\\nas\\media\\lib\\C\\Scene.mp4';

function makeSettings(init) {
  const store = structuredClone(init);
  return { store, get: (k) => store[k], set: vi.fn((k, v) => { store[k] = v; }) };
}

/** Only `/Volumes/2TB` and its GVP folder answer; the rest is absent. */
const pathState = async (p) => (
  ['/Volumes/2TB', '/Volumes/2TB/GVP', '/Volumes/2TB/GVP/EroScripts'].includes(p) ? 'present' : 'absent'
);

function library() {
  return makeSettings({
    'library.customThumbnails': {
      [LIVE]: { type: 'image', imagePath: '/ud/custom-thumbs/live.jpg' },
      [MOVED]: { type: 'image', imagePath: '/ud/custom-thumbs/moved-eroscripts.jpg', source: 'eroscripts' },
      [UNPLUGGED]: { seekPct: 0.3 },
      [SHARE]: { type: 'image', imagePath: '/ud/custom-thumbs/share.jpg' },
    },
    'library.faptapThumbnails': {
      [MOVED]: { url: 'https://faptap.net/x.webp', status: 'no-match' },
      [UNPLUGGED]: { url: 'https://faptap.net/y.webp', status: 'imported' },
    },
    'library.rule34videoThumbnails': {
      [MOVED]: { url: 'https://rule34video.com/contents/videos_screenshots/3065000/3065491/preview.jpg', status: 'imported' },
    },
    'library.durationCache': { [LIVE]: 60, [MOVED]: 90, [UNPLUGGED]: 120, [SHARE]: 30 },
  });
}

describe('pruneThumbConfig', () => {
  it('drops every key of a video gone from a mounted drive, keeps unplugged and unreachable ones', async () => {
    const settings = library();
    const deleteFiles = vi.fn(async () => 2);

    const r = await pruneThumbConfig(settings, { scannedPaths: [LIVE], pathState, deleteFiles });

    expect(r).toEqual({ gone: 1, customThumbnails: 1, faptapThumbnails: 1, rule34videoThumbnails: 1, durationCache: 1, files: 2 });
    for (const key of ['library.customThumbnails', 'library.faptapThumbnails', 'library.rule34videoThumbnails', 'library.durationCache']) {
      expect(settings.store[key]).not.toHaveProperty([MOVED]);
    }
    expect(Object.keys(settings.store['library.customThumbnails'])).toEqual([LIVE, UNPLUGGED, SHARE]);
    expect(settings.store['library.durationCache']).toEqual({ [LIVE]: 60, [UNPLUGGED]: 120, [SHARE]: 30 });
    expect(settings.store['library.faptapThumbnails']).toHaveProperty([UNPLUGGED]);
    expect(deleteFiles).toHaveBeenCalledWith(
      [MOVED],
      ['/ud/custom-thumbs/moved-eroscripts.jpg'],
      ['/ud/custom-thumbs/live.jpg', '/ud/custom-thumbs/share.jpg'],
    );
  });

  it('never asks about videos the scan found, nor paths it is told to skip', async () => {
    const settings = library();
    const state = vi.fn(pathState);
    await pruneThumbConfig(settings, { scannedPaths: [LIVE], pathState: state, skip: (p) => p === MOVED });
    const asked = state.mock.calls.map(([p]) => p);
    expect(asked).not.toContain(LIVE);
    expect(asked).not.toContain(MOVED);
    expect(settings.set).not.toHaveBeenCalled();
  });

  it('a check that fails or times out keeps the entry', async () => {
    const settings = library();
    // The library folder timed out: nothing proves the drive answers.
    const flaky = async (p) => (p === '/Volumes/2TB/GVP' ? 'unknown' : 'absent');
    const r = await pruneThumbConfig(settings, { scannedPaths: [LIVE], pathState: flaky });
    expect(r.gone).toBe(0);
    expect(settings.set).not.toHaveBeenCalled();
  });

  it('keeps an entry written while the checks ran', async () => {
    const settings = library();
    const slow = vi.fn(async (p) => {
      if (p === MOVED) {
        settings.store['library.customThumbnails'] = {
          ...settings.store['library.customThumbnails'],
          '/Volumes/2TB/GVP/EroScripts/A/New/New.mp4': { seekPct: 0.5 },
        };
      }
      return pathState(p);
    });
    await pruneThumbConfig(settings, { scannedPaths: [LIVE], pathState: slow });
    expect(settings.store['library.customThumbnails']).toHaveProperty(['/Volumes/2TB/GVP/EroScripts/A/New/New.mp4']);
    expect(settings.store['library.customThumbnails']).not.toHaveProperty([MOVED]);
  });
});

describe('deleteVideoThumbs', () => {
  let dir;
  afterEach(() => { if (dir) fs.rmSync(dir, { recursive: true, force: true }); });

  it('deletes the gone videos\' files of every source and pruned images, never kept ones', async () => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), 'custom-thumbs-'));
    const touch = (name) => { fs.writeFileSync(path.join(dir, name), 'x'); return name; };
    const gone = touch(`${thumbStem(MOVED)}.jpg`);
    const goneOld = touch(`${thumbStem(MOVED)}.webp`);
    const goneEs = touch(`${thumbStem(MOVED, 'eroscripts')}.jpg`);
    const goneFt = touch(`${thumbStem(MOVED, 'faptap')}.jpg`);
    const goneR34 = touch(`${thumbStem(MOVED, 'rule34video')}.jpg`);
    const remapped = touch('old-hash.jpg');        // a reorganised entry's image
    const shared = touch('shared.jpg');            // also used by a live entry
    const live = touch(`${thumbStem(LIVE)}.jpg`);
    const outside = path.join(os.tmpdir(), `outside-${process.pid}.jpg`);
    fs.writeFileSync(outside, 'x');

    const n = await deleteVideoThumbs(dir, [MOVED], {
      imagePaths: [path.join(dir, remapped), path.join(dir, shared), outside],
      keep: [path.join(dir, shared)],
    });

    expect(n).toBe(6);
    expect(fs.readdirSync(dir).sort()).toEqual([live, shared].sort());
    expect(fs.existsSync(outside)).toBe(true);
    fs.rmSync(outside);
    for (const f of [gone, goneOld, goneEs, goneFt, goneR34, remapped]) expect(fs.existsSync(path.join(dir, f))).toBe(false);
  });

  it('a missing directory deletes nothing', async () => {
    await expect(deleteVideoThumbs('/nonexistent/custom-thumbs', [MOVED])).resolves.toBe(0);
  });
});
