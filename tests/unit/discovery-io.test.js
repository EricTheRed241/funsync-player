/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// What the scouts take for the library's creators (scripts/discovery-io.mjs
// libraryState): artist folders, aliases (ARTIST_ALIASES) under the artist
// they name.
import { describe, it, expect, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { libraryState } from '../../scripts/discovery-io.mjs';

let tmp = null;
afterEach(() => { if (tmp) fs.rmSync(tmp, { recursive: true, force: true }); tmp = null; });

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
