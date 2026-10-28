/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// library:reorganize: which source folder each scene goes to, what is left
// alone, the sources afterwards, and a full run (move, path rewrite, undo) on
// a temp library.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { planReorganize, sceneSource, sourcesAfter } from '../../scripts/library-reorganize-lib.mjs';

describe('sceneSource', () => {
  it('goes by the sidecar first, then an EroScripts link, else Other', () => {
    expect(sceneSource({ files: ['a.mp4', 'eroscripts.json'] })).toBe('EroScripts');
    expect(sceneSource({ files: ['a.mp4', 'faptap.json', 'eroscripts.json'] })).toBe('EroScripts');
    // A FapTap download later linked to the same script's post came from FapTap.
    expect(sceneSource({ files: ['a.mp4', 'faptap.json'], linked: true })).toBe('FapTap');
    expect(sceneSource({ files: ['a.mp4', 'a.funscript'], linked: true })).toBe('EroScripts');
    expect(sceneSource({ files: ['a.mp4', 'a.funscript'] })).toBe('Other');
  });
});

describe('planReorganize', () => {
  const scene = (dir, files, sourcePath = '/GVP/Multi-Axis') => ({ dir, files, sourcePath });

  it('keeps the path below the source and leaves conflicts where they are', () => {
    const scenes = [
      scene('/GVP/Multi-Axis/Misiv/One', ['eroscripts.json']),
      scene('/GVP/_incoming/Zoot/Yoga', ['faptap.json'], '/GVP/_incoming'),
      scene('/GVP/Multi Generated/Misiv/One', ['x.mp4'], '/GVP/Multi Generated'),
      scene('/GVP/Multi-Axis/Taken/Scene', ['x.mp4']),
      scene('/GVP/Multi-Axis', ['loose.mp4']),
      scene('/GVP/EroScripts/Done/Scene', ['eroscripts.json'], '/GVP/EroScripts'),
    ];
    const plan = planReorganize({ scenes, root: '/GVP', exists: (p) => p === '/GVP/Other/Taken/Scene' });
    expect(plan.moves).toEqual([
      { from: '/GVP/Multi-Axis/Misiv/One', to: '/GVP/EroScripts/Misiv/One', files: [] },
      { from: '/GVP/_incoming/Zoot/Yoga', to: '/GVP/FapTap/Zoot/Yoga', files: [] },
      { from: '/GVP/Multi Generated/Misiv/One', to: '/GVP/Other/Misiv/One', files: [] },
    ]);
    expect(plan.conflicts.map((c) => [c.dir, c.reason])).toEqual([
      ['/GVP/Multi-Axis/Taken/Scene', 'target already exists'],
      ['/GVP/Multi-Axis', 'not in a folder of its own under its source'],
    ]);
    expect(plan.counts).toEqual({ EroScripts: 2, FapTap: 1, Other: 3 });
  });

  it('refuses two scenes that would land in one folder, whatever the case', () => {
    const plan = planReorganize({
      root: '/GVP',
      scenes: [scene('/GVP/Multi-Axis/Misiv/One', ['x.mp4']), scene('/GVP/Multi Generated/misiv/one', ['y.mp4'], '/GVP/Multi Generated')],
    });
    expect(plan.moves.map((m) => m.from)).toEqual(['/GVP/Multi-Axis/Misiv/One']);
    expect(plan.conflicts[0]).toMatchObject({ dir: '/GVP/Multi Generated/misiv/one', reason: 'same target as /GVP/Multi-Axis/Misiv/One' });
  });

  it('moves a scene folder nested in another along with it', () => {
    const plan = planReorganize({
      root: '/GVP',
      scenes: [scene('/GVP/Multi-Axis/A/Scene', ['eroscripts.json']), scene('/GVP/Multi-Axis/A/Scene/Extras', ['b.mp4'])],
    });
    expect(plan.moves).toEqual([{ from: '/GVP/Multi-Axis/A/Scene', to: '/GVP/EroScripts/A/Scene', files: [] }]);
  });
});

describe('sourcesAfter', () => {
  it('puts the new folders first, keeps an existing one, and drops emptied sources only', () => {
    const sources = [
      { id: 'm', name: 'Multi-Axis', path: '/GVP/Multi-Axis', enabled: true },
      { id: 'e', name: 'Old name', path: '/GVP/EroScripts', enabled: false, extra: 1 },
      { id: 'x', name: 'Elsewhere', path: '/Other/Lib', enabled: true },
    ];
    let n = 0;
    const after = sourcesAfter(sources, { root: '/GVP', folders: ['FapTap', 'EroScripts'], emptied: ['/GVP/Multi-Axis'], newId: () => `n${++n}` });
    expect(after).toEqual([
      { id: 'e', name: 'EroScripts', path: '/GVP/EroScripts', enabled: true, extra: 1 },
      { id: 'n1', name: 'FapTap', path: '/GVP/FapTap', enabled: true },
      { id: 'x', name: 'Elsewhere', path: '/Other/Lib', enabled: true },
    ]);
  });
});

describe('library:reorganize CLI', () => {
  let tmp; let gvp; let userData;
  const cli = path.resolve('scripts/library-reorganize.mjs');
  const run = (...args) => spawnSync(process.execPath, [cli, '--user-data', userData, ...args], { encoding: 'utf8' });
  const put = (p, text = 'x') => { fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, text); };
  const readJson = (name) => JSON.parse(fs.readFileSync(path.join(userData, name), 'utf8'));

  beforeEach(() => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'reorganize-'));
    gvp = path.join(tmp, 'GVP');
    userData = path.join(tmp, 'userData');
    put(path.join(gvp, 'Multi-Axis/Misiv/One/One.mp4'));
    put(path.join(gvp, 'Multi-Axis/Misiv/One/One.twist.funscript'));
    put(path.join(gvp, 'Multi Generated/Cock Hero/Round/Round.mp4'));
    put(path.join(gvp, '_incoming/Zoot/Yoga/Yoga.mp4'));
    put(path.join(gvp, '_incoming/Zoot/Yoga/faptap.json'), '{}');
    put(path.join(gvp, '_incoming/Aidenhet/Lesson/Lesson.mp4'));
    put(path.join(gvp, '_incoming/Aidenhet/Lesson/eroscripts.json'), '{}');
    const one = path.join(gvp, 'Multi-Axis/Misiv/One/One.mp4');
    const yoga = path.join(gvp, '_incoming/Zoot/Yoga/Yoga.mp4');
    const round = path.join(gvp, 'Multi Generated/Cock Hero/Round/Round.mp4');
    put(path.join(userData, 'config.json'), JSON.stringify({
      settings: { library: {
        sources: [
          { id: 'a', name: 'Multi-Axis', path: path.join(gvp, 'Multi-Axis'), enabled: true },
          { id: 'g', name: 'Multi Generated', path: path.join(gvp, 'Multi Generated'), enabled: true },
          { id: 'd', name: 'Discovery', path: path.join(gvp, '_incoming'), enabled: true },
        ],
        ratings: { [one]: 5, [yoga]: 4 },
        associations: { [one]: { axes: { twist: path.join(gvp, 'Multi-Axis/Misiv/One/One.twist.funscript') } } },
        resumePositions: { [round]: { position: 30 } },
      } },
      playlists: [{ id: 'p', name: 'Keep', videoPaths: [one, yoga] }],
      videoCategories: { [round]: ['c1'] },
    }, null, '\t'));
    put(path.join(userData, 'eroscripts-links.json'), JSON.stringify({ version: 1, links: { [one]: { topicId: 1 } }, review: {} }, null, 2));
  });

  afterEach(() => fs.rmSync(tmp, { recursive: true, force: true }));

  it('moves every scene to its source folder, rewrites the paths, and undo puts it all back', () => {
    const before = fs.readFileSync(path.join(userData, 'config.json'), 'utf8');
    const dry = run('--dry-run');
    expect(dry.status).toBe(0);
    expect(fs.existsSync(path.join(gvp, 'EroScripts'))).toBe(false);

    const r = run();
    expect(r.status, r.stdout + r.stderr).toBe(0);
    const one = path.join(gvp, 'EroScripts/Misiv/One/One.mp4');
    const yoga = path.join(gvp, 'FapTap/Zoot/Yoga/Yoga.mp4');
    const round = path.join(gvp, 'Other/Cock Hero/Round/Round.mp4');
    for (const p of [one, yoga, round, path.join(gvp, 'EroScripts/Aidenhet/Lesson/Lesson.mp4'), path.join(gvp, 'EroScripts/Misiv/One/One.twist.funscript')]) {
      expect(fs.existsSync(p), p).toBe(true);
    }
    for (const old of ['Multi-Axis', 'Multi Generated', '_incoming']) expect(fs.existsSync(path.join(gvp, old)), old).toBe(false);

    const config = readJson('config.json');
    const lib = config.settings.library;
    expect(lib.sources.map((s) => s.name)).toEqual(['EroScripts', 'FapTap', 'Other']);
    expect(lib.ratings).toEqual({ [one]: 5, [yoga]: 4 });
    expect(lib.associations[one].axes.twist).toBe(path.join(gvp, 'EroScripts/Misiv/One/One.twist.funscript'));
    expect(lib.resumePositions[round]).toEqual({ position: 30 });
    expect(config.playlists[0].videoPaths).toEqual([one, yoga]);
    expect(config.videoCategories).toEqual({ [round]: ['c1'] });
    expect(Object.keys(readJson('eroscripts-links.json').links)).toEqual([one]);
    expect(fs.readFileSync(path.join(userData, 'config.json.bak-reorganize'), 'utf8')).toBe(before);

    const log = fs.readdirSync(path.join(userData, 'logs')).find((n) => n.startsWith('library-reorganize-'));
    const u = run('--undo', path.join(userData, 'logs', log));
    expect(u.status, u.stdout + u.stderr).toBe(0);
    expect(JSON.parse(fs.readFileSync(path.join(userData, 'config.json'), 'utf8'))).toEqual(JSON.parse(before));
    expect(fs.existsSync(path.join(gvp, 'Multi-Axis/Misiv/One/One.twist.funscript'))).toBe(true);
    for (const f of ['EroScripts', 'FapTap', 'Other']) expect(fs.existsSync(path.join(gvp, f)), f).toBe(false);
  });

  it('refuses while the app holds its lock', () => {
    fs.symlinkSync(`host-${process.pid}`, path.join(userData, 'SingletonLock'));
    expect(run().status).toBe(2);
    expect(fs.existsSync(path.join(gvp, 'Multi-Axis/Misiv/One/One.mp4'))).toBe(true);
  });
});
