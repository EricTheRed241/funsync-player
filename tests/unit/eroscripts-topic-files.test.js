/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Finding the script files in an EroScripts topic.
//
// A survey of ~100 real script posts (2026-10-22) showed free scripts
// outside the first post's attachments: updated files in the author's
// replies, other users' versions in replies, and whole packs on pixeldrain
// (e.g. a free "4 Cock Heros + 4 PMVs" pack with no forum attachments at
// all). Paid Scripts posts have no files and used to read as "no
// attachments found".
import { describe, it, expect, vi, beforeAll, afterAll, afterEach } from 'vitest';

const log = (await import('../../electron/logger.js')).default;
const { EroScriptsAPI, parsePostLinks } = await import('../../electron/eroscripts-api.js');

let prev;
beforeAll(() => {
  prev = [log.transports.file.level, log.transports.console.level];
  log.transports.file.level = false;
  log.transports.console.level = false;
});
afterAll(() => { [log.transports.file.level, log.transports.console.level] = prev; });

const realFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = realFetch; });

const att = (href, name) => `<a class="attachment" href="${href}">${name}</a>`;
const json = (body) => ({ ok: true, status: 200, text: async () => JSON.stringify(body), json: async () => body });

/** Route stubbed fetches by URL substring; records every request. */
function route(table) {
  const calls = [];
  globalThis.fetch = vi.fn(async (url, init = {}) => {
    calls.push({ url: String(url), headers: init.headers || {} });
    const hit = Object.keys(table).find((k) => String(url).includes(k));
    if (!hit) return { ok: false, status: 404, text: async () => '', json: async () => ({}) };
    return json(table[hit]);
  });
  return calls;
}

const CATEGORIES = {
  category_list: { categories: [{ id: 5, slug: 'scripts', subcategory_list: [{ id: 14, slug: 'free-scripts' }, { id: 15, slug: 'paid-scripts' }] }] },
};

describe('parsePostLinks', () => {
  it('finds attachments, pixeldrain links and named external hosts', () => {
    const html = [
      att('/uploads/short-url/a.funscript', 'Luna &amp; Sue.funscript'),
      att('/uploads/short-url/b.txt', 'readme.txt'),
      '<a href="https://pixeldrain.com/l/AbC123#item=2">pack</a>',
      '<a href="https://pixeldrain.com/u/Zz9">vid</a>',
      '<a href="https://mega.nz/folder/x#y">mega</a>',
      '<a href="https://www.patreon.com/c/someone">patreon</a>',
    ].join('');
    const r = parsePostLinks(html);
    expect(r.attachments).toEqual([{ name: 'Luna & Sue.funscript', href: '/uploads/short-url/a.funscript' }]);
    expect(r.pixeldrain).toEqual([{ kind: 'l', id: 'AbC123' }, { kind: 'u', id: 'Zz9' }]);
    expect(r.external.map((e) => e.host)).toEqual(['pixeldrain', 'pixeldrain', 'MEGA']);
  });
});

describe('getTopicAttachments', () => {
  it("lists the author's files first, then replies, and fetches replies past the first page", async () => {
    const calls = route({
      '/t/7.json': {
        category_id: 14,
        post_stream: {
          stream: [1, 2, 3],
          posts: [
            { id: 1, post_number: 1, username: 'author', cooked: att('/u/main.funscript', 'Scene.funscript') },
            { id: 2, post_number: 2, username: 'fan', cooked: att('/u/fan.funscript', 'Scene SR6.funscript') },
          ],
        },
      },
      '/t/7/posts.json?post_ids[]=3': {
        post_stream: { posts: [{ id: 3, post_number: 3, username: 'author', cooked: att('/u/v2.funscript', 'Scene v2.funscript') }] },
      },
      '/categories.json': CATEGORIES,
    });
    const r = await new EroScriptsAPI().getTopicAttachments(7);
    expect(r.attachments.map((a) => [a.name, a.username, a.postNumber, a.byAuthor])).toEqual([
      ['Scene.funscript', 'author', 1, true],
      ['Scene v2.funscript', 'author', 3, true],
      ['Scene SR6.funscript', 'fan', 2, false],
    ]);
    expect(r.attachments[0].url).toBe('https://discuss.eroscripts.com/u/main.funscript');
    expect(r.paid).toBe(false);
    expect(calls.some((c) => c.url.includes('/t/7/posts.json'))).toBe(true);
  });

  it('lists funscripts inside a pixeldrain list as links, without sending the forum cookie there', async () => {
    const calls = route({
      '/t/8.json': {
        category_id: 14,
        post_stream: { stream: [1], posts: [{ id: 1, post_number: 1, username: 'a', cooked: '<a href="https://pixeldrain.com/l/PACK">pack</a>' }] },
      },
      'pixeldrain.com/api/list/PACK': { files: [{ id: 'f1', name: 'Cock Hero - Crush.funscript' }, { id: 'v1', name: 'Cock Hero - Crush.mp4' }] },
      '/categories.json': CATEGORIES,
    });
    const api = new EroScriptsAPI();
    api.restoreSession('secret', 'me');
    const r = await api.getTopicAttachments(8);
    // pixeldrain refuses programmatic downloads on free accounts, so these
    // are page links for the browser, never download candidates.
    expect(r.attachments).toEqual([]);
    expect(r.linked).toEqual([expect.objectContaining({
      name: 'Cock Hero - Crush.funscript', url: 'https://pixeldrain.com/u/f1', host: 'pixeldrain',
    })]);
    expect(r.external.map((e) => e.host)).toEqual(['pixeldrain']);
    const pd = calls.filter((c) => c.url.includes('pixeldrain'));
    expect(pd.length).toBe(1);
    expect(pd[0].headers.Cookie).toBeUndefined();
  });

  it('marks a Paid Scripts post and reports the hosts it links to', async () => {
    route({
      '/t/9.json': {
        category_id: 15,
        post_stream: { stream: [1], posts: [{ id: 1, post_number: 1, username: 'a', cooked: '<a href="https://mega.nz/file/q">get</a>' }] },
      },
      '/categories.json': CATEGORIES,
    });
    const r = await new EroScriptsAPI().getTopicAttachments(9);
    expect(r).toEqual({ attachments: [], linked: [], external: [{ host: 'MEGA', url: 'https://mega.nz/file/q' }], paid: true });
  });
});

describe('downloadFile hosts', () => {
  it('only fetches https links on the forum, which receive the session cookie', async () => {
    globalThis.fetch = vi.fn();
    const api = new EroScriptsAPI();
    api.restoreSession('secret', 'me');
    for (const url of [
      'https://evil.example/a.funscript',
      'https://pixeldrain.com/api/file/f1?download',
      'http://discuss.eroscripts.com/a',
      'file:///etc/passwd',
    ]) {
      expect(await api.downloadFile(url, '/tmp/never')).toEqual({ success: false, error: 'Unsupported download link' });
    }
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });
});
