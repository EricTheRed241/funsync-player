/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The video tags file (`video-tags.json`, written by the `video-tags` Rust
// CLI) and its merge into `library.scriptTags` after every scan, on top of
// the post tags the sidecar and link merges keep rebuilding.
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { mergeVideoTags, applyVideoTags, VIDEO_TAGS_SOURCE } from '../../renderer/js/video-tags.js';
import { mergeLinksIntoScriptTags, mergeScoutPosts, SCOUT_SOURCE, SYNC_SOURCE, FAPTAP_SOURCE } from '../../renderer/js/eroscripts-links.js';
import { SCRIPT_TAGS_KEY, baseTags } from '../../renderer/js/script-tags.js';
import { VIDEO_TAGGER_TAGS } from '../../renderer/js/content-preferences.js';

const { readVideoTagsFile, videoTagsFilePath } = await import('../../electron/video-tags.js');

const NOW = Date.parse('2026-11-04T12:00:00Z');
const LATER = NOW + 60_000;
const ALLOWED = ['vr', 'pov', 'creampie', 'riding'];
const file = (videos) => ({ version: 1, updatedAt: '2026-11-04T12:00:00Z', videos });
const scoutRecord = (topicId, tags) => ({ tags, topicId, topicUrl: `https://discuss.eroscripts.com/t/x/${topicId}`, source: SCOUT_SOURCE, savedAt: NOW });

describe('mergeVideoTags', () => {
  it('creates a video-tags record for a video without one', () => {
    const r = mergeVideoTags({}, file({ '/Other/a.mp4': { tags: ['pov', 'creampie'] } }), ALLOWED, NOW);
    expect(r).toMatchObject({ added: 1, updated: 0, removed: 0 });
    expect(r.map['/Other/a.mp4']).toEqual({
      tags: ['pov', 'creampie'], addedTags: ['pov', 'creampie'], topicId: null, topicUrl: null, source: VIDEO_TAGS_SOURCE, savedAt: NOW,
    });
  });

  it('adds the tags a post record lacks after its own, remembering them in addedTags', () => {
    const current = { '/a.mp4': scoutRecord(1, ['osr2', 'creampie']) };
    const r = mergeVideoTags(current, file({ '/a.mp4': { tags: ['creampie', 'pov'] } }), ALLOWED, LATER);
    expect(r).toMatchObject({ added: 0, updated: 1, removed: 0 });
    expect(r.map['/a.mp4']).toEqual({ ...scoutRecord(1, ['osr2', 'creampie', 'pov']), addedTags: ['pov'] });
    expect(baseTags(r.map['/a.mp4'])).toEqual(['osr2', 'creampie']);
    // The input map is left alone.
    expect(current['/a.mp4'].tags).toEqual(['osr2', 'creampie']);
  });

  it('returns null on a second merge of the same file', () => {
    const f = file({ '/a.mp4': { tags: ['pov'] }, '/b.mp4': { tags: ['vr'] } });
    const first = mergeVideoTags({ '/a.mp4': scoutRecord(1, ['osr2']) }, f, ALLOWED, NOW);
    expect(mergeVideoTags(first.map, f, ALLOWED, LATER)).toBe(null);
  });

  it('adds nothing when the post already has every tag', () => {
    expect(mergeVideoTags({ '/a.mp4': scoutRecord(1, ['pov']) }, file({ '/a.mp4': { tags: ['pov'] } }), ALLOWED, NOW)).toBe(null);
  });

  it('strips only the added tags when an entry leaves the file, and deletes a video-tags record left empty', () => {
    const f = file({ '/a.mp4': { tags: ['pov', 'riding'] }, '/b.mp4': { tags: ['vr'] } });
    const first = mergeVideoTags({ '/a.mp4': scoutRecord(1, ['osr2', 'riding']) }, f, ALLOWED, NOW);
    const r = mergeVideoTags(first.map, file({}), ALLOWED, LATER);
    expect(r).toMatchObject({ added: 0, updated: 1, removed: 1 });
    expect(r.map['/a.mp4']).toEqual(scoutRecord(1, ['osr2', 'riding']));
    expect(r.map).not.toHaveProperty('/b.mp4');
  });

  it('applies only allowed tags, lower-cased and once each', () => {
    const r = mergeVideoTags({}, file({ '/a.mp4': { tags: ['POV', 'pov', 'blowjob', 'Riding', 7] }, '/b.mp4': { tags: ['blowjob'] } }), ALLOWED, NOW);
    expect(r.map['/a.mp4'].tags).toEqual(['pov', 'riding']);
    expect(r.map).not.toHaveProperty('/b.mp4');
    expect(r.added).toBe(1);
  });

  it('withdraws an added tag that is no longer allowed', () => {
    const first = mergeVideoTags({}, file({ '/a.mp4': { tags: ['pov', 'vr'] } }), ALLOWED, NOW);
    const r = mergeVideoTags(first.map, file({ '/a.mp4': { tags: ['pov', 'vr'] } }), ['pov'], LATER);
    expect(r.map['/a.mp4']).toMatchObject({ tags: ['pov'], addedTags: ['pov'] });
  });

  it('changes nothing when the file could not be read', () => {
    const first = mergeVideoTags({}, file({ '/a.mp4': { tags: ['pov'] } }), ALLOWED, NOW);
    expect(mergeVideoTags(first.map, null, ALLOWED, LATER)).toBe(null);
    expect(mergeVideoTags(first.map, undefined, ALLOWED, LATER)).toBe(null);
  });

  it('allows exactly the video tagger vocabulary', () => {
    expect(VIDEO_TAGGER_TAGS).toEqual(ALLOWED);
  });
});

describe('the scan merges leave added tags alone', () => {
  it('mergeScoutPosts does not rewrite a record whose only difference is its added tags', () => {
    const tagged = mergeVideoTags({ '/a.mp4': scoutRecord(1, ['osr2']) }, file({ '/a.mp4': { tags: ['pov'] } }), ALLOWED, NOW).map;
    const videos = [{ path: '/a.mp4', eroscripts: { topicId: 1, url: 'https://discuss.eroscripts.com/t/x/1', tags: ['osr2'] } }];
    expect(mergeScoutPosts(tagged, videos, LATER)).toBe(null);
  });

  it('mergeScoutPosts refreshes a FapTap record by its own tags only', () => {
    const faptap = { tags: ['pmv'], topicId: null, topicUrl: null, faptapId: 'f1', faptapUrl: 'https://faptap.net/v/f1', source: FAPTAP_SOURCE, savedAt: NOW };
    const tagged = mergeVideoTags({ '/f.mp4': faptap }, file({ '/f.mp4': { tags: ['vr'] } }), ALLOWED, NOW).map;
    const videos = [{ path: '/f.mp4', faptap: { faptapId: 'f1', url: 'https://faptap.net/v/f1', tags: ['pmv'] } }];
    expect(mergeScoutPosts(tagged, videos, LATER)).toBe(null);
  });

  it('mergeScoutPosts replaces a video-tags gap record with the sidecar post or FapTap scene, without added tags', () => {
    const gaps = mergeVideoTags({}, file({ '/a.mp4': { tags: ['pov'] }, '/f.mp4': { tags: ['vr'] } }), ALLOWED, NOW).map;
    const r = mergeScoutPosts(gaps, [
      { path: '/a.mp4', eroscripts: { topicId: 1, url: null, tags: ['osr2'] } },
      { path: '/f.mp4', faptap: { faptapId: 'f1', url: null, tags: ['pmv'] } },
    ], LATER);
    expect(r.updated).toBe(2);
    expect(r.map['/a.mp4']).toEqual({ tags: ['osr2'], topicId: 1, topicUrl: null, source: SCOUT_SOURCE, savedAt: LATER });
    expect(r.map['/f.mp4']).toMatchObject({ tags: ['pmv'], source: FAPTAP_SOURCE });
    expect(r.map['/f.mp4']).not.toHaveProperty('addedTags');
  });

  const link = (topicId, tags) => ({
    topicId, topicUrl: `https://discuss.eroscripts.com/t/x/${topicId}`, tags, confidence: 'confirmed', matchedAt: '2026-11-01T00:00:00.000Z',
  });

  it('mergeLinksIntoScriptTags does not rewrite a record whose only difference is its added tags', () => {
    const synced = mergeLinksIntoScriptTags({}, { '/a.mp4': link(2, ['osr2']) }, NOW).map;
    const tagged = mergeVideoTags(synced, file({ '/a.mp4': { tags: ['riding'] } }), ALLOWED, NOW).map;
    expect(tagged['/a.mp4'].tags).toEqual(['osr2', 'riding']);
    expect(mergeLinksIntoScriptTags(tagged, { '/a.mp4': link(2, ['osr2']) }, LATER)).toBe(null);
  });

  it('mergeLinksIntoScriptTags replaces a video-tags gap record with the linked post', () => {
    const gaps = mergeVideoTags({}, file({ '/a.mp4': { tags: ['pov'] } }), ALLOWED, NOW).map;
    const r = mergeLinksIntoScriptTags(gaps, { '/a.mp4': link(2, ['osr2']) }, LATER);
    expect(r.updated).toBe(1);
    expect(r.map['/a.mp4']).toMatchObject({ tags: ['osr2'], topicId: 2, source: SYNC_SOURCE });
    expect(r.map['/a.mp4']).not.toHaveProperty('addedTags');
  });

  it('a whole scan (scout → links → video tags) ends with the added tags, and a second scan changes nothing', () => {
    const videos = [
      { path: '/E/a.mp4', eroscripts: { topicId: 1, url: null, tags: ['osr2', 'creampie'] } },
      { path: '/E/b.mp4' },
      { path: '/O/c.mp4' },
    ];
    const links = { '/E/b.mp4': link(2, ['sr6']) };
    const tags = file({ '/E/a.mp4': { tags: ['pov', 'creampie'] }, '/E/b.mp4': { tags: ['vr'] }, '/O/c.mp4': { tags: ['riding'] } });
    const scan = (map, now) => {
      const scout = mergeScoutPosts(map, videos, now);
      const linked = mergeLinksIntoScriptTags(scout?.map || map, links, now);
      const tagged = mergeVideoTags(linked?.map || scout?.map || map, tags, ALLOWED, now);
      return { scout, linked, tagged, map: tagged?.map || linked?.map || scout?.map || map };
    };
    const first = scan({}, NOW);
    expect(first.map['/E/a.mp4'].tags).toEqual(['osr2', 'creampie', 'pov']);
    expect(first.map['/E/b.mp4'].tags).toEqual(['sr6', 'vr']);
    expect(first.map['/O/c.mp4']).toMatchObject({ tags: ['riding'], source: VIDEO_TAGS_SOURCE });
    const second = scan(first.map, LATER);
    expect([second.scout, second.linked, second.tagged]).toEqual([null, null, null]);
  });
});

describe('applyVideoTags', () => {
  function settings(initial = {}) {
    const data = { ...initial };
    let sets = 0;
    return { data, get: (k) => data[k], set: (k, v) => { data[k] = v; sets++; }, get sets() { return sets; } };
  }

  it('writes the settings once and only on a change', async () => {
    const s = settings();
    const api = { readVideoTags: async () => file({ '/a.mp4': { tags: ['pov'] } }) };
    expect(await applyVideoTags(s, api, NOW)).toEqual({ added: 1, updated: 0, removed: 0 });
    expect(s.data[SCRIPT_TAGS_KEY]['/a.mp4'].tags).toEqual(['pov']);
    expect(await applyVideoTags(s, api, LATER)).toBe(null);
    expect(s.sets).toBe(1);
  });

  it('does nothing without a readable file or the IPC', async () => {
    const s = settings({ [SCRIPT_TAGS_KEY]: { '/a.mp4': { tags: ['pov'], addedTags: ['pov'], topicId: null, topicUrl: null, source: VIDEO_TAGS_SOURCE, savedAt: NOW } } });
    expect(await applyVideoTags(s, { readVideoTags: async () => null }, NOW)).toBe(null);
    expect(await applyVideoTags(s, {}, NOW)).toBe(null);
    expect(s.sets).toBe(0);
  });
});

describe('readVideoTagsFile', () => {
  let tmp;
  beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'video-tags-')); });
  afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

  it('reads a version 1 file', () => {
    fs.writeFileSync(videoTagsFilePath(tmp), JSON.stringify(file({ '/a.mp4': { tags: ['pov'], addedAt: '2026-11-04T12:00:00Z' } })));
    expect(readVideoTagsFile(tmp)).toEqual(file({ '/a.mp4': { tags: ['pov'], addedAt: '2026-11-04T12:00:00Z' } }));
  });

  it('returns null for a missing, corrupt or foreign file', () => {
    expect(readVideoTagsFile(tmp)).toBe(null);
    fs.writeFileSync(videoTagsFilePath(tmp), '{nope');
    expect(readVideoTagsFile(tmp)).toBe(null);
    fs.writeFileSync(videoTagsFilePath(tmp), JSON.stringify({ version: 2, videos: {} }));
    expect(readVideoTagsFile(tmp)).toBe(null);
    fs.writeFileSync(videoTagsFilePath(tmp), JSON.stringify({ version: 1, videos: [] }));
    expect(readVideoTagsFile(tmp)).toBe(null);
  });
});
