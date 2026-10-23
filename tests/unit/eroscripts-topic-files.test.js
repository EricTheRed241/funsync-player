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

const FORUM = 'https://discuss.eroscripts.com';

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
    expect(r.attachments).toEqual([{ name: 'Luna & Sue.funscript', url: `${FORUM}/uploads/short-url/a.funscript` }]);
    expect(r.pixeldrain).toEqual([{ kind: 'l', id: 'AbC123' }, { kind: 'u', id: 'Zz9' }]);
    expect(r.external.map((e) => e.host)).toEqual(['pixeldrain', 'pixeldrain', 'MEGA']);
  });

  // Topic 311478 (multi-axis Fap Hero - Fantasy Play) lists its five axis
  // files as plain links, without class="attachment".
  it('finds plain upload links without class="attachment"', () => {
    const axes = { pitch: '4BMs0Gd7BivjFAhi4Ji3mfctSNy', roll: 'eXg3NUeBUjJm6nRHLCf43MCpZsI', surge: 'btZXaiWVKdaWk6WshWj9MvcoIoR', sway: '1N7h7Wu2rRqjQl7pIrwdcSrVJxK', twist: 'pa3WxdeRSzmtBLUli6tununqOXT' };
    const html = '<p><a href="https://discuss.eroscripts.com/t/fap-hero-fantasy-play-both-versions-4k-60fps-1080p-links/125133">Original Script by quantoz</a><br>\n'
      + Object.entries(axes).map(([axis, id]) => `<a href="/uploads/short-url/${id}.funscript">Fap Hero - Fantasy Play - HARD MODE_ADV.${axis}.funscript</a>`).join('<br>\n')
      + '</p>';
    expect(parsePostLinks(html).attachments).toEqual(Object.entries(axes).map(([axis, id]) => ({
      name: `Fap Hero - Fantasy Play - HARD MODE_ADV.${axis}.funscript`,
      url: `${FORUM}/uploads/short-url/${id}.funscript`,
    })));
  });

  it('reads href wherever it sits among the attributes, and keeps class="attachment" links', () => {
    const html = [
      '<a class="attachment" href="/uploads/short-url/a.funscript">A.funscript</a>',
      '<a href="/uploads/short-url/b.funscript" class="attachment">B.funscript</a>',
      '<a target="_blank" rel="noopener" href=\'https://discuss.eroscripts.com/uploads/short-url/c.zip\'>C pack.zip</a>',
      '<a href="/uploads/short-url/d.funscript"><span>download</span></a>',
    ].join(' ');
    expect(parsePostLinks(html).attachments).toEqual([
      { name: 'A.funscript', url: `${FORUM}/uploads/short-url/a.funscript` },
      { name: 'B.funscript', url: `${FORUM}/uploads/short-url/b.funscript` },
      { name: 'C pack.zip', url: `${FORUM}/uploads/short-url/c.zip` },
      { name: 'd.funscript', url: `${FORUM}/uploads/short-url/d.funscript` },
    ]);
  });

  it('ignores images, image downloads and links that are not forum uploads', () => {
    const html = [
      '<a class="lightbox" data-download-href="/uploads/short-url/pTknwAaVpxrMatszfnF6jZSYeMQ.png?dl=1" '
        + 'href="https://eroscripts-discourse.eroscripts.com/original/4X/b/5/7/b577779a56cb1a37d362b21ee7b58e66997fb0ac.png" title="image">'
        + '<div class="lightbox-wrapper"><img src="x.png" alt="image"><span class="filename">image.png</span></div></a>',
      '<a href="/uploads/short-url/pTknwAaVpxrMatszfnF6jZSYeMQ.png?dl=1">Scene.funscript</a>',
      '<a href="https://discuss.eroscripts.com/t/scene/125133">Scene.funscript</a>',
      '<a href="https://evil.example/uploads/short-url/x.funscript">Scene.funscript</a>',
      '<a class="mention" href="/u/someone">@someone</a>',
    ].join('');
    expect(parsePostLinks(html).attachments).toEqual([]);
  });

  it('collapses the same file linked twice, including via the upload CDN', () => {
    const cdn = 'https://eroscripts-discourse.eroscripts.com/original/4X/b/5/7/b577779a56cb1a37d362b21ee7b58e66997fb0ac.funscript';
    const html = [
      '<a href="/uploads/short-url/pTknwAaVpxrMatszfnF6jZSYeMQ.funscript">Scene.funscript</a>',
      '<a class="attachment" href="/uploads/short-url/pTknwAaVpxrMatszfnF6jZSYeMQ.funscript?dl=1">Scene.funscript</a>',
      `<a href="${cdn}">Scene (mirror).funscript</a>`,
      '<a href="upload://pTknwAaVpxrMatszfnF6jZSYeMQ.funscript">Scene.funscript</a>',
    ].join('');
    expect(parsePostLinks(html).attachments).toEqual([
      { name: 'Scene.funscript', url: `${FORUM}/uploads/short-url/pTknwAaVpxrMatszfnF6jZSYeMQ.funscript` },
    ]);
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
            { id: 1, post_number: 1, username: 'author', cooked: att('/uploads/short-url/main.funscript', 'Scene.funscript') },
            { id: 2, post_number: 2, username: 'fan', cooked: att('/uploads/short-url/fan.funscript', 'Scene SR6.funscript') },
          ],
        },
      },
      '/t/7/posts.json?post_ids[]=3': {
        post_stream: { posts: [{ id: 3, post_number: 3, username: 'author', cooked: att('/uploads/short-url/v2.funscript', 'Scene v2.funscript') }] },
      },
      '/categories.json': CATEGORIES,
    });
    const r = await new EroScriptsAPI().getTopicAttachments(7);
    expect(r.attachments.map((a) => [a.name, a.username, a.postNumber, a.byAuthor])).toEqual([
      ['Scene.funscript', 'author', 1, true],
      ['Scene v2.funscript', 'author', 3, true],
      ['Scene SR6.funscript', 'fan', 2, false],
    ]);
    expect(r.attachments[0].url).toBe('https://discuss.eroscripts.com/uploads/short-url/main.funscript');
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
    expect(r).toEqual({ attachments: [], linked: [], external: [{ host: 'MEGA', url: 'https://mega.nz/file/q' }], paid: true, preview: null });
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
