/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// `trash-generated-companions` (electron/generated-companions.js) and the
// renderer side that decides when to call it (handmade-downloads.js).
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { scriptsReplacedBy, trashReplacedGenerated } from '../../renderer/js/handmade-downloads.js';

const require = createRequire(import.meta.url);
const { isGeneratedCompanionOf, trashGeneratedCompanions } = require('../../electron/generated-companions.js');

let dir;
beforeEach(() => { dir = fs.mkdtempSync(path.join(os.tmpdir(), 'trash-generated-')); });
afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });

function touch(...names) {
  for (const n of names) {
    fs.mkdirSync(path.dirname(path.join(dir, n)), { recursive: true });
    fs.writeFileSync(path.join(dir, n), '{}');
  }
}

describe('trashGeneratedCompanions', () => {
  it("trashes only `<stem>.generated.<axis>.funscript` beside the script", async () => {
    touch(
      'Title.funscript',
      'Title.twist.funscript',
      'Title.generated.twist.funscript',
      'Title.GENERATED.R1.funscript',
      'Title.generated.notes.funscript',
      'Title.generated.twist.funscript.bak',
      'Title_hard.generated.pitch.funscript',
      'Other.generated.roll.funscript',
      '._Title.generated.pitch.funscript',
      'sub/Title.generated.roll.funscript',
    );
    const trashItem = vi.fn(async () => {});
    const result = await trashGeneratedCompanions(path.join(dir, 'Title.funscript'), { trashItem });
    const expected = [path.join(dir, 'Title.GENERATED.R1.funscript'), path.join(dir, 'Title.generated.twist.funscript')];
    expect(result.trashed.sort()).toEqual(expected.sort());
    expect(result.failed).toEqual([]);
    expect(trashItem.mock.calls.map((c) => c[0]).sort()).toEqual(expected.sort());
  });

  it('refuses anything that is not an absolute .funscript path', async () => {
    touch('Title.generated.twist.funscript');
    const trashItem = vi.fn(async () => {});
    for (const bad of [
      'Title.funscript',
      path.join(dir, 'Title.mp4'),
      path.join(dir, 'Title.generated.twist.funscript', '..'),
      dir,
      null,
      42,
    ]) {
      expect((await trashGeneratedCompanions(bad, { trashItem })).trashed).toEqual([]);
    }
    expect(trashItem).not.toHaveBeenCalled();
  });

  it('a generated companion named as the script trashes nothing else', async () => {
    touch('Title.generated.twist.funscript', 'Title.generated.twist.generated.roll.funscript');
    const trashItem = vi.fn(async () => {});
    const r = await trashGeneratedCompanions(path.join(dir, 'Title.generated.twist.funscript'), { trashItem });
    expect(r.trashed).toEqual([path.join(dir, 'Title.generated.twist.generated.roll.funscript')]);
  });

  it('reports files the trash refused', async () => {
    touch('Title.generated.twist.funscript');
    const r = await trashGeneratedCompanions(path.join(dir, 'Title.funscript'), {
      trashItem: async () => { throw new Error('no trash'); },
    });
    expect(r).toEqual({ trashed: [], failed: [path.join(dir, 'Title.generated.twist.funscript')] });
  });

  it('isGeneratedCompanionOf matches the stem exactly, any case', () => {
    expect(isGeneratedCompanionOf('title.Generated.suction.funscript', 'Title')).toBe(true);
    expect(isGeneratedCompanionOf('Title 2.generated.twist.funscript', 'Title')).toBe(false);
    expect(isGeneratedCompanionOf('Title.twist.funscript', 'Title')).toBe(false);
  });
});

describe('scriptsReplacedBy', () => {
  const read = (contents) => async (p) => contents[p] ?? null;

  it('maps a hand-made axis file to its main script', async () => {
    expect(await scriptsReplacedBy(['/v/Title.twist.funscript', 'C:\\v\\Title.r1.funscript'], read({})))
      .toEqual(['/v/Title.funscript', 'C:\\v\\Title.funscript']);
  });

  it('counts a main script with embedded axes, not a plain one or a generated file', async () => {
    const contents = {
      '/v/Emb.funscript': JSON.stringify({ actions: [], channels: { twist: [{ at: 0, pos: 1 }, { at: 9, pos: 2 }] } }),
      '/v/Plain.funscript': JSON.stringify({ actions: [{ at: 0, pos: 1 }, { at: 9, pos: 2 }] }),
    };
    expect(await scriptsReplacedBy(
      ['/v/Emb.funscript', '/v/Plain.funscript', '/v/X.generated.twist.funscript', '/v/notes.txt'],
      read(contents),
    )).toEqual(['/v/Emb.funscript']);
  });

  it('trashReplacedGenerated collects what the IPC trashed', async () => {
    const trash = vi.fn(async (p) => ({ trashed: [p.replace('.funscript', '.generated.roll.funscript')], failed: [] }));
    expect(await trashReplacedGenerated(['/v/T.twist.funscript', '/v/T.pitch.funscript'], { readScript: read({}), trash }))
      .toEqual(['/v/T.generated.roll.funscript']);
    expect(trash).toHaveBeenCalledTimes(1);
  });
});
