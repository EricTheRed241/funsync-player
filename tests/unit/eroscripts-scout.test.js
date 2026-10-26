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
  videoPreference, creatorFolder, sceneTitle, retitlePlan, remapPaths, downloadQueue, addToDiscovery, coreName, DISCOVERY_NAME,
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
  it('names the creator from the title, skipping labels, then a trailing "by", else the poster', () => {
    expect(creatorFolder('[Multi-axis] [Bastati] Yuria Boss Battle 1', 'shuny404')).toBe('Bastati');
    expect(creatorFolder('[VAM] Elf ride', 'manluo77')).toBe('manluo77');
    expect(creatorFolder('[Fap Hero] (poyopoyo) Tougen Renka', 'voloi')).toBe('poyopoyo');
    expect(creatorFolder('(Meliodaswrath) Oral Bliss 2', 'bekscript')).toBe('Meliodaswrath');
    expect(creatorFolder('Cock Hero: Girls of Overwatch 1 and 2 by Duodimension', 'FiniteJest')).toBe('Duodimension');
  });
});

describe('sceneTitle', () => {
  const title = (t, author, video = 'x.mp4', shared = false) => sceneTitle({ title: t, author, video, shared });

  it('is "Artist - Title (metadata)" from the post, without script labels', () => {
    expect(title('[Multi-axis][VAM] Linger 2', 'manluo77', '3.mp4')).toBe('manluo77 - Linger 2 (VAM)');
    expect(title('(Meliodaswrath) Oral Bliss 2 (Commissioned, Multi Axis Script)', 'bekscript')).toBe('Meliodaswrath - Oral Bliss 2');
    expect(title('[Oniko] 【NIKKE#7】Goddess Of Booty', 'Phel')).toBe('Oniko - Goddess Of Booty (NIKKE#7)');
    expect(title('[VR Multi-Axis] Hush LapDanceSEX (DLSS NR ON!)', 'laozhao')).toBe('laozhao - Hush LapDanceSEX (VR, DLSS NR ON!)');
    expect(title('HALLOWEEN SPECIAL (FREE Bonus Round) by FapZone3D', '0721sikosiko')).toBe('FapZone3D - HALLOWEEN SPECIAL (Bonus Round)');
    expect(title('Axis Master [SR6] Blonde beauty | ✦ 3-Axis Organic Movement', '1036476631')).toBe('1036476631 - Blonde beauty');
    expect(title('（poyopoyo）すきっちゅーの！ / Sukichuuno! | Linnea（multi-axis）', 'voloi')).toBe('poyopoyo - すきっちゅーの！ - Sukichuuno! (Linnea)');
  });

  it('names each video of a several-video post by its own file, keeping words like (G)I-DLE whole', () => {
    expect(title('[Seraphine] Overwatch Fuck Doll Package', 'Solace2K', '[Reinamation] - Overwatch Fuck Doll Compilation.mp4', true))
      .toBe('Seraphine - Overwatch Fuck Doll Compilation (Reinamation)');
    expect(title('(Multi-axis) Iwara MMD dance pack', 'loldorf', 'Iwara - 竖屏版---弱音haku----(G)I-DLE - Queencard [E1kz8fXZmnWTV6] [Source].mp4', true))
      .toBe('loldorf - 竖屏版 - 弱音haku - (G)I-DLE - Queencard');
    expect(title('VaM POV PMV - School Fantasy', 'Misiv', 'VaM POV PMV by Misiv - School Fantasy.mp4', true)).toBe('Misiv - VaM POV PMV - School Fantasy');
  });

  it('keeps the post title when a video name says nothing, and puts that name in the metadata', () => {
    expect(title("Cock Hero - Overwatch King's Row Story Mode Part 1 UPDATED", 'FruitPunch96', '7.mp4', true))
      .toBe("FruitPunch96 - Cock Hero - Overwatch King's Row Story Mode Part 1 (7)");
  });

  it('keeps a VR projection suffix at the end of the name', () => {
    expect(title('Yvonne from Arknights VAM 8KVR', 'maechoon', 'Yvonne - She Ridin On Me Passthrough_180_3DH.mp4', true))
      .toBe('maechoon - Yvonne - She Ridin On Me Passthrough_180_3DH');
    expect(title('Some dance', 'a', 'dance_LRF_Full_SBS.mp4')).toBe('a - Some dance_LRF_Full_SBS');
    expect(title('Some dance', 'a', 'dance Full.mp4')).toBe('a - Some dance');
  });

  it('stays within the file name limit, dropping metadata before cutting the title', () => {
    const long = title(`[Artist] ${'長い'.repeat(40)} (Extra)`, 'p');
    expect(Buffer.byteLength(long)).toBeLessThanOrEqual(180);
    expect(long).not.toMatch(/\(/);
  });
});

describe('retitlePlan', () => {
  const scene = (dir, video, rec, extra = []) => ({ dir, files: [video, `${video.replace(/\.mp4$/, '')}.twist.funscript`, 'eroscripts.json', ...extra], rec: { video, ...rec } });

  it('moves each scene to <artist>/<title>, renaming every file that carries the stem', () => {
    const moves = retitlePlan([scene('/in/manluo77/3', '3.mp4', { topicId: 1, title: '[Multi-axis][VAM] Linger 2', author: 'manluo77' }, ['3.en.srt', 'cover.jpg'])], '/in');
    expect(moves).toEqual([{
      from: '/in/manluo77/3', to: '/in/manluo77/manluo77 - Linger 2 (VAM)', video: 'manluo77 - Linger 2 (VAM).mp4',
      files: [
        { from: '3.mp4', to: 'manluo77 - Linger 2 (VAM).mp4' },
        { from: '3.twist.funscript', to: 'manluo77 - Linger 2 (VAM).twist.funscript' },
        { from: '3.en.srt', to: 'manluo77 - Linger 2 (VAM).en.srt' },
      ],
    }]);
  });

  it('counts a post with several fetched videos as shared, and numbers clashing names', () => {
    const rec = { topicId: 2, title: 'Pack', author: 'z' };
    const moves = retitlePlan([scene('/in/z/a', 'Dance.mp4', rec), scene('/in/z/b', 'dance.mp4', rec)], '/in');
    expect(moves.map((m) => m.to)).toEqual(['/in/z/z - Dance', '/in/z/z - dance (2)']);
  });

  it('plans nothing for scenes already named right', () => {
    const s = scene('/in/manluo77/manluo77 - Linger 2 (VAM)', 'manluo77 - Linger 2 (VAM).mp4', { topicId: 1, title: '[VAM] Linger 2', author: 'manluo77', video: '3.mp4' });
    expect(retitlePlan([s], '/in')).toEqual([]);
  });

  it('steps around a folder already on disk under the new name', () => {
    const s = scene('/in/manluo77/3', '3.mp4', { topicId: 1, title: '[VAM] Linger 2', author: 'manluo77' });
    const exists = (p) => p === '/in/manluo77/manluo77 - Linger 2 (VAM)';
    expect(retitlePlan([s], '/in', exists)[0].to).toBe('/in/manluo77/manluo77 - Linger 2 (VAM) (2)');
  });
});

describe('remapPaths', () => {
  it('moves renamed files and anything else under a moved folder, in keys and values', () => {
    const moves = [{ from: '/in/a/3', to: '/in/a/A - B', files: [{ from: '3.mp4', to: 'A - B.mp4' }] }];
    const config = {
      playlists: [{ videoPaths: ['/in/a/3/3.mp4', '/in/a/30/30.mp4'] }],
      associations: { '/in/a/3/3.mp4': { main: '/in/a/3/3.funscript' } },
    };
    const { value, changed } = remapPaths(config, moves);
    expect(value).toEqual({
      playlists: [{ videoPaths: ['/in/a/A - B/A - B.mp4', '/in/a/30/30.mp4'] }],
      associations: { '/in/a/A - B/A - B.mp4': { main: '/in/a/A - B/3.funscript' } },
    });
    expect(changed).toBe(3);
  });

  it('matches a path spelled in another case or Unicode form', () => {
    const moves = [{ from: '/in/bubukuin/Mona パ', to: '/in/bubukuin/Bubukuin - Mona', files: [{ from: 'Mona パ.mp4', to: 'Bubukuin - Mona.mp4' }] }];
    const old = '/in/Bubukuin/Mona パ'.normalize('NFD');
    const sub = 'パ.en.srt'.normalize('NFD');
    expect(remapPaths([`${old}/Mona ${'パ'.normalize('NFD')}.mp4`, `${old}/${sub}`], moves).value)
      .toEqual(['/in/bubukuin/Bubukuin - Mona/Bubukuin - Mona.mp4', `/in/bubukuin/Bubukuin - Mona/${sub}`]);
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
