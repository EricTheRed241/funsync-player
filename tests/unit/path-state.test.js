/**
 * @vitest-environment node
 */
// `pathState` must only answer 'absent' when the OS said so. A timeout or any
// other error is 'unknown': callers that delete data on 'absent'
// (association-dead.js) depend on it.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const require = createRequire(import.meta.url);
const { pathState, pathReachable } = require('../../electron/path-state.js');

const failWith = (code) => async () => { throw Object.assign(new Error(code), { code }); };

let tmp;
beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'path-state-')); });
afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

describe('pathState', () => {
  it('present / absent / ENOTDIR on a real filesystem', async () => {
    const file = path.join(tmp, 'a.mp4');
    fs.writeFileSync(file, '');
    await expect(pathState(file)).resolves.toBe('present');
    await expect(pathState(tmp)).resolves.toBe('present');
    await expect(pathState(path.join(tmp, 'gone.mp4'))).resolves.toBe('absent');
    await expect(pathState(path.join(file, 'child'))).resolves.toBe('absent');
  });

  it('only ENOENT and ENOTDIR mean absent', async () => {
    await expect(pathState('/x', 100, failWith('ENOENT'))).resolves.toBe('absent');
    await expect(pathState('/x', 100, failWith('ENOTDIR'))).resolves.toBe('absent');
    for (const code of ['EACCES', 'EIO', 'EPERM', 'ETIMEDOUT', undefined]) {
      await expect(pathState('/x', 100, failWith(code))).resolves.toBe('unknown');
    }
  });

  it('a check that does not answer in time is unknown', async () => {
    await expect(pathState('/x', 10, () => new Promise(() => {}))).resolves.toBe('unknown');
  });

  it('a non-path is unknown', async () => {
    await expect(pathState('')).resolves.toBe('unknown');
    await expect(pathState(null)).resolves.toBe('unknown');
  });
});

describe('pathReachable', () => {
  it('is true only for a present path', async () => {
    await expect(pathReachable(tmp)).resolves.toBe(true);
    await expect(pathReachable(path.join(tmp, 'gone'))).resolves.toBe(false);
    await expect(pathReachable('')).resolves.toBe(false);
  });
});
