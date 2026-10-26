/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run eroscripts:scout — which free posts qualify, how a post's scripts
// pair with its video files, and the Discovery playlist update
// (scripts/eroscripts-scout-lib.mjs). No forum, MEGA or pixeldrain.
import { describe, it, expect } from 'vitest';
import {
  screenTopic, hasExcludedText, scoreCandidate, scriptParts, matchScore, planScenes,
  videoPreference, creatorFolder, sceneStem, downloadQueue, addToDiscovery, coreName, DISCOVERY_NAME,
} from '../../scripts/eroscripts-scout-lib.mjs';

const topic = (tags, title = 'Some scene', id = 1) => ({ id, title, tags });
const files = (...names) => names.map((name) => ({ name, host: 'forum', url: `https://x/${encodeURIComponent(name)}` }));
const vids = (...names) => names.map((name) => ({ name, size: 1e8, host: 'mega' }));

describe('screenTopic', () => {
  it.each([
    ['loli tag', topic(['animation', 'loli'])],
    ['shota tag', topic(['hentai', 'shota'])],
    ['furry tag', topic(['animation', 'furry'])],
    ['anthro tag', topic(['vam', 'anthro'])],
    ['furry in the title', topic(['animation', 'pmv'], '[LKYU] SAVAGE - Furry PMV Contest')],
    ['futanari tag', topic(['hentai', 'futanari'])],
    ['hyphenated futa tag', topic(['animation', 'futa-on-female'])],
    ['futa in the title', topic(['3d'], '[Multi-axis] Futa Elf Riding')],
  ])('never accepts %s', (_, t) => {
    expect(screenTopic(t)).toEqual({ ok: false, reason: 'excluded' });
  });

  it('takes animated posts, including AI-generated video, but not live action', () => {
    expect(screenTopic(topic(['animation', 'ai-video'])).ok).toBe(true);
    expect(screenTopic(topic(['animation', 'real'])).reason).toBe('not-animated');
    expect(screenTopic(topic(['pov', 'blowjob'])).reason).toBe('not-animated');
  });

  it('takes a music post by a creator already in the library even without an animation tag', () => {
    const t = topic(['pmv', 'real'], '[Multi-axis] [Bastati] Yuria Boss Battle 1');
    expect(screenTopic(t, { creators: ['bastati'] })).toEqual({ ok: true, music: true, vr: false, artist: 'bastati' });
    expect(screenTopic(t, { creators: [] }).reason).toBe('not-animated');
  });

  it('skips posts the library already links or whose scene folder it has', () => {
    expect(screenTopic(topic(['animation'], 'X', 7), { knownTopics: new Set([7]) }).reason).toBe('known');
    expect(screenTopic(topic(['animation'], 'Fap Hero - Fantasy Play'), { sceneNames: ['fap hero fantasy play'] }).reason).toBe('in-library');
  });
});

describe('hasExcludedText', () => {
  it('matches whole words only', () => {
    expect(hasExcludedText('a lolicon edit')).toBe(true);
    expect(hasExcludedText('Furry/anthro pack')).toBe(true);
    expect(hasExcludedText('shotgun lollipop anthropology')).toBe(false);
    expect(hasExcludedText('Futaba Sakura (Persona 5)')).toBe(false);
  });
});

describe('scoreCandidate', () => {
  it('prefers creampie / riding posts, both more than one', () => {
    const base = { likes: 20 };
    const none = scoreCandidate(base);
    const one = scoreCandidate({ ...base, preferred: ['creampie'] });
    const both = scoreCandidate({ ...base, preferred: ['creampie', 'riding'] });
    expect(one).toBeGreaterThan(none);
    expect(both).toBeGreaterThan(one);
  });

  it('ranks VR above a preferred tag and caps likes', () => {
    expect(scoreCandidate({ vr: true })).toBeGreaterThan(scoreCandidate({ preferred: ['riding'] }));
    expect(scoreCandidate({ likes: 10_000 })).toBe(3);
  });
});

describe('scriptParts', () => {
  it('reads named and TCode axis suffixes', () => {
    expect(scriptParts('Scene.twist.funscript')).toEqual({ base: 'Scene', axis: 'twist' });
    expect(scriptParts('bobo.R0.funscript')).toEqual({ base: 'bobo', axis: 'twist' });
    expect(scriptParts('bobo.L1.funscript')).toEqual({ base: 'bobo', axis: 'surge' });
    expect(scriptParts('bobo.L0.funscript')).toEqual({ base: 'bobo', axis: '' });
  });

  it('keeps dots that are part of the name and drops a video extension in the base', () => {
    expect(scriptParts('D.va Doll.funscript')).toEqual({ base: 'D.va Doll', axis: '' });
    expect(scriptParts('5プレス_7止.mp4.funscript')).toEqual({ base: '5プレス_7止', axis: '' });
    expect(scriptParts('readme.txt')).toBeNull();
  });
});

describe('matchScore', () => {
  it('ignores encode words and handles any script', () => {
    expect(matchScore('ね○ねト○ワ - #ウチらしか勝たん', 'ね○ねト○ワ - #ウチらしか勝たん 60fps.mp4')).toBe(1);
    expect(matchScore('VaM - War Song', 'VaM - War Song VR 8K60FPS low-bitrate.mp4')).toBe(1);
  });

  it('never pairs different numbers', () => {
    expect(matchScore('LewdGazer Momiji - Shrine Maiden Duties Part 1', '[LewdGazer] Momiji - Shrine Maiden Duties Part 2.mp4')).toBe(0);
    expect(matchScore('7', '17.mp4')).toBe(0);
    expect(matchScore('3手淫_1会話', '13電気椅子_1警察署.mp4')).toBe(0);
  });

  it('does not pair scenes that only share the creator and a word', () => {
    expect(matchScore('[Sodeno19] Xianyun - Library', '[Sodeno19] Rosaria - Library.mp4')).toBe(0);
  });
});

describe('planScenes', () => {
  it('pairs each script set with its own video and leaves a set without one out', () => {
    const { scenes } = planScenes({
      scripts: files('Widowmaker Doll.funscript', 'Widowmaker Doll.twist.funscript', 'D.va Doll.funscript', 'D.va Doll.roll.funscript',
        '[Reinamation] - Overwatch Fuck Doll Compilation.funscript'),
      videos: vids('Widowmaker Doll.mp4', '[Reinamation] - Overwatch Fuck Doll Compilation.mp4'),
    });
    expect(scenes.map((s) => [s.base, s.video.name])).toEqual([
      ['Widowmaker Doll', 'Widowmaker Doll.mp4'],
      ['[Reinamation] - Overwatch Fuck Doll Compilation', '[Reinamation] - Overwatch Fuck Doll Compilation.mp4'],
    ]);
    expect(scenes[0].axes).toEqual(['twist']);
  });

  it('merges bracket variants of one script set and keeps the first file per axis', () => {
    const { scenes } = planScenes({
      scripts: [
        ...files('LKYU SAVAGE.funscript', 'LKYU SAVAGE.pitch.funscript'),
        ...files('[LKYU] SAVAGE.funscript', '[LKYU] SAVAGE.roll.funscript').map((f) => ({ ...f, host: 'pixeldrain' })),
      ],
      videos: vids('[LKYU] SAVAGE.mp4'),
    });
    expect(scenes).toHaveLength(1);
    expect(scenes[0].scripts.map((f) => [f.axis, f.host])).toEqual([['', 'forum'], ['pitch', 'forum'], ['roll', 'pixeldrain']]);
  });

  it('gives a lone script set the post video even when names differ', () => {
    const { scenes } = planScenes({ scripts: files('tZdPbDBEeG0lFCgInmWkbVAYE90.funscript'), videos: vids('[Zero3d] After class with Ochako.mp4') });
    expect(scenes.map((s) => s.video.name)).toEqual(['[Zero3d] After class with Ochako.mp4']);
    expect(scenes[0].axes).toEqual([]);
  });

  it('picks ≤ 8K, ≤ 60 fps, low bitrate for VR', () => {
    const { scenes } = planScenes({
      vr: true,
      scripts: files('VaM - Chocolate Cream.funscript', 'VaM - Chocolate Cream.pitch.funscript', 'VaM - Chocolate Cream HANDY.funscript'),
      videos: vids('VaM - Chocolate Cream FLAT 4K60FPS.mp4', 'VaM - Chocolate Cream VR 8K60FPS high-bitrate.mp4',
        'VaM - Chocolate Cream VR 8K60FPS low-bitrate.mp4', 'VaM - Chocolate Cream VR 8K120FPS.mp4'),
    });
    expect(scenes).toHaveLength(1);
    expect(scenes[0].video.name).toBe('VaM - Chocolate Cream VR 8K60FPS low-bitrate.mp4');
  });

  it('skips scenes whose video is already in the library', () => {
    const r = planScenes({
      scripts: files('Anjbgo - 01 Remake Hello venus First Edition.funscript'),
      videos: vids('Anjbgo - 01 Remake Hello venus First Edition_VR_75Mbps.mp4'),
      libraryVideos: [coreName('Anjbgo - 01 Remake Hello venus First Edition.mp4')],
    });
    expect(r).toEqual({ scenes: [], inLibrary: 1 });
  });
});

describe('videoPreference', () => {
  it('never prefers a preview, and keeps flat posts at ≤ 4K', () => {
    expect(videoPreference('scene 4K preview.mp4', 1, false)).toBeLessThan(videoPreference('scene 1080p.mp4', 1, false));
    expect(videoPreference('scene 4K.mp4', 1, false)).toBeGreaterThan(videoPreference('scene 8K.mp4', 1, false));
  });
});

describe('folders', () => {
  it('names the creator from the title, skipping labels, else the poster', () => {
    expect(creatorFolder('[Multi-axis] [Bastati] Yuria Boss Battle 1', 'shuny404')).toBe('Bastati');
    expect(creatorFolder('[VAM] Elf ride', 'manluo77')).toBe('manluo77');
    expect(creatorFolder('(Meliodaswrath) Oral Bliss 2', 'bekscript')).toBe('Meliodaswrath');
  });

  it('prefixes a scene name that says nothing on its own with the post title', () => {
    expect(sceneStem('7', "Cock Hero - King's Row Story Mode")).toBe("Cock Hero - King's Row Story Mode - 7");
    expect(sceneStem('Widowmaker Doll', 'Overwatch Package')).toBe('Widowmaker Doll');
  });
});

describe('downloadQueue', () => {
  it('puts every multi-axis post before single-axis ones and drops posts without scenes', () => {
    const c = (id, axisType, score, scenes = 1) => ({ id, axisType, score, scenes: Array(scenes).fill({}) });
    const q = downloadQueue([c(1, 'single', 9), c(2, 'multi', 1), c(3, 'multi', 5), c(4, 'single', 2), c(5, 'multi', 8, 0)]);
    expect(q.map((x) => x.id)).toEqual([3, 2, 1, 4]);
  });
});

describe('addToDiscovery', () => {
  it('adds the incoming source and playlist once, then only new videos', () => {
    let n = 0;
    const newId = () => `id${++n}`;
    const config = { settings: { library: { sources: [{ id: 's', name: 'Multi-Axis', path: '/GVP/Multi-Axis' }] } }, playlists: [] };
    const first = addToDiscovery(config, ['/GVP/_incoming/a/a.mp4'], { incomingDir: '/GVP/_incoming', newId, now: 1 });
    expect(first).toEqual({ sourceAdded: true, playlistCreated: true, added: ['/GVP/_incoming/a/a.mp4'] });
    const again = addToDiscovery(config, ['/GVP/_incoming/a/a.mp4', '/GVP/_incoming/b/b.mp4'], { incomingDir: '/GVP/_incoming/', newId });
    expect(again).toEqual({ sourceAdded: false, playlistCreated: false, added: ['/GVP/_incoming/b/b.mp4'] });
    expect(config.settings.library.sources.map((s) => s.name)).toEqual(['Multi-Axis', DISCOVERY_NAME]);
    expect(config.playlists).toEqual([{ id: 'id2', name: DISCOVERY_NAME, createdAt: 1, videoPaths: ['/GVP/_incoming/a/a.mp4', '/GVP/_incoming/b/b.mp4'] }]);
  });

  it('uses an existing Discovery playlist whatever its case', () => {
    const config = { settings: { library: { sources: [{ path: '/in' }] } }, playlists: [{ id: 'p', name: ' discovery ', videoPaths: [] }] };
    expect(addToDiscovery(config, ['/in/x.mp4'], { incomingDir: '/in', newId: () => 'new' }).playlistCreated).toBe(false);
    expect(config.playlists[0].videoPaths).toEqual(['/in/x.mp4']);
  });
});
