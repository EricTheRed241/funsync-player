/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Hand-made axis scripts always win, per folder: `npm run upgrade:axes`
// never writes into a folder with hand-made axes, and `npm run
// prune:generated` extracts embedded axes, copies hand-made axes to sibling
// videos whose timing matches and archives the folder's generated files.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { planUpgrade } from '../../renderer/js/upgrade-multi-axis.js';
import { groupScenes, timingMatch } from '../../scripts/handmade-scenes.mjs';
import { sceneVideoStem } from '../../scripts/upgrade-pairing.mjs';
import { planPrune, applyPrune, archiveFiles } from '../../scripts/prune-generated-lib.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const STROKE = Array.from({ length: 40 }, (_, i) => ({ at: i * 250, pos: i % 2 ? 90 : 10 }));
const two = [{ at: 0, pos: 20 }, { at: 500, pos: 80 }];

function run(script, args) {
  const r = spawnSync(process.execPath, [path.join(ROOT, 'scripts', script), ...args], { encoding: 'utf8' });
  return { code: r.status, out: r.stdout + r.stderr };
}

let tmp;
const p = (...parts) => path.join(tmp, ...parts);
function put(rel, content = '') {
  fs.mkdirSync(path.dirname(p(rel)), { recursive: true });
  fs.writeFileSync(p(rel), typeof content === 'string' ? content : JSON.stringify(content));
}
const stroke = (extra = {}) => ({ version: '1.0', actions: STROKE, ...extra });
const exists = (rel) => fs.existsSync(p(rel));
const read = (rel) => fs.readFileSync(p(rel), 'utf8');

beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'handmade-wins-')); });
afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

describe('planUpgrade', () => {
  it('writes nothing for a script with hand-made axes', () => {
    const plan = planUpgrade({ primaryPath: '/v/Title.funscript', strokeActions: STROKE, handmadeAxes: ['twist'] });
    expect(plan.writes).toEqual([]);
    expect(plan.skipped).toEqual([]);
    expect(plan.error).toBe('handmade-axes');
  });

  it('plans as before with no hand-made axes', () => {
    const plan = planUpgrade({ primaryPath: '/v/Title.funscript', strokeActions: STROKE, handmadeAxes: [] });
    expect(plan.error).toBeNull();
    expect(plan.writes.map((w) => w.suffix)).toEqual(['twist', 'roll', 'pitch']);
  });
});

describe('scenes', () => {
  it('sceneVideoStem follows the library pairing rules', () => {
    const videos = ['Title', 'Title_4k'];
    expect(sceneVideoStem('Title', videos)).toBe('Title');
    expect(sceneVideoStem('Title_4k', videos)).toBe('Title_4k');
    expect(sceneVideoStem('Title_hardcore', videos)).toBe('Title');
    expect(sceneVideoStem('Title (Soft)', videos)).toBe('Title');
    expect(sceneVideoStem('Lost', videos)).toBeNull();
  });

  it('groupScenes puts variants with their video and ignores AppleDouble files', () => {
    expect(groupScenes([
      'Title.mp4', 'Title.funscript', 'Title_hard.funscript', 'Title.twist.funscript',
      '._Title (Soft).funscript', 'Orphan.funscript', 'NoScript.mp4',
    ])).toEqual([
      { video: 'Title.mp4', scripts: ['Title.funscript', 'Title_hard.funscript'] },
      { video: null, scripts: ['Orphan.funscript'] },
    ]);
  });
});

describe('upgrade:axes skips scenes with hand-made axes', () => {
  it('a hand-made companion: nothing written, and it says why', () => {
    put('lib/A/A.mp4');
    put('lib/A/A.funscript', stroke());
    put('lib/A/A.roll.funscript', { actions: two });
    const r = run('upgrade-multi-axis.mjs', [p('lib')]);
    expect(r.code).toBe(0);
    expect(r.out).toContain('has hand-made axes (roll), nothing generated');
    expect(fs.readdirSync(p('lib/A')).filter((n) => n.includes('.generated.'))).toEqual([]);
  });

  it('axes embedded as `channels`: nothing written', () => {
    put('lib/B/B.mp4');
    put('lib/B/B.funscript', stroke({ channels: { twist: { actions: two } } }));
    const r = run('upgrade-multi-axis.mjs', [p('lib')]);
    expect(r.out).toContain('has hand-made axes (twist)');
    expect(fs.readdirSync(p('lib/B')).filter((n) => n.includes('.generated.'))).toEqual([]);
  });

  it("a variant of a video with hand-made axes is skipped too; other scenes still upgrade", () => {
    put('lib/C/C.mp4');
    put('lib/C/C.funscript', stroke());
    put('lib/C/C.pitch.funscript', { actions: two });
    put('lib/C/C_hard.funscript', stroke());
    put('lib/D/D.mp4');
    put('lib/D/D.funscript', stroke());
    const r = run('upgrade-multi-axis.mjs', [p('lib')]);
    expect(r.code).toBe(0);
    expect(r.out).toContain(`skip ${p('lib/C/C_hard.funscript')}: has hand-made axes (pitch)`);
    expect(fs.readdirSync(p('lib/C')).filter((n) => n.includes('.generated.'))).toEqual([]);
    expect(fs.readdirSync(p('lib/D')).filter((n) => n.includes('.generated.')).sort())
      .toEqual(['D.generated.pitch.funscript', 'D.generated.roll.funscript', 'D.generated.twist.funscript']);
  });
});

describe('prune:generated', () => {
  function library() {
    // Hand-made companion + generated companions, plus a variant's generated ones.
    put('Lib/Studio/Hand/Hand.mp4');
    put('Lib/Studio/Hand/Hand.funscript', stroke());
    put('Lib/Studio/Hand/Hand.twist.funscript', { actions: two });
    put('Lib/Studio/Hand/Hand.generated.twist.funscript', { actions: two });
    put('Lib/Studio/Hand/Hand.generated.roll.funscript', { actions: two });
    put('Lib/Studio/Hand/Hand_hard.funscript', stroke());
    put('Lib/Studio/Hand/Hand_hard.generated.pitch.funscript', { actions: two });
    put('Lib/Studio/Hand/._Hand.generated.pitch.funscript', 'appledouble');
    // Embedded `channels` axes; a roll companion already exists under an alias.
    put('Lib/Emb/Emb.mp4');
    put('Lib/Emb/Emb.funscript', stroke({ channels: { twist: { actions: two }, roll: two, suction: two, pitch: [two[0]] } }));
    put('Lib/Emb/Emb.r1.funscript', { actions: two });
    put('Lib/Emb/Emb.generated.twist.funscript', { actions: two });
    // Stroke only: untouched.
    put('Lib/Plain/Plain.mp4');
    put('Lib/Plain/Plain.funscript', stroke());
    put('Lib/Plain/Plain.generated.twist.funscript', { actions: two });
    // AppleDouble main script with "embedded axes": ignored.
    put('Lib/Apple/Apple.mp4');
    put('Lib/Apple/._Apple.funscript', stroke({ channels: { twist: two } }));
    put('Lib/Apple/Apple.generated.twist.funscript', { actions: two });
  }
  const archive = () => p('Archive');

  it('--dry-run lists the work and changes nothing', () => {
    library();
    const before = fs.readdirSync(p('Lib'), { recursive: true }).sort();
    const r = run('prune-generated.mjs', ['--dry-run', '--archive', archive(), p('Lib')]);
    expect(r.code).toBe(0);
    expect(r.out).toContain(`scene    ${p('Lib/Studio/Hand/Hand.mp4')}`);
    expect(r.out).toContain(`extract  ${p('Lib/Emb/Emb.twist.funscript')}`);
    expect(r.out).toContain(`archive  ${p('Lib/Studio/Hand/Hand_hard.generated.pitch.funscript')}`);
    expect(r.out).toContain(`→  ${path.join(archive(), 'Lib/Studio/Hand/Hand.generated.roll.funscript')}`);
    expect(r.out).not.toContain('Plain');
    expect(r.out).not.toContain('Apple');
    expect(r.out).toContain('2 folders with hand-made axes (2 with changes), 2 scenes with hand-made axes, 2 embedded axes to extract, '
      + '0 hand-made axes to copy to 0 siblings, 0 siblings not timing-compatible, 4 generated files to archive (dry run');
    expect(fs.readdirSync(p('Lib'), { recursive: true }).sort()).toEqual(before);
    expect(exists('Archive')).toBe(false);
  });

  it('a real run extracts, archives and writes the manifest; never overwrites', () => {
    library();
    put('Lib/Emb/Emb.suck.funscript', 'pre-existing');
    const r = run('prune-generated.mjs', ['--archive', archive(), p('Lib')]);
    expect(r.code).toBe(0);
    expect(r.out).toContain('1 embedded axes extracted, 0 hand-made axes copied to siblings, 0 siblings not timing-compatible, 4 generated files archived');

    // Extracted with canonical suffix; existing files and aliases kept as they were.
    expect(JSON.parse(read('Lib/Emb/Emb.twist.funscript')).actions).toEqual(two);
    expect(read('Lib/Emb/Emb.suck.funscript')).toBe('pre-existing');
    expect(exists('Lib/Emb/Emb.roll.funscript')).toBe(false);
    expect(exists('Lib/Emb/Emb.pitch.funscript')).toBe(false);
    expect(JSON.parse(read('Lib/Emb/Emb.funscript')).channels).toBeDefined();

    // Generated companions moved, keeping the path below the library's parent.
    for (const rel of [
      'Lib/Studio/Hand/Hand.generated.twist.funscript',
      'Lib/Studio/Hand/Hand.generated.roll.funscript',
      'Lib/Studio/Hand/Hand_hard.generated.pitch.funscript',
      'Lib/Emb/Emb.generated.twist.funscript',
    ]) {
      expect(exists(rel)).toBe(false);
      expect(exists(path.join('Archive', rel))).toBe(true);
    }
    expect(exists('Lib/Studio/Hand/Hand.twist.funscript')).toBe(true);
    expect(exists('Lib/Studio/Hand/._Hand.generated.pitch.funscript')).toBe(true);
    expect(exists('Lib/Plain/Plain.generated.twist.funscript')).toBe(true);
    expect(exists('Lib/Apple/Apple.generated.twist.funscript')).toBe(true);

    const manifest = read('Archive/MANIFEST.txt').trim().split('\n');
    expect(manifest).toHaveLength(5);
    expect(manifest.slice(1)).toContain(
      `${p('Lib/Emb/Emb.generated.twist.funscript')}\t${path.join(archive(), 'Lib/Emb/Emb.generated.twist.funscript')}`,
    );

    // A second run has nothing left to do and keeps the manifest.
    const again = run('prune-generated.mjs', ['--archive', archive(), p('Lib')]);
    expect(again.out).toContain('0 embedded axes extracted, 0 hand-made axes copied to siblings, 0 siblings not timing-compatible, 0 generated files archived');
    expect(read('Archive/MANIFEST.txt').trim().split('\n')).toHaveLength(5);
  });

  it('planPrune leaves a scene without hand-made axes out', async () => {
    put('Lib/Plain/Plain.mp4');
    put('Lib/Plain/Plain.funscript', stroke());
    put('Lib/Plain/Plain.generated.twist.funscript', { actions: two });
    expect(await planPrune([p('Lib')], archive())).toEqual([]);
  });

  it('archiveFiles never replaces an archived file and appends to the manifest', async () => {
    put('Lib/X/X.generated.twist.funscript', 'new');
    put('Archive/Lib/X/X.generated.twist.funscript', 'old');
    put('Archive/MANIFEST.txt', 'header\n');
    const [res] = await archiveFiles([p('Lib/X/X.generated.twist.funscript')], { archiveDir: archive(), relativeTo: tmp });
    expect(res.error).toMatch(/archive target exists/);
    expect(read('Lib/X/X.generated.twist.funscript')).toBe('new');
    expect(read('Archive/Lib/X/X.generated.twist.funscript')).toBe('old');
    expect(read('Archive/MANIFEST.txt')).toBe('header\n');
  });

  it('needs --archive and a folder', () => {
    expect(run('prune-generated.mjs', ['--dry-run', p('Lib')]).code).toBe(1);
    expect(run('prune-generated.mjs', ['--archive', archive()]).code).toBe(1);
  });
});

describe('sibling videos: hand-made axes win per folder', () => {
  const archive = () => p('Archive');
  const shifted = (ms, drop = 0) => STROKE.slice(0, STROKE.length - drop).map((a) => ({ at: a.at + ms, pos: a.pos }));
  // FP_hard has hand-made axes; FP (another version of the scene) only generated ones.
  function folder(siblingActions, { embedded = false } = {}) {
    put('Lib/FP/FP_hard.mp4');
    if (embedded) {
      put('Lib/FP/FP_hard.funscript', stroke({ channels: { twist: { actions: two } } }));
    } else {
      put('Lib/FP/FP_hard.funscript', stroke());
      put('Lib/FP/FP_hard.twist.funscript', JSON.stringify({ actions: two, metadata: { creator: 'scripter' } }, null, 1));
      put('Lib/FP/FP_hard.R1.funscript', { actions: two });
    }
    put('Lib/FP/FP.mp4');
    put('Lib/FP/FP.funscript', { metadata: { title: 'FP' }, actions: siblingActions });
    put('Lib/FP/FP.generated.twist.funscript', { actions: two });
    put('Lib/FP/FP.generated.pitch.funscript', { actions: two });
    // Another folder keeps its generated files.
    put('Lib/Other/Other.mp4');
    put('Lib/Other/Other.funscript', stroke());
    put('Lib/Other/Other.generated.twist.funscript', { actions: two });
  }
  const generatedLeft = () => fs.readdirSync(p('Lib/FP')).filter((n) => n.includes('.generated.'));

  it('timingMatch: identical, near (±50 ms, 95 %, ends within 1 s) or nothing', () => {
    const src = stroke();
    expect(timingMatch(src, { actions: STROKE.map(({ at, pos }) => ({ pos, at })) })).toBe('identical');
    expect(timingMatch(src, { actions: shifted(20, 1) })).toBe('near');
    expect(timingMatch(src, { actions: shifted(125) })).toBe(null);
    expect(timingMatch(src, { actions: shifted(20, 3) })).toBe(null); // 37 of 40 < 95 %
    expect(timingMatch(src, { actions: [...STROKE, { at: 11_000, pos: 0 }] })).toBe(null); // ends 1.25 s later
    expect(timingMatch(src, null)).toBe(null);
  });

  it('identical actions: the hand-made set is copied to the sibling and its generated files are archived', () => {
    folder(STROKE.map(({ at, pos }) => ({ pos, at })));
    const r = run('prune-generated.mjs', ['--archive', archive(), p('Lib')]);
    expect(r.code).toBe(0);
    expect(r.out).toContain(`sibling  ${p('Lib/FP/FP.mp4')}\n         from FP_hard.funscript (identical actions)`);
    expect(r.out).toContain('2 hand-made axes copied to siblings, 0 siblings not timing-compatible, 2 generated files archived');
    expect(read('Lib/FP/FP.twist.funscript')).toBe(read('Lib/FP/FP_hard.twist.funscript'));
    expect(read('Lib/FP/FP.R1.funscript')).toBe(read('Lib/FP/FP_hard.R1.funscript'));
    expect(generatedLeft()).toEqual([]);
    expect(exists('Archive/Lib/FP/FP.generated.pitch.funscript')).toBe(true);
    expect(exists('Lib/Other/Other.generated.twist.funscript')).toBe(true);
    const again = run('prune-generated.mjs', ['--archive', archive(), p('Lib')]);
    expect(again.out).toContain('1 folders with hand-made axes (0 with changes), 2 scenes with hand-made axes');
  });

  it('a near match is accepted', async () => {
    folder(shifted(20, 1));
    const [plan] = await planPrune([p('Lib')], archive());
    expect(plan.siblings).toHaveLength(1);
    expect(plan.siblings[0]).toMatchObject({ video: p('Lib/FP/FP.mp4'), source: p('Lib/FP/FP_hard.funscript'), match: 'near' });
    expect(plan.siblings[0].copy.map((c) => path.basename(c.path)).sort()).toEqual(['FP.R1.funscript', 'FP.twist.funscript']);
  });

  it('a mismatch copies nothing and says so, but the generated files still go', () => {
    folder(shifted(125));
    const r = run('prune-generated.mjs', ['--archive', archive(), p('Lib')]);
    expect(r.code).toBe(0);
    expect(r.out).toContain(`sibling  ${p('Lib/FP/FP.mp4')}\n         sibling not timing-compatible, nothing copied`);
    expect(r.out).toContain('0 hand-made axes copied to siblings, 1 siblings not timing-compatible, 2 generated files archived');
    expect(fs.readdirSync(p('Lib/FP')).filter((n) => n.startsWith('FP.')).sort()).toEqual(['FP.funscript', 'FP.mp4']);
    expect(generatedLeft()).toEqual([]);
  });

  it('never overwrites a file that appears before the copy', async () => {
    folder(STROKE);
    const plans = await planPrune([p('Lib')], archive());
    put('Lib/FP/FP.twist.funscript', 'MINE');
    const res = await applyPrune(plans, archive());
    expect(read('Lib/FP/FP.twist.funscript')).toBe('MINE');
    expect(res.copied).toEqual([p('Lib/FP/FP.R1.funscript')]);
    expect(res.errors).toEqual([expect.stringMatching(/^copy .*FP\.twist\.funscript: .*EEXIST/)]);
  });

  it('an embedded-only source is extracted and its axes copied', () => {
    folder(STROKE, { embedded: true });
    const dry = run('prune-generated.mjs', ['--dry-run', '--archive', archive(), p('Lib')]);
    expect(dry.out).toContain(`extract  ${p('Lib/FP/FP_hard.twist.funscript')}`);
    expect(dry.out).toContain(`copy     ${p('Lib/FP/FP.twist.funscript')}`);
    expect(exists('Lib/FP/FP.twist.funscript')).toBe(false);
    run('prune-generated.mjs', ['--archive', archive(), p('Lib')]);
    expect(JSON.parse(read('Lib/FP/FP.twist.funscript')).actions).toEqual(two);
    expect(JSON.parse(read('Lib/FP/FP_hard.twist.funscript')).actions).toEqual(two);
  });

  it('upgrade:axes skips the whole folder', () => {
    folder(shifted(125));
    for (const n of generatedLeft()) fs.rmSync(p('Lib/FP', n));
    const r = run('upgrade-multi-axis.mjs', [p('Lib')]);
    expect(r.code).toBe(0);
    expect(r.out).toContain(`skip ${p('Lib/FP/FP.funscript')}: hand-made axes in its folder (FP_hard.funscript: roll, twist), nothing generated`);
    expect(generatedLeft()).toEqual([]);
    expect(fs.readdirSync(p('Lib/Other')).filter((n) => n.includes('.generated.')).length).toBeGreaterThan(1);
  });
});
