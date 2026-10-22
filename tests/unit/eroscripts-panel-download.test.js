// EroScripts downloads: where a file lands, and what happens when one is
// already there.
//
// With a video open the side panel renames the download after the video so
// it auto-pairs. That rename used to be `<video>.funscript` for EVERY
// attachment, which turned a `.zip` into a fake funscript and an axis file
// (`Scene.roll.funscript`) into the video's main script — and the main
// process wrote over whatever was already there, on both download surfaces.
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { EroScriptsPanel } from '../../renderer/components/eroscripts-panel.js';
import { Modal } from '../../renderer/components/modal.js';
import { keepBothPath, planTargets } from '../../renderer/js/eroscripts-save.js';

const VIDEO = '/lib/Scene/My Video.mp4';

function makePanel() {
  const store = { 'library.sources': [{ path: '/lib', enabled: true }] };
  const settings = { get: (k) => store[k], set: (k, v) => { store[k] = v; } };
  const panel = new EroScriptsPanel({ settings });
  panel.onScriptDownloaded = vi.fn();
  return panel;
}

/** Make `filesExist` report these paths as present. */
function existing(...paths) {
  const set = new Set(paths);
  window.funsync.filesExist.mockImplementation(async (list) => list.map((p) => set.has(p)));
}

beforeEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = `<div id="app"><div id="player-container" data-video-path="${VIDEO}"></div></div>`;
  window.funsync.eroscriptsDownload.mockReset().mockResolvedValue({ success: true });
  existing();
});

const downloads = () => window.funsync.eroscriptsDownload.mock.calls;

describe('EroScripts panel download naming', () => {
  it('renames a main script after the open video', async () => {
    const panel = makePanel();
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null);
    expect(downloads()).toEqual([['u1', '/lib/Scene/My Video.funscript', { overwrite: false }]]);
    expect(panel.onScriptDownloaded).toHaveBeenCalledWith('/lib/Scene/My Video.funscript', 'My Video.funscript', { axis: null });
  });

  it('keeps the axis suffix, so an axis file never replaces the main script', async () => {
    const panel = makePanel();
    await panel._downloadAttachment({ name: 'Scene.Roll.funscript', url: 'u2' }, null);
    expect(downloads()[0][1]).toBe('/lib/Scene/My Video.roll.funscript');
    expect(panel.onScriptDownloaded).toHaveBeenCalledWith(
      '/lib/Scene/My Video.roll.funscript', 'My Video.roll.funscript', { axis: 'roll' },
    );
  });

  it('never saves a zip as a funscript', async () => {
    const panel = makePanel();
    await panel._downloadAttachment({ name: 'Scene pack.zip', url: 'u3' }, null);
    expect(downloads()).toEqual([]);
    expect(panel.onScriptDownloaded).not.toHaveBeenCalled();
  });
});

describe('EroScripts panel download conflicts', () => {
  it('Cancel writes nothing', async () => {
    existing('/lib/Scene/My Video.funscript');
    const open = vi.spyOn(Modal, 'open').mockResolvedValue(null);
    const panel = makePanel();
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null);
    expect(open).toHaveBeenCalledTimes(1);
    expect(downloads()).toEqual([]);
    expect(panel.onScriptDownloaded).not.toHaveBeenCalled();
  });

  it('Replace overwrites the existing file', async () => {
    existing('/lib/Scene/My Video.funscript');
    vi.spyOn(Modal, 'open').mockResolvedValue('replace');
    const panel = makePanel();
    await panel._downloadAttachment({ name: 'Scene.funscript', url: 'u1' }, null);
    expect(downloads()).toEqual([['u1', '/lib/Scene/My Video.funscript', { overwrite: true }]]);
  });

  it('Keep both picks the first free variant name', async () => {
    existing('/lib/Scene/My Video.roll.funscript', '/lib/Scene/My Video (EroScripts).roll.funscript');
    vi.spyOn(Modal, 'open').mockResolvedValue('keep');
    const panel = makePanel();
    await panel._downloadAttachment({ name: 'Scene.roll.funscript', url: 'u2' }, null);
    expect(downloads()).toEqual([['u2', '/lib/Scene/My Video (EroScripts 2).roll.funscript', { overwrite: false }]]);
  });
});

describe('planTargets', () => {
  it('moves a whole set to one shared name, so axes still pair with their main', async () => {
    existing('/v/A.funscript');
    const plan = await planTargets(['/v/A.funscript', '/v/A.twist.funscript'], async () => 'keep');
    expect(plan).toEqual({ paths: ['/v/A (EroScripts).funscript', '/v/A (EroScripts).twist.funscript'], overwrite: false });
  });

  it('does not ask when nothing is in the way', async () => {
    const ask = vi.fn();
    expect(await planTargets(['/v/A.funscript'], ask)).toEqual({ paths: ['/v/A.funscript'], overwrite: false });
    expect(ask).not.toHaveBeenCalled();
  });
});

describe('keepBothPath', () => {
  it('labels before the axis suffix and handles Windows paths', () => {
    expect(keepBothPath('C:\\v\\My.Video.pitch.funscript', 3)).toBe('C:\\v\\My.Video (EroScripts 3).pitch.funscript');
    expect(keepBothPath('/v/Title 2.0.funscript')).toBe('/v/Title 2.0 (EroScripts).funscript');
  });
});
