/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// EroScriptsAPI.fetchScript reads a forum upload into memory so the sync
// can compare it with a local script. Forum links only; the session cookie
// must not follow the redirect to the upload CDN.
import { describe, it, expect, vi, beforeAll, afterAll, afterEach } from 'vitest';

// Silence the real logger the CommonJS module holds (see eroscripts-auth-expiry.test.js).
const log = (await import('../../electron/logger.js')).default;
const { EroScriptsAPI } = await import('../../electron/eroscripts-api.js');

let prev;
const realFetch = globalThis.fetch;
beforeAll(() => {
  prev = [log.transports.file.level, log.transports.console.level];
  log.transports.file.level = false;
  log.transports.console.level = false;
});
afterAll(() => { [log.transports.file.level, log.transports.console.level] = prev; });
afterEach(() => { globalThis.fetch = realFetch; });

const FORUM_FILE = 'https://discuss.eroscripts.com/uploads/short-url/a.funscript';
const CDN_FILE = 'https://eroscripts-discourse.eroscripts.com/original/4X/a/b/c/abc.funscript';

function api() {
  const a = new EroScriptsAPI();
  a.restoreSession('SECRET', 'me');
  return a;
}

describe('EroScriptsAPI.fetchScript', () => {
  it('follows the forum redirect to the CDN without the cookie', async () => {
    const calls = [];
    globalThis.fetch = vi.fn(async (url, opts) => {
      calls.push({ url, cookie: opts.headers.Cookie });
      if (url === FORUM_FILE) return new Response(null, { status: 302, headers: { location: CDN_FILE } });
      return new Response('{"actions":[]}', { status: 200 });
    });
    const r = await api().fetchScript(FORUM_FILE);
    expect(r.success).toBe(true);
    expect(r.bytes.toString()).toBe('{"actions":[]}');
    expect(calls).toEqual([{ url: FORUM_FILE, cookie: '_t=SECRET' }, { url: CDN_FILE, cookie: undefined }]);
  });

  it('refuses links that do not start on the forum', async () => {
    globalThis.fetch = vi.fn();
    expect((await api().fetchScript(CDN_FILE)).success).toBe(false);
    expect((await api().fetchScript('http://discuss.eroscripts.com/uploads/a.funscript')).success).toBe(false);
    expect((await api().fetchScript('https://evil.example/a.funscript')).success).toBe(false);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('flags rate limiting and an expired session', async () => {
    globalThis.fetch = vi.fn(async () => new Response('', { status: 429 }));
    expect(await api().fetchScript(FORUM_FILE)).toMatchObject({ success: false, rateLimited: true });
    globalThis.fetch = vi.fn(async () => new Response('', { status: 403 }));
    expect(await api().fetchScript(FORUM_FILE)).toMatchObject({ success: false, authExpired: true });
  });

  it('caps the size', async () => {
    globalThis.fetch = vi.fn(async () => new Response('x'.repeat(100), { status: 200 }));
    expect(await api().fetchScript(FORUM_FILE, { maxBytes: 10 })).toMatchObject({ success: false, error: 'File too large' });
  });
});
