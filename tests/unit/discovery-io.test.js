/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// What the scouts take for the library's creators (scripts/discovery-io.mjs
// libraryState): artist folders, aliases (ARTIST_ALIASES) under the artist
// they name. writeAtomic: the scout list and state are written so that a run
// of another kind reading them meanwhile never sees half a file.
import { describe, it, expect, afterEach, vi } from 'vitest';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { libraryState, writeAtomic } from '../../scripts/discovery-io.mjs';

let tmp = null;
afterEach(() => { vi.restoreAllMocks(); if (tmp) fs.rmSync(tmp, { recursive: true, force: true }); tmp = null; });

const scene = (root, artist, name) => {
  fs.mkdirSync(path.join(root, artist, name), { recursive: true });
  fs.writeFileSync(path.join(root, artist, name, `${name}.mp4`), '');
};

describe('libraryState', () => {
  it('counts alias folders as the artist they name', () => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'discovery-io-'));
    const a = path.join(tmp, 'EroScripts');
    const b = path.join(tmp, 'FapTap');
    scene(a, 'Rondoudou', 'One');
    scene(a, 'Rondoudou Media', 'Two');
    scene(b, 'RondoudouMedia', 'Three');
    scene(a, 'S022', 'Cute Command');
    scene(a, 'Misiv', 'Four');
    scene(b, 'Horny Herring Studios', 'Five');
    const s = libraryState([a, b]);
    expect([...s.creators].sort()).toEqual(['horny herring', 'miffn', 'misiv', 'rondoudou']);
    expect([...s.creatorNames].sort()).toEqual(['Horny Herring', 'Misiv', 'Rondoudou', 'miffn']);
    expect(s.videos).toHaveLength(6);
  });
});

describe('writeAtomic', () => {
  it('replaces the file through a temporary one and leaves none behind', () => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'discovery-io-'));
    const list = path.join(tmp, 'eroscripts-scout.json');
    writeAtomic(list, '{"candidates":[1]}');
    writeAtomic(list, '{"candidates":[1,2]}');
    expect(fs.readFileSync(list, 'utf8')).toBe('{"candidates":[1,2]}');
    expect(fs.readdirSync(tmp)).toEqual(['eroscripts-scout.json']);
  });

  it('a write that fails halfway leaves the old file whole and no temporary file', () => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'discovery-io-'));
    const list = path.join(tmp, 'eroscripts-scout.json');
    writeAtomic(list, '{"candidates":[1]}');
    const real = fs.writeFileSync.bind(fs);
    vi.spyOn(fs, 'writeFileSync').mockImplementationOnce((file, text) => {
      real(file, String(text).slice(0, 5));
      throw new Error('ENOSPC: no space left on device');
    });
    expect(() => writeAtomic(list, '{"candidates":[1,2,3]}')).toThrow(/ENOSPC/);
    expect(fs.readFileSync(list, 'utf8')).toBe('{"candidates":[1]}');
    expect(fs.readdirSync(tmp)).toEqual(['eroscripts-scout.json']);
  });

  it('a reader in another process sees a whole list on every read while it is rewritten', async () => {
    tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'discovery-io-'));
    const list = path.join(tmp, 'eroscripts-scout.json');
    const done = path.join(tmp, 'done');
    const big = (n) => JSON.stringify({ n, candidates: Array.from({ length: 20_000 }, (_, i) => ({ id: i, title: `post ${i} ${'x'.repeat(40)}` })) });
    writeAtomic(list, big(0));
    // Reads and parses the list until `done` appears; prints `<reads> <broken reads>`.
    const reader = spawn(process.execPath, ['-e', `
      const fs = require('fs');
      let reads = 0, broken = 0;
      while (!fs.existsSync(${JSON.stringify(done)})) {
        try { JSON.parse(fs.readFileSync(${JSON.stringify(list)}, 'utf8')); } catch { broken++; }
        reads++;
      }
      console.log(reads + ' ' + broken);
    `]);
    let out = '';
    reader.stdout.on('data', (d) => { out += d; });
    const ended = new Promise((resolve) => reader.on('close', resolve));
    await new Promise((resolve) => setTimeout(resolve, 200));
    for (let n = 1; n <= 40; n++) writeAtomic(list, big(n));
    fs.writeFileSync(done, '');
    await ended;
    const [reads, broken] = out.trim().split(' ').map(Number);
    expect(reads).toBeGreaterThan(0);
    expect(broken).toBe(0);
    expect(JSON.parse(fs.readFileSync(list, 'utf8')).n).toBe(40);
  }, 30_000);
});
