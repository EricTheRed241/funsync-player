/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// EroScriptsAPI.downloadFile must not replace an existing file unless told
// to: the renderer asks the user first, and a file that appears between
// that question and the write must survive too.
import { describe, it, expect, vi, beforeAll, afterAll, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

// Silence the real logger the CommonJS module holds (vi.mock can't reach it;
// see eroscripts-auth-expiry.test.js).
const log = (await import('../../electron/logger.js')).default;
const { EroScriptsAPI } = await import('../../electron/eroscripts-api.js');

let prev;
let dir;
const realFetch = globalThis.fetch;

beforeAll(() => {
  prev = [log.transports.file.level, log.transports.console.level];
  log.transports.file.level = false;
  log.transports.console.level = false;
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'es-dl-'));
});
afterAll(() => {
  [log.transports.file.level, log.transports.console.level] = prev;
  fs.rmSync(dir, { recursive: true, force: true });
});
afterEach(() => { globalThis.fetch = realFetch; });

function serve(body) {
  globalThis.fetch = vi.fn(async () => ({ ok: true, status: 200, arrayBuffer: async () => new TextEncoder().encode(body).buffer }));
}

describe('EroScriptsAPI.downloadFile', () => {
  it('reports an existing file instead of replacing it', async () => {
    const target = path.join(dir, 'keep.funscript');
    fs.writeFileSync(target, 'mine');
    serve('theirs');
    expect(await new EroScriptsAPI().downloadFile('https://x/a', target)).toEqual({ success: false, exists: true });
    expect(fs.readFileSync(target, 'utf8')).toBe('mine');
  });

  it('replaces it when asked to', async () => {
    const target = path.join(dir, 'replace.funscript');
    fs.writeFileSync(target, 'mine');
    serve('theirs');
    expect(await new EroScriptsAPI().downloadFile('https://x/a', target, { overwrite: true })).toEqual({ success: true });
    expect(fs.readFileSync(target, 'utf8')).toBe('theirs');
  });
});
