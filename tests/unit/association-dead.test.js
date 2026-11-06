/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// An association whose VIDEO is gone for good is dropped, whole — but only
// when the drive it lived on demonstrably answers. An unplugged drive must
// never look like a deleted video (the 2026-08-11 loss, see
// association-prune-safety.test.js).
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

vi.mock('../../renderer/js/toast.js', () => ({ showToast: vi.fn() }));

import { isVideoLocationGone, splitVolumePath } from '../../renderer/js/association-dead.js';
import { buildAssociationEntry } from '../../renderer/js/association-shape.js';
import { Library } from '../../renderer/components/library.js';

/**
 * A fake `pathState`: the listed paths are 'present', those in `unknown`
 * (timed out, EACCES, EIO) are 'unknown', everything else is 'absent'.
 */
const state = (present, unknown = []) => {
  const has = new Set(present);
  const unsure = new Set(unknown);
  return vi.fn(async (p) => (has.has(p) ? 'present' : unsure.has(p) ? 'unknown' : 'absent'));
};
const fs = (...paths) => state(paths);

const OLD = '/Volumes/2TB/GVP/Multi Generated/Scene/Scene.mp4';
const OLD_SCRIPT = '/Volumes/2TB/GVP/Multi Generated/Scene/Scene.funscript';

describe('isVideoLocationGone', () => {
  it('video and folder gone, a folder above them still on the mounted drive → gone', async () => {
    await expect(isVideoLocationGone(OLD, fs('/Volumes/2TB', '/Volumes/2TB/GVP'))).resolves.toBe(true);
  });

  it('drive not mounted → never gone', async () => {
    await expect(isVideoLocationGone(OLD, fs('/Volumes'))).resolves.toBe(false);
  });

  it('a stale, empty mount point is not a mounted drive', async () => {
    await expect(isVideoLocationGone(OLD, fs('/Volumes', '/Volumes/2TB'))).resolves.toBe(false);
  });

  it('video present, or its folder present → not this rule', async () => {
    await expect(isVideoLocationGone(OLD, fs('/Volumes/2TB/GVP', OLD))).resolves.toBe(false);
    await expect(isVideoLocationGone(OLD, fs('/Volumes/2TB/GVP', '/Volumes/2TB/GVP/Multi Generated/Scene'))).resolves.toBe(false);
  });

  it('Windows: an absent drive letter never counts, a present one does', async () => {
    const v = 'D:\\lib\\Artist\\Scene\\Scene.mp4';
    await expect(isVideoLocationGone(v, fs())).resolves.toBe(false);
    await expect(isVideoLocationGone(v, fs('D:\\'))).resolves.toBe(false);
    await expect(isVideoLocationGone(v, fs('D:\\lib'))).resolves.toBe(true);
  });

  it('a network share that does not answer never counts', async () => {
    const v = '\\\\nas\\media\\lib\\Scene\\Scene.mp4';
    await expect(isVideoLocationGone(v, fs('\\\\nas\\media'))).resolves.toBe(false);
    await expect(isVideoLocationGone(v, fs('\\\\nas\\media\\lib'))).resolves.toBe(true);
  });

  it('fails closed on errors, relative paths and paths too shallow to prove anything', async () => {
    const boom = vi.fn(async () => { throw new Error('EIO'); });
    await expect(isVideoLocationGone(OLD, boom)).resolves.toBe(false);
    await expect(isVideoLocationGone('Scene/Scene.mp4', fs('Scene'))).resolves.toBe(false);
    await expect(isVideoLocationGone('/Volumes/2TB/Scene/Scene.mp4', fs('/Volumes/2TB'))).resolves.toBe(false);
  });

  it('a check that timed out or errored (EACCES, EIO) on the video or its folder keeps it', async () => {
    const folder = '/Volumes/2TB/GVP/Multi Generated/Scene';
    await expect(isVideoLocationGone(OLD, state(['/Volumes/2TB/GVP'], [OLD]))).resolves.toBe(false);
    await expect(isVideoLocationGone(OLD, state(['/Volumes/2TB/GVP'], [folder]))).resolves.toBe(false);
  });

  it('asks the ancestor first, so a spun-down drive wakes before the checks that matter', async () => {
    const ps = fs('/Volumes/2TB/GVP', '/Volumes/2TB/GVP/Multi Generated');
    await isVideoLocationGone(OLD, ps);
    expect(ps.mock.calls.map((c) => c[0])).toEqual(['/Volumes/2TB/GVP/Multi Generated', OLD, '/Volumes/2TB/GVP/Multi Generated/Scene']);
  });

  it('knows where common mount roots end', () => {
    expect(splitVolumePath('/Volumes/X/a/b.mp4').rootDepth).toBe(2);
    expect(splitVolumePath('/media/dave/X/a/b.mp4').rootDepth).toBe(3);
    expect(splitVolumePath('/run/media/dave/X/a/b.mp4').rootDepth).toBe(4);
    expect(splitVolumePath('/home/dave/a/b.mp4').rootDepth).toBe(1);
    expect(splitVolumePath('C:\\a\\b.mp4').rootDepth).toBe(1);
    expect(splitVolumePath('\\\\nas\\share\\a\\b.mp4').rootDepth).toBe(2);
  });
});

describe('association validation', () => {
  let logSpy;
  let warnSpy;
  beforeEach(() => {
    logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
    warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
  });
  afterEach(() => { vi.restoreAllMocks(); });

  function host(associations, pathState) {
    const store = { 'library.associations': associations };
    global.window = global.window || {};
    window.funsync = { pathState, fileExists: async (p) => (await pathState(p)) === 'present' };
    return {
      store,
      _settings: { get: (k) => store[k], set: (k, v) => { store[k] = v; } },
      _unavailablePaths: new Set(),
      _failedScanPaths: new Set(),
      _isUnderUnavailableSource: Library.prototype._isUnderUnavailableSource,
      _safeToPrune: Library.prototype._safeToPrune,
      validate() { return Library.prototype._validateAssociationsInBackground.call(this); },
    };
  }

  const LIVE = '/Volumes/2TB/GVP/Artist/Live/Live.mp4';
  const LIVE_SCRIPT = '/Volumes/2TB/GVP/Artist/Live/Live.funscript';

  it('removes the whole entry when the video and its folder are gone from a mounted drive', async () => {
    const h = host({
      [OLD]: buildAssociationEntry('single', OLD_SCRIPT, { main: OLD_SCRIPT, axes: {} }, null),
      [LIVE]: buildAssociationEntry('single', LIVE_SCRIPT, null, null),
    }, fs('/Volumes/2TB', '/Volumes/2TB/GVP', '/Volumes/2TB/GVP/Artist', '/Volumes/2TB/GVP/Artist/Live', LIVE, LIVE_SCRIPT));
    await h.validate();
    expect(Object.keys(h.store['library.associations'])).toEqual([LIVE]);
    expect(logSpy).toHaveBeenCalledWith(expect.stringMatching(/removed 1 association/));
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it('keeps the entry untouched when the drive is not mounted', async () => {
    const before = buildAssociationEntry('single', OLD_SCRIPT, null, null);
    const h = host({ [OLD]: before }, fs('/Volumes'));
    await h.validate();
    expect(h.store['library.associations']).toEqual({ [OLD]: before });
    expect(logSpy).not.toHaveBeenCalledWith(expect.stringMatching(/removed/));
    expect(warnSpy).toHaveBeenCalledWith(expect.stringMatching(/LEFT 1 association/));
  });

  it('keeps the entry when the drive answers but the video or folder check times out (EACCES alike)', async () => {
    const before = buildAssociationEntry('single', OLD_SCRIPT, null, null);
    for (const unsure of [OLD, '/Volumes/2TB/GVP/Multi Generated/Scene']) {
      const h = host({ [OLD]: before }, state(['/Volumes/2TB', '/Volumes/2TB/GVP', '/Volumes/2TB/GVP/Multi Generated'], [unsure]));
      await h.validate();
      expect(h.store['library.associations']).toEqual({ [OLD]: before });
    }
    expect(logSpy).not.toHaveBeenCalledWith(expect.stringMatching(/removed/));
  });

  it('video present, script missing in a folder that answers: drops the single slot as before', async () => {
    const gone = '/Volumes/2TB/GVP/Artist/Live/old.funscript';
    const h = host({ [LIVE]: buildAssociationEntry('single', gone, null, null) },
      fs('/Volumes/2TB/GVP/Artist/Live', LIVE));
    await h.validate();
    expect(h.store['library.associations'][LIVE]).toMatchObject({ single: null, active: null });
    expect(logSpy).toHaveBeenCalledWith(expect.stringMatching(/pruned 1/));
  });

  it('video present, script folder unreachable: left untouched as before', async () => {
    const before = buildAssociationEntry('single', 'D:\\funsc\\Live.funscript', null, null);
    const h = host({ [LIVE]: before }, fs(LIVE));
    await h.validate();
    expect(h.store['library.associations']).toEqual({ [LIVE]: before });
    expect(warnSpy).toHaveBeenCalledWith(expect.stringMatching(/LEFT 1 association/));
  });
});
