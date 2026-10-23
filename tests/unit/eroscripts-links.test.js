/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The links file written by `npm run eroscripts:sync` and its merge into
// `library.scriptTags` on every library scan. The CLI must never write
// config.json: the app owns it and may be running.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { mergeLinksIntoScriptTags, applyEroscriptsLinks, SYNC_SOURCE } from '../../renderer/js/eroscripts-links.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';

const { readLinksFile, writeLinksFile, linksFilePath, LINKS_FILE } = await import('../../electron/eroscripts-links.js');

const NOW = Date.parse('2026-10-22T12:00:00Z');
const link = (topicId, confidence = 'confirmed', extra = {}) => ({
  topicId, topicUrl: `https://discuss.eroscripts.com/t/x/${topicId}`, title: 'T', tags: ['osr2'], confidence, matchedAt: '2026-10-21T00:00:00.000Z', ...extra,
});

let tmp;
beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'es-links-')); });
afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

describe('mergeLinksIntoScriptTags', () => {
  it('adds confirmed and probable links as sync records, keeping the preview URL', () => {
    const r = mergeLinksIntoScriptTags({}, { '/a.mp4': link(1, 'confirmed', { previewUrl: 'https://p/1.jpg' }), '/b.mp4': link(2, 'probable') }, NOW);
    expect(r.added).toBe(2);
    expect(r.map['/a.mp4']).toEqual({
      tags: ['osr2'], topicId: 1, topicUrl: 'https://discuss.eroscripts.com/t/x/1', source: SYNC_SOURCE,
      savedAt: NOW, confidence: 'confirmed', matchedAt: '2026-10-21T00:00:00.000Z', previewUrl: 'https://p/1.jpg',
    });
    expect(r.map['/b.mp4'].confidence).toBe('probable');
  });

  it('never replaces a record the app wrote when the user downloaded from a post', () => {
    const mine = { tags: ['vr'], topicId: 9, topicUrl: 'u', source: 'eroscripts', savedAt: 1 };
    expect(mergeLinksIntoScriptTags({ '/a.mp4': mine }, { '/a.mp4': link(1) }, NOW)).toBe(null);
  });

  it('ignores anything that is not confirmed or probable', () => {
    expect(mergeLinksIntoScriptTags({}, { '/a.mp4': link(1, 'review') }, NOW)).toBe(null);
  });

  it('updates changed sync records, leaves identical ones and drops withdrawn ones', () => {
    const first = mergeLinksIntoScriptTags({}, { '/a.mp4': link(1), '/b.mp4': link(2) }, NOW).map;
    expect(mergeLinksIntoScriptTags(first, { '/a.mp4': link(1), '/b.mp4': link(2) }, NOW + 1)).toBe(null);
    const next = mergeLinksIntoScriptTags(first, { '/a.mp4': link(3) }, NOW + 1);
    expect(next).toMatchObject({ added: 0, updated: 1, removed: 1 });
    expect(next.map['/a.mp4'].topicId).toBe(3);
    expect(next.map['/b.mp4']).toBeUndefined();
  });
});

describe('applyEroscriptsLinks', () => {
  function settings(initial = {}) {
    const data = { ...initial };
    let sets = 0;
    return { get: (k) => data[k], set: (k, v) => { data[k] = v; sets++; }, data, get sets() { return sets; } };
  }

  it('reads the file through the main process and writes scriptTags once', async () => {
    const s = settings();
    const api = { readEroscriptsLinks: async () => ({ version: 1, links: { '/a.mp4': link(1) } }) };
    expect(await applyEroscriptsLinks(s, api, NOW)).toEqual({ added: 1, updated: 0, removed: 0 });
    expect(s.data[SCRIPT_TAGS_KEY]['/a.mp4'].topicId).toBe(1);
    expect(await applyEroscriptsLinks(s, api, NOW)).toBe(null);
    expect(s.sets).toBe(1);
  });

  it('does nothing without a links file or the IPC', async () => {
    const s = settings();
    expect(await applyEroscriptsLinks(s, { readEroscriptsLinks: async () => null }, NOW)).toBe(null);
    expect(await applyEroscriptsLinks(s, {}, NOW)).toBe(null);
    expect(s.sets).toBe(0);
  });
});

describe('links file', () => {
  it('reads a missing file as empty and round-trips atomically', () => {
    expect(readLinksFile(tmp)).toEqual({ version: 1, updatedAt: null, links: {}, review: {} });
    writeLinksFile(tmp, { links: { '/a.mp4': link(1) }, review: {} });
    expect(readLinksFile(tmp).links['/a.mp4'].topicId).toBe(1);
    expect(fs.readdirSync(tmp)).toEqual([LINKS_FILE]);
  });

  it('refuses a corrupt or foreign file instead of treating it as empty', () => {
    fs.writeFileSync(linksFilePath(tmp), '{nope');
    expect(readLinksFile(tmp)).toBe(null);
    fs.writeFileSync(linksFilePath(tmp), JSON.stringify({ version: 99, links: {} }));
    expect(readLinksFile(tmp)).toBe(null);
  });
});

describe('npm run eroscripts:sync never writes config.json', () => {
  const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
  const script = (len, seed) => {
    const actions = [];
    for (let at = 0, i = 0; at <= len; at += 500, i++) actions.push({ at, pos: (i * 29 * seed) % 101 });
    return JSON.stringify({ actions });
  };

  it('links a video end to end against a stubbed forum, touching only its own files', () => {
    const userData = path.join(tmp, 'userData');
    const sceneDir = path.join(tmp, 'Library', 'Fap Hero', 'Scene One');
    fs.mkdirSync(userData, { recursive: true });
    fs.mkdirSync(sceneDir, { recursive: true });
    fs.writeFileSync(path.join(sceneDir, 'Scene One.mp4'), '');
    fs.writeFileSync(path.join(sceneDir, 'Scene One.funscript'), script(60_000, 1));
    const config = JSON.stringify({ settings: {
      library: { sources: [{ path: path.join(tmp, 'Library'), enabled: true }], scriptTags: {}, customThumbnails: {} },
      eroscripts: { session: { cookie: 'COOKIE', username: 'me' } },
    } });
    const configPath = path.join(userData, 'config.json');
    fs.writeFileSync(configPath, config);
    const before = fs.statSync(configPath).mtimeMs;

    // fetch stub loaded with --import into the CLI process.
    const fixture = {
      'session/current.json': { current_user: { username: 'me' } },
      'categories.json': { category_list: { categories: [{ id: 7, slug: 'free-scripts' }] } },
      'search.json': { topics: [{ id: 42, title: 'Fap Hero Scene One', slug: 'scene-one', category_id: 7, tags: ['osr2'] }], posts: [{ topic_id: 42, username: 'author' }], users: [] },
      't/42.json': { category_id: 7, post_stream: { posts: [{ id: 1, post_number: 1, username: 'author', cooked: '<a class="attachment" href="/uploads/short-url/abc.funscript">Scene One.funscript</a>' }], stream: [1] } },
      'uploads/short-url/abc.funscript': script(60_000, 1),
    };
    const stub = path.join(tmp, 'fetch-stub.mjs');
    fs.writeFileSync(stub, `
      const fixture = ${JSON.stringify(fixture)};
      const cookies = [];
      globalThis.fetch = async (url, opts = {}) => {
        const u = new URL(url);
        if (u.hostname !== 'discuss.eroscripts.com') throw new Error('unexpected host ' + u.hostname);
        const key = Object.keys(fixture).find((k) => u.pathname === '/' + k);
        if (!key) return new Response('not found', { status: 404 });
        const body = fixture[key];
        return new Response(typeof body === 'string' ? body : JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } });
      };
    `);

    const r = spawnSync(process.execPath, [
      '--disable-warning=MODULE_TYPELESS_PACKAGE_JSON', '--import', stub,
      path.join(repo, 'scripts', 'eroscripts-sync.mjs'), '--user-data', userData,
    ], { encoding: 'utf8', cwd: repo });
    expect(r.stderr).toBe('');
    expect(r.status).toBe(0);
    expect(r.stdout).toContain('Linked: 1 (confirmed 1, probable 0)');

    expect(fs.readFileSync(configPath, 'utf8')).toBe(config);
    expect(fs.statSync(configPath).mtimeMs).toBe(before);
    expect(fs.readdirSync(userData).sort()).toEqual(['config.json', 'eroscripts-links.json', 'eroscripts-sync-report.md']);
    const links = readLinksFile(userData).links;
    expect(links[path.join(sceneDir, 'Scene One.mp4')]).toMatchObject({ topicId: 42, confidence: 'confirmed', basis: 'main-hash' });
    // Nothing new in the scene folder either.
    expect(fs.readdirSync(sceneDir).sort()).toEqual(['Scene One.funscript', 'Scene One.mp4']);
  });

  it('exits 3 with a clear message when not logged in', () => {
    const userData = path.join(tmp, 'ud');
    fs.mkdirSync(path.join(tmp, 'Library'), { recursive: true });
    fs.mkdirSync(userData);
    const config = JSON.stringify({ settings: { library: { sources: [{ path: path.join(tmp, 'Library') }] }, eroscripts: {} } });
    fs.writeFileSync(path.join(userData, 'config.json'), config);
    const r = spawnSync(process.execPath, ['--disable-warning=MODULE_TYPELESS_PACKAGE_JSON', path.join(repo, 'scripts', 'eroscripts-sync.mjs'), '--user-data', userData], { encoding: 'utf8', cwd: repo });
    expect(r.status).toBe(3);
    expect(r.stderr).toMatch(/Not logged in to EroScripts/);
    expect(fs.readdirSync(userData)).toEqual(['config.json']);
  });

  it('exits 3 when the forum says the session has expired, writing nothing', () => {
    const userData = path.join(tmp, 'ud2');
    fs.mkdirSync(path.join(tmp, 'Library'), { recursive: true });
    fs.mkdirSync(userData);
    const config = JSON.stringify({ settings: { library: { sources: [{ path: path.join(tmp, 'Library') }] }, eroscripts: { session: { cookie: 'OLD' } } } });
    fs.writeFileSync(path.join(userData, 'config.json'), config);
    const stub = path.join(tmp, 'forbidden.mjs');
    fs.writeFileSync(stub, "globalThis.fetch = async () => new Response('', { status: 403 });");
    const r = spawnSync(process.execPath, ['--disable-warning=MODULE_TYPELESS_PACKAGE_JSON', '--import', stub, path.join(repo, 'scripts', 'eroscripts-sync.mjs'), '--user-data', userData], { encoding: 'utf8', cwd: repo });
    expect(r.status).toBe(3);
    expect(r.stderr).toMatch(/session has expired/);
    expect(fs.readFileSync(path.join(userData, 'config.json'), 'utf8')).toBe(config);
    expect(fs.readdirSync(userData)).toEqual(['config.json']);
  });
});
