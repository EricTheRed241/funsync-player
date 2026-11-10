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
import { mergeVideoTags, applyVideoTags, combineTagFiles, VIDEO_TAGS_SOURCE } from '../../renderer/js/video-tags.js';
import { mergeLinksIntoScriptTags, mergeScoutPosts, SCOUT_SOURCE, SYNC_SOURCE, FAPTAP_SOURCE } from '../../renderer/js/eroscripts-links.js';
import { SCRIPT_TAGS_KEY, baseTags } from '../../renderer/js/script-tags.js';
import { VIDEO_TAGGER_TAGS } from '../../renderer/js/content-preferences.js';
import { remapPaths } from '../../scripts/eroscripts-scout-lib.mjs';

const {
  readVideoTagsFile, rewriteVideoTagsFile, videoTagsFilePath, readPornstarTagsFile, pornstarTagsFilePath,
} = await import('../../electron/video-tags.js');

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

  it('moves keys after a rename, keeping the previous file as .bak; leaves a missing or unchanged file alone', () => {
    const moves = [{ from: '/in/a/3', to: '/in/a/A - B', files: [{ from: '3.mp4', to: 'A - B.mp4' }] }];
    const update = (data) => remapPaths(data, moves);
    expect(rewriteVideoTagsFile(tmp, update)).toBe(0);
    expect(fs.readdirSync(tmp)).toEqual([]);

    const before = `${JSON.stringify(file({ '/in/a/3/3.mp4': { tags: ['pov'], addedAt: 'T' } }), null, 2)}\n`;
    fs.writeFileSync(videoTagsFilePath(tmp), before);
    expect(rewriteVideoTagsFile(tmp, update)).toBe(1);
    expect(Object.keys(readVideoTagsFile(tmp).videos)).toEqual(['/in/a/A - B/A - B.mp4']);
    expect(fs.readFileSync(`${videoTagsFilePath(tmp)}.bak`, 'utf8')).toBe(before);

    const after = fs.readFileSync(videoTagsFilePath(tmp), 'utf8');
    expect(rewriteVideoTagsFile(tmp, update)).toBe(0);
    expect(fs.readFileSync(videoTagsFilePath(tmp), 'utf8')).toBe(after);
    expect(fs.readdirSync(tmp).sort()).toEqual(['video-tags.json', 'video-tags.json.bak']);
  });
});

// The pornstar tags file (`pornstar-tags.json`, written by the
// `pornstar-tags` Rust CLI), merged together with the video tags file.
describe('pornstar tags', () => {
  const pornstarFile = (pornstars, videos) => ({ version: 1, updatedAt: 'T', minScenes: 2, pornstars, videos });
  const star = (name, scenes) => ({ name, scenes, applied: scenes >= 2 });
  const STARS = { 'candy-love': star('Candy Love', 3), 'riley-reid': star('Riley Reid', 1) };
  /** One scan's merge, as applyVideoTags does it. */
  const scan = (map, videoFile, psFile, now) => {
    const combined = combineTagFiles(map, videoFile, psFile, ALLOWED);
    return combined && mergeVideoTags(map, combined.file, combined.allowed, now);
  };

  it('unions both files per video and allows only applied pornstars on top of the tagger tags', () => {
    const combined = combineTagFiles({}, file({ '/a.mp4': { tags: ['pov'] } }), pornstarFile(STARS, {
      '/a.mp4': ['candy-love'], '/b.mp4': ['candy-love', 'riley-reid'],
    }), ALLOWED);
    expect(combined.allowed).toEqual([...ALLOWED, 'candy-love']);
    expect(combined.file.videos).toEqual({ '/a.mp4': { tags: ['pov', 'candy-love'] }, '/b.mp4': { tags: ['candy-love', 'riley-reid'] } });

    const r = mergeVideoTags({ '/a.mp4': scoutRecord(1, ['osr2']) }, combined.file, combined.allowed, NOW);
    expect(r.map['/a.mp4']).toMatchObject({ tags: ['osr2', 'pov', 'candy-love'], addedTags: ['pov', 'candy-love'], source: SCOUT_SOURCE });
    expect(r.map['/b.mp4']).toMatchObject({ tags: ['candy-love'], addedTags: ['candy-love'], source: VIDEO_TAGS_SOURCE });
  });

  it('removes a pornstar tag dropped from the file on the next scan, never a post tag', () => {
    const videoFile = file({ '/a.mp4': { tags: ['pov'] } });
    const first = scan({
      '/a.mp4': scoutRecord(1, ['osr2']),
      '/p.mp4': scoutRecord(2, ['candy-love']),
    }, videoFile, pornstarFile(STARS, { '/a.mp4': ['candy-love'], '/b.mp4': ['candy-love'], '/p.mp4': ['candy-love'] }), NOW);
    expect(scan(first.map, videoFile, pornstarFile(STARS, { '/a.mp4': ['candy-love'], '/b.mp4': ['candy-love'], '/p.mp4': ['candy-love'] }), LATER)).toBe(null);

    const second = scan(first.map, videoFile, pornstarFile(STARS, { '/p.mp4': ['candy-love'] }), LATER);
    expect(second).toMatchObject({ updated: 1, removed: 1 });
    expect(second.map['/a.mp4']).toMatchObject({ tags: ['osr2', 'pov'], addedTags: ['pov'] });
    expect(second.map['/b.mp4']).toBeUndefined();
    expect(second.map['/p.mp4'].tags).toEqual(['candy-love']);
    expect(second.map['/p.mp4']).not.toHaveProperty('addedTags');

    // A performer no longer applied: her tag goes even where the file still lists it.
    const third = scan(first.map, videoFile, pornstarFile({ 'candy-love': star('Candy Love', 1) }, { '/a.mp4': ['candy-love'] }), LATER);
    expect(third.map['/a.mp4'].tags).toEqual(['osr2', 'pov']);
    expect(third.map['/b.mp4']).toBeUndefined();
  });

  it('a missing pornstar file withdraws only the pornstar tags; a missing video tags file keeps the video tags', () => {
    const videoFile = file({ '/a.mp4': { tags: ['pov'] } });
    const psFile = pornstarFile(STARS, { '/a.mp4': ['candy-love'], '/b.mp4': ['candy-love'] });
    const first = scan({}, videoFile, psFile, NOW);
    expect(first.map['/a.mp4'].tags).toEqual(['pov', 'candy-love']);

    const noStars = scan(first.map, videoFile, null, LATER);
    expect(noStars.map['/a.mp4']).toMatchObject({ tags: ['pov'], addedTags: ['pov'] });
    expect(noStars.map['/b.mp4']).toBeUndefined();

    expect(scan(first.map, null, psFile, LATER)).toBe(null);
    const noVideoFile = scan(first.map, null, pornstarFile(STARS, {}), LATER);
    expect(noVideoFile.map['/a.mp4']).toMatchObject({ tags: ['pov'], addedTags: ['pov'] });
    expect(noVideoFile.map['/b.mp4']).toBeUndefined();
    expect(combineTagFiles(first.map, null, null, ALLOWED)).toBe(null);
  });

  it('applyVideoTags reads both files', async () => {
    const data = {};
    const s = { get: (k) => data[k], set: (k, v) => { data[k] = v; } };
    const api = {
      readVideoTags: async () => null,
      readPornstarTags: async () => pornstarFile(STARS, { '/a.mp4': ['candy-love'] }),
    };
    expect(await applyVideoTags(s, api, NOW)).toEqual({ added: 1, updated: 0, removed: 0 });
    expect(data[SCRIPT_TAGS_KEY]['/a.mp4'].tags).toEqual(['candy-love']);
    expect(await applyVideoTags(s, api, LATER)).toBe(null);
  });

  describe('readPornstarTagsFile', () => {
    let tmp;
    beforeEach(() => { tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'pornstar-tags-')); });
    afterEach(() => { fs.rmSync(tmp, { recursive: true, force: true }); });

    it('reads a version 1 file, and returns null for a missing, corrupt or foreign one', () => {
      expect(readPornstarTagsFile(tmp)).toBe(null);
      const good = pornstarFile(STARS, { '/a.mp4': ['candy-love'] });
      fs.writeFileSync(pornstarTagsFilePath(tmp), JSON.stringify(good));
      expect(readPornstarTagsFile(tmp)).toEqual({ version: 1, updatedAt: 'T', pornstars: STARS, videos: good.videos });
      for (const bad of ['{nope', JSON.stringify({ ...good, version: 2 }), JSON.stringify({ ...good, pornstars: [] }), JSON.stringify({ version: 1, pornstars: {} })]) {
        fs.writeFileSync(pornstarTagsFilePath(tmp), bad);
        expect(readPornstarTagsFile(tmp)).toBe(null);
      }
    });
  });
});
