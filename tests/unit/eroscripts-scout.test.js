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
  screenTopic, hasExcludedText, isSexual, scriptDurationMs, wantedScenes, scoreCandidate, scriptParts, matchScore, planScenes,
  videoPreference, creatorFolder, sceneTitle, retitlePlan, remapPaths, downloadQueue, addToDiscovery, coreName, DISCOVERY_NAME,
  emptyScoutState, normalizeScoutState, seenTopic, markTopic, listFeed, mergeCandidates, RULES_VERSION, needsJudging, pendingTopics, screenPick,
  contentRuleBreak, isHentaiOnly, sexEvidence, judgeFirstPost, needsSexRecheck, recheckWithText, lessPreferred, candidateScore, isBlockedCreator, hasBlockedTag,
} from '../../scripts/eroscripts-scout-lib.mjs';
import { MIN_VIDEO_MS, RANKING } from '../../renderer/js/content-preferences.js';
import { DEFAULT_TAG_SCORES as D } from '../../renderer/js/preference-score.js';

const topic = (tags, title = 'Some riding scene', id = 1) => ({ id, title, tags });
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

  it('takes monster and feet posts, favourites too; they only rank lower', () => {
    expect(screenTopic(topic(['sfm', 'riding'], 'Solace2K - Mercy X Horseguy'), { favourites: ['Solace2K'] })).toMatchObject({ ok: true, favourite: 'Solace2K' });
    expect(screenTopic(topic(['animation', 'riding', 'footjob']))).toMatchObject({ ok: true });
    expect(screenTopic(topic(['sfm', 'sex'], '触手 セックス'))).toMatchObject({ ok: true });
  });

  it('takes animated posts, including AI-generated video; live action only waits for the live quota', () => {
    expect(screenTopic(topic(['animation', 'ai-video']))).toMatchObject({ ok: true, live: false, liveQuota: false });
    expect(screenTopic(topic(['animation', 'real']))).toMatchObject({ ok: true, live: true, liveQuota: true });
    expect(screenTopic(topic(['pov', 'blowjob', 'doggy']))).toMatchObject({ ok: true, live: true, liveQuota: true });
  });

  it('takes live action by a favourite, named in the title or a tag, without the quota', () => {
    const favourites = ['Candy Love', 'NoFaceGirl'];
    expect(screenTopic(topic(['real', 'pov'], 'Candy Love rides POV'), { favourites })).toMatchObject({ ok: true, live: true, favourite: 'Candy Love', liveQuota: false });
    expect(screenTopic(topic(['real', 'nofacegirl'], 'Reverse cowgirl'), { favourites })).toMatchObject({ favourite: 'NoFaceGirl', liveQuota: false });
    expect(screenTopic(topic(['real', 'vaginal'], 'Candy Lovers'), { favourites })).toMatchObject({ favourite: null, liveQuota: true });
  });

  it('takes a music post by a creator already in the library even without an animation tag', () => {
    const t = topic(['pmv', 'real'], '[Multi-axis] [Bastati] Yuria Boss Battle 1');
    expect(screenTopic(t, { creators: ['bastati'] })).toMatchObject({ ok: true, music: true, vr: false, artist: 'bastati', liveQuota: false });
    expect(screenTopic(t, { creators: [] })).toMatchObject({ ok: true, liveQuota: true });
  });

  it('skips posts the library already links or whose scene folder it has', () => {
    expect(screenTopic(topic(['animation'], 'X', 7), { knownTopics: new Set([7]) }).reason).toBe('known');
    expect(screenTopic(topic(['animation'], 'Fap Hero - Fantasy Play'), { sceneNames: ['fap hero fantasy play'] }).reason).toBe('in-library');
  });

  it('turns down posts tagged sfw or non-porn before reading them; the sex rule waits for the first post', () => {
    expect(screenTopic(topic(['animation', 'sfw'])).reason).toBe('not-sexual');
    expect(screenTopic(topic(['animation', 'dance', 'non-porn', 'riding'])).reason).toBe('not-sexual');
    expect(screenTopic(topic(['animation', 'titjob'], 'Rio Paizuri'))).toMatchObject({ ok: true });
  });

  it('marks 2D-hentai-only posts so they rank lower', () => {
    expect(screenTopic(topic(['animation', 'hentai', 'vaginal']))).toMatchObject({ ok: true, hentaiOnly: true });
    expect(screenTopic(topic(['3d', 'hentai', 'vaginal']))).toMatchObject({ ok: true, hentaiOnly: false });
  });
});

describe('lessPreferred and the monster / feet / FFM penalty', () => {
  it.each([
    ['monster tag', { tags: ['tentacle'] }, { monster: true, feet: false }],
    ['horse-cock tag', { tags: ['horse-cock'] }, { monster: true, feet: false }],
    ['horse guy in the title', { title: 'Solace2K - Mercy X Horseguy' }, { monster: true, feet: false }],
    ['WW (werewolf) in the title', { title: '2B X WW - BJ' }, { monster: true, feet: false }],
    ['orcs in the title', { title: 'Elf captured by orcs' }, { monster: true, feet: false }],
    ['tentacles in Japanese', { title: '触手 セックス' }, { monster: true, feet: false }],
    ['footjob tag', { tags: ['footjob'] }, { monster: false, feet: true }],
    ['feet in the title', { title: 'Tifa feet and riding' }, { monster: false, feet: true }],
    ['a werewolf and soles in the first post', { title: 'X', text: 'a werewolf licks her soles' }, { monster: true, feet: true }],
    ['succubi, demons, elves and lookalike words', { title: 'Succubus demon elf at the Orchid trolley, Footloose Ww' }, { monster: false, feet: false }],
    ['ffm tag', { tags: ['threesome', 'ffm'] }, { monster: false, feet: false, ffm: true }],
    ['FFM in the title', { title: 'Tifa and Aerith - FFM Threesome' }, { monster: false, feet: false, ffm: true }],
    ['FFM in the first post, not a plain threesome or a word containing it', { tags: ['threesome'], title: 'Staffmeeting', text: 'an ffm scene' }, { monster: false, feet: false, ffm: true }],
  ])('spots %s', (_, post, expected) => {
    expect(lessPreferred(post)).toEqual({ ffm: false, ...expected });
  });

  it('leaves a plain threesome and words containing ffm alone', () => {
    expect(lessPreferred({ tags: ['threesome'], title: 'Staffmeeting' }).ffm).toBe(false);
  });

  it('ranks a monster, feet and FFM lower by their tag-score groups, each once, and they add up', () => {
    const base = { title: 'Tracer rides', tags: ['animation', 'riding'], axisType: 'multi', likes: 20 };
    const plain = candidateScore(base);
    expect([D.monster, D.feet, D.ffm].every((s) => s < 0)).toBe(true);
    expect(candidateScore({ ...base, tags: [...base.tags, 'tentacle'] })).toBe(plain + D.tentacle);
    expect(candidateScore({ ...base, title: 'Tracer rides - footjob' })).toBe(plain + D.footjob);
    expect(candidateScore({ ...base, tags: [...base.tags, 'ffm'] })).toBe(plain + D.ffm);
    expect(candidateScore({ ...base, ffm: true })).toBe(plain + D.ffm);
    expect(candidateScore({ ...base, tags: [...base.tags, 'werewolf', 'feet'] })).toBe(plain + D.werewolf + D.feet);
    expect(candidateScore({ ...base, tags: [...base.tags, 'werewolf', 'feet', 'ffm'] })).toBe(plain + D.werewolf + D.feet + D.ffm);
    expect(candidateScore({ ...base, title: 'Werewolf footjob', tags: [...base.tags, 'werewolf', 'feet'] })).toBe(plain + D.werewolf + D.feet);
    expect(candidateScore({ ...base, monster: true })).toBe(plain + D.monster);
    expect(candidateScore({ ...base, contentBreak: 'feet' })).toBe(plain + D.footjob);
  });
});

describe('blocked creators', () => {
  it('matches a blocked name in the title, as poster or artist, whatever its case, spacing or brackets', () => {
    expect(isBlockedCreator({ title: '[AidenHet] Rina - Lesson In Restraint' })).toBe(true);
    expect(isBlockedCreator({ title: 'Ganyu by aiden het (VR)' })).toBe(true);
    expect(isBlockedCreator({ title: 'Some scene', author: 'Aidenhet' })).toBe(true);
    expect(isBlockedCreator({ title: 'Some scene', performers: ['AIDENHET'] })).toBe(true);
    expect(isBlockedCreator({ title: 'Aiden - Het Party', author: 'Aiden' })).toBe(true);
    expect(isBlockedCreator({ title: 'Aiden rides', author: 'Hetty' })).toBe(false);
  });

  it('keeps blocked posts out of new scans, picks and saved lists', () => {
    expect(screenTopic(topic(['sfm', 'riding'], '[AidenHet] Belle and Zhu Yuan - Movie Night')).reason).toBe('blocked');
    expect(screenPick(topic(['sfm', 'riding'], '[AidenHet] Belle'), {}).reason).toBe('blocked');
    const c = (id, title, author) => ({ id, title, author, tags: ['sfm', 'riding'], axisType: 'multi', score: 1, scenes: [{ axes: ['twist'] }] });
    expect(contentRuleBreak(c(1, 'Mavuika and Xilonen', 'AidenHet'))).toBe('blocked');
    expect(downloadQueue([c(1, '[AidenHet] Mavuika', 'x'), c(2, 'Tracer rides', 'y'), c(3, 'Kafka', 'aidenhet')]).map((x) => x.id)).toEqual([2]);
  });
});

describe('blocked tags (MMD)', () => {
  it('blocks the mmd tag and MMD as a title word, not words containing it', () => {
    expect(hasBlockedTag({ tags: ['mmd', 'riding'] })).toBe(true);
    expect(hasBlockedTag({ tags: ['3d'], title: '[MMD] Idol - Reine' })).toBe(true);
    expect(hasBlockedTag({ tags: ['3d'], title: 'Bengugu MMD - 2 Phut Hon HMV' })).toBe(true);
    expect(hasBlockedTag({ tags: ['3d', 'mmd-like'], title: 'Hammdock riding' })).toBe(false);
  });

  it('keeps MMD posts out of new scans, picks and saved lists', () => {
    expect(screenTopic(topic(['mmd', 'riding'], 'Hu Tao rides')).reason).toBe('blocked');
    expect(screenPick(topic(['3d', 'riding'], 'MMD Hu Tao rides'), {}).reason).toBe('blocked');
    const c = (id, title, tags) => ({ id, title, author: 'x', tags, axisType: 'multi', score: 1, scenes: [{ axes: ['twist'] }] });
    expect(downloadQueue([c(1, 'Reine dance sex', ['mmd', 'sex']), c(2, 'Tracer rides', ['sfm', 'riding'])]).map((x) => x.id)).toEqual([2]);
  });
});

describe('contentRuleBreak', () => {
  it('names the rule a post or saved candidate breaks, by its tags and title; monsters and feet break none', () => {
    expect(contentRuleBreak({ tags: ['sfm', 'riding'], title: 'Solace2K - Mercy X Horseguy' })).toBeNull();
    expect(contentRuleBreak({ tags: ['animation', 'footjob', 'vaginal'], title: 'X' })).toBeNull();
    expect(contentRuleBreak({ tags: ['animation', 'blowjob'], title: 'X' })).toBe('not-sexual');
    expect(contentRuleBreak({ tags: ['animation', 'blowjob', 'riding'], title: 'X' })).toBeNull();
  });

  it('lets sex shown by the first post pass; an old monster / feet mark no longer breaks anything', () => {
    expect(contentRuleBreak({ tags: ['animation', 'straight'], title: 'X', sexual: 'text' })).toBeNull();
    expect(contentRuleBreak({ tags: ['animation', 'sfw'], title: 'X', sexual: 'text' })).toBe('not-sexual');
    expect(contentRuleBreak({ tags: ['animation', 'vaginal'], title: 'X', contentBreak: 'feet' })).toBeNull();
    expect(contentRuleBreak({ tags: ['animation', 'vaginal'], title: 'X', contentBreak: 'monster' })).toBeNull();
    expect(contentRuleBreak({ tags: ['animation', 'vaginal'], title: 'X', contentBreak: 'excluded' })).toBe('excluded');
  });
});

describe('sexEvidence and judgeFirstPost', () => {
  it('says where the sex shows: tags, then title, then the first post', () => {
    expect(sexEvidence({ tags: ['riding'], title: 'X', text: 'riding' })).toBe('tags');
    expect(sexEvidence({ tags: ['pmv'], title: 'Overwatch' })).toBe('tags');
    expect(sexEvidence({ tags: ['animation'], title: 'Tifa rides' })).toBe('title');
    expect(sexEvidence({ tags: ['animation'], title: 'Silver Streaks', text: 'She ends up riding him' })).toBe('text');
    expect(sexEvidence({ tags: ['vam'], title: '[VAM]红衣', text: '骑乘 动作' })).toBe('text');
    expect(sexEvidence({ tags: ['animation'], title: 'X', text: 'a titfuck and oral sex' })).toBeNull();
    expect(sexEvidence({ tags: ['animation', 'sfw'], title: 'X', text: 'sex' })).toBeNull();
  });

  it('turns down a first post naming excluded content, then needs sex somewhere; monster / feet words only flag it', () => {
    expect(judgeFirstPost({ tags: ['animation', 'riding'], title: 'X' }, 'a loli edit')).toEqual({ ok: false, reason: 'excluded' });
    expect(judgeFirstPost({ tags: ['animation', 'riding'], title: 'X' }, 'with tentacles')).toEqual({ ok: true, sexual: 'tags', monster: true, feet: false, ffm: false });
    expect(judgeFirstPost({ tags: ['animation', 'riding'], title: 'X' }, 'soles and toes')).toEqual({ ok: true, sexual: 'tags', monster: false, feet: true, ffm: false });
    expect(judgeFirstPost({ tags: ['animation', 'titjob'], title: 'Rio Paizuri' }, 'paizuri only')).toEqual({ ok: false, reason: 'not-sexual' });
    expect(judgeFirstPost({ tags: ['animation', 'blowjob'], title: 'Seraphine BJ' }, 'then she rides you')).toEqual({ ok: true, sexual: 'text', monster: false, feet: false, ffm: false });
    expect(judgeFirstPost({ tags: ['animation', 'vaginal'], title: 'X' }, 'an FFM threesome')).toEqual({ ok: true, sexual: 'tags', monster: false, feet: false, ffm: true });
  });
});

describe('recheck', () => {
  const c = (title, tags, extra = {}) => ({ id: 1, title, tags, score: 1, ...extra });

  it('reads the first post only of candidates failing just the sex rule without evidence, old monster / feet marks included', () => {
    expect(needsSexRecheck(c('Silver Streaks', ['animation', 'straight']))).toBe(true);
    expect(needsSexRecheck(c('Silver Streaks', ['animation', 'straight'], { sexual: 'text' }))).toBe(false);
    expect(needsSexRecheck(c('Silver Streaks', ['animation', 'straight'], { contentBreak: 'monster' }))).toBe(true);
    expect(needsSexRecheck(c('Silver Streaks', ['animation', 'straight'], { contentBreak: 'excluded' }))).toBe(false);
    expect(needsSexRecheck(c('Tracer rides', ['animation']))).toBe(false);
  });

  it('keeps sex shown by the text (flagging monster / feet), marks excluded text for good, and leaves the rest skipped', () => {
    const post = c('Silver Streaks', ['animation', 'straight']);
    expect(recheckWithText(post, 'She ends up riding him')).toEqual({ outcome: 'text', candidate: { ...post, sexual: 'text', monster: false, feet: false, ffm: false } });
    expect(recheckWithText(post, 'riding a werewolf')).toEqual({ outcome: 'text', candidate: { ...post, sexual: 'text', monster: true, feet: false, ffm: false } });
    expect(recheckWithText({ ...post, contentBreak: 'monster' }, 'riding a werewolf')).toEqual({ outcome: 'text', candidate: { ...post, sexual: 'text', monster: true, feet: false, ffm: false } });
    expect(recheckWithText(post, 'a loli edit')).toEqual({ outcome: 'excluded', candidate: { ...post, contentBreak: 'excluded' } });
    expect(recheckWithText(post, 'a slow blowjob')).toEqual({ outcome: 'not-sexual', candidate: post });
  });
});

describe('isSexual', () => {
  it('needs penetrative sex by tag or title word, in any language, dance or not', () => {
    expect(isSexual({ tags: ['mmd', 'dance', 'solo-female'], title: 'CLC - Devil MMD Dance' })).toBe(false);
    expect(isSexual({ tags: ['mmd', 'dance', 'straight', 'music'], title: 'Miku Tell Me What You Want' })).toBe(false);
    expect(isSexual({ tags: ['mmd', 'dance', 'riding'], title: 'Lantern Rite' })).toBe(true);
    expect(isSexual({ tags: ['mmd', 'dance'], title: 'Iroha Sex Dance' })).toBe(true);
    expect(isSexual({ tags: ['vam'], title: '[VAM]红衣 骑乘' })).toBe(true);
    expect(isSexual({ tags: ['mmd'], title: 'ミク 中出し' })).toBe(true);
    expect(isSexual({ tags: ['3d'], title: 'Getting fucked on the couch' })).toBe(true);
    expect(isSexual({ tags: ['3d'], title: 'Tifa mating press' })).toBe(true);
  });

  it('refuses blowjob, handjob, titjob or paizuri alone, and takes them with sex', () => {
    expect(isSexual({ tags: ['animation', 'titjob', 'titfuck'], title: 'Rio Paizuri' })).toBe(false);
    expect(isSexual({ tags: ['animation', 'blowjob', 'deepthroat', 'facefuck'], title: 'Tifa Side Facefuck' })).toBe(false);
    expect(isSexual({ tags: ['animation', 'handjob', 'cumshot'], title: 'Hand service' })).toBe(false);
    expect(isSexual({ tags: ['animation', 'oral-creampie'], title: 'Miku oral sex and face fucking' })).toBe(false);
    expect(isSexual({ tags: ['animation', 'blowjob', 'riding'], title: 'Bocchi Blowjob' })).toBe(true);
    expect(isSexual({ tags: ['animation', 'blowjob'], title: 'Blowjob then doggy' })).toBe(true);
  });

  it('takes a music video unless it is a dance or about a non-sex act', () => {
    expect(isSexual({ tags: ['hmv', 'beat-based'], title: 'KDA PORN STARS' })).toBe(true);
    expect(isSexual({ tags: ['animation'], title: 'Overwatch PMV' })).toBe(true);
    expect(isSexual({ tags: ['pmv'], title: 'Blowjob PMV' })).toBe(false);
    expect(isSexual({ tags: ['hmv'], title: 'Titjob HMV' })).toBe(false);
    expect(isSexual({ tags: ['cock-hero'], title: 'JOI Cock Hero' })).toBe(false);
    expect(isSexual({ tags: ['pmv'], title: 'Blowjob & Riding PMV' })).toBe(true);
    expect(isSexual({ tags: ['hmv', 'dance', 'music'], title: 'Thumbs up' })).toBe(false);
  });

  it('no longer takes a porn genre tag alone', () => {
    expect(isSexual({ tags: ['hentai', 'straight'], title: 'Fluorite - Caught You Staring' })).toBe(false);
    expect(isSexual({ tags: ['animation', 'solo-female', 'ai-video'], title: 'Golden thread beauty sways' })).toBe(false);
  });

  it('never takes a post tagged sfw or non-porn, and matches whole words only', () => {
    expect(isSexual({ tags: ['dance', 'non-porn', 'riding'], title: 'CH4NGE MMD sex' })).toBe(false);
    expect(isSexual({ tags: ['animation'], title: 'Cumulus Dickens Analysis' })).toBe(false);
    expect(isSexual({ tags: ['animation'], title: 'Sexy titfuck' })).toBe(false);
  });
});

describe('isHentaiOnly', () => {
  it('is 2D hentai: hentai / 2d / anime with nothing 3D in tags or title', () => {
    expect(isHentaiOnly({ tags: ['animation', 'hentai'], title: 'X' })).toBe(true);
    expect(isHentaiOnly({ tags: ['2d', 'riding'], title: 'X' })).toBe(true);
    expect(isHentaiOnly({ tags: ['hentai', 'koikatsu'], title: 'X' })).toBe(false);
    expect(isHentaiOnly({ tags: ['hentai', 'animation'], title: '[SFM] X' })).toBe(false);
    expect(isHentaiOnly({ tags: ['animation', '3d'], title: 'X' })).toBe(false);
  });
});

describe('scriptDurationMs', () => {
  it('is the last action time, so a 1:59 script is under the two-minute minimum', () => {
    expect(scriptDurationMs({ actions: [{ at: 0, pos: 0 }, { at: 119_000, pos: 90 }, { at: 120, pos: 10 }] })).toBe(119_000);
    expect(scriptDurationMs({ actions: [{ at: 119_000 }] })).toBeLessThan(MIN_VIDEO_MS);
    expect(scriptDurationMs({ actions: [{ at: 120_000 }] })).not.toBeLessThan(MIN_VIDEO_MS);
    expect(scriptDurationMs({})).toBe(0);
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
  it('caps likes', () => {
    expect(scoreCandidate({ likes: 10_000 })).toBe(RANKING.likesMax);
  });

  it('ranks a 2D-hentai-only post lower', () => {
    const post = { vr: true, likes: 40 };
    expect(scoreCandidate({ ...post, hentaiOnly: true })).toBe(scoreCandidate(post) - RANKING.hentaiOnly);
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

  it('makes one scene of identical script sets named per encode, with the best video', () => {
    const sized = (size, ...names) => files(...names).map((f) => ({ ...f, size }));
    const { scenes } = planScenes({
      scripts: [
        ...sized(500, 'Portfolio Part 1 - 1080p60fps.funscript', 'Portfolio Part 1 - 4k120fps.funscript'),
        ...sized(80, 'Portfolio Part 1 - 1080p60fps.twist.funscript', 'Portfolio Part 1 - 4k120fps.twist.funscript'),
      ],
      videos: vids('Portfolio Part 1 - 1080p60fps.mp4', 'Portfolio Part 1 - 4k120fps.mp4'),
    });
    expect(scenes.map((s) => s.video.name)).toEqual(['Portfolio Part 1 - 4k120fps.mp4']);
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
  it('never prefers a preview, keeps flat posts at ≤ 4K, and takes more fps for flat but not for VR', () => {
    expect(videoPreference('scene 4K preview.mp4', 1, false)).toBeLessThan(videoPreference('scene 1080p.mp4', 1, false));
    expect(videoPreference('scene 4K.mp4', 1, false)).toBeGreaterThan(videoPreference('scene 8K.mp4', 1, false));
    expect(videoPreference('scene 4K 120fps.mp4', 1, false)).toBeGreaterThan(videoPreference('scene 4K 60fps.mp4', 1, false));
    expect(videoPreference('scene VR 8K 120fps.mp4', 1, true)).toBeLessThan(videoPreference('scene VR 8K 60fps.mp4', 1, true));
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

  it('moves the path-keyed preference scores with their videos', () => {
    const moves = [{ from: '/in/a/3', to: '/in/a/A - B', files: [{ from: '3.mp4', to: 'A - B.mp4' }] }];
    const config = { settings: { library: { preferenceScores: { '/in/a/3/3.mp4': 7.5, '/in/b/x.mp4': -3 } } } };
    expect(remapPaths(config, moves).value.settings.library.preferenceScores).toEqual({ '/in/a/A - B/A - B.mp4': 7.5, '/in/b/x.mp4': -3 });
  });
});

describe('downloadQueue', () => {
  it('orders multi-axis posts by score and drops single-axis and empty ones, VR creampie PMVs too', () => {
    const vrCreampiePmv = { vr: true, tags: ['creampie', 'pmv'], artistInLibrary: true };
    const c = (id, axisType, likes, scenes = 1) => ({ id, axisType, likes, ...vrCreampiePmv, scenes: Array(scenes).fill({ axes: axisType === 'multi' ? ['twist'] : [] }) });
    const q = downloadQueue([c(1, 'single', 9), c(2, 'multi', 1), c(3, 'multi', 5), c(4, 'single', 2), c(5, 'multi', 8, 0)]);
    expect(q.map((x) => x.id)).toEqual([3, 2]);
  });

  it('drops single-axis scenes a saved list holds that the rules no longer want', () => {
    const q = downloadQueue([{ id: 1, axisType: 'multi', score: 1, tags: ['vaginal'], vr: false, scenes: [{ axes: ['roll'] }, { axes: [] }] }, { id: 2, axisType: 'single', score: 9, tags: ['creampie'], vr: false, scenes: [{ axes: [] }] }]);
    expect(q.map((x) => [x.id, x.scenes.length])).toEqual([[1, 1]]);
  });

  it('keeps every scene of a post the user picked, single-axis ones too', () => {
    const q = downloadQueue([{ id: 3, axisType: 'single', score: 1, tags: ['riding'], vr: false, picked: true, scenes: [{ axes: [] }, { axes: [] }] }]);
    expect(q.map((x) => [x.id, x.scenes.length])).toEqual([[3, 2]]);
  });

  it('drops saved posts that break the content rules, picks and favourites too; monster and feet posts stay, ranked lower', () => {
    const c = (id, title, tags, extra = {}) => ({ id, title, tags, axisType: 'multi', score: 1, vr: false, likes: 20, scenes: [{ axes: ['twist'] }], ...extra });
    const q = downloadQueue([
      c(1, 'Solace2K - Mercy X Horseguy', ['sfm', 'riding']),
      c(2, 'Necromancer steamy footjob', ['animation', 'riding']),
      c(3, 'Rio Paizuri', ['animation', 'titjob'], { picked: true }),
      c(4, 'Tracer rides', ['animation', 'riding']),
      c(5, 'Werewolf footjob', ['animation', 'riding']),
    ]);
    expect(q.map((x) => x.id)).toEqual([4, 1, 2, 5]);
  });

  it('takes a saved post whose first post shows the sex, and brings back one an older --recheck marked monster / feet', () => {
    const c = (id, extra) => ({ id, title: 'Silver Streaks', tags: ['animation', 'straight'], axisType: 'multi', score: 1, vr: false, scenes: [{ axes: ['twist'] }], ...extra });
    const q = downloadQueue([c(1, { sexual: 'text' }), c(2, { sexual: 'text', contentBreak: 'monster' }), c(3, {}), c(4, { sexual: 'text', contentBreak: 'excluded' })]);
    expect(q.map((x) => x.id)).toEqual([1, 2]);
  });

  it('orders by the user\'s tag scores too, within multi-axis and single-axis', () => {
    const c = (id, axisType, tags) => ({ id, title: `Scene ${id}`, axisType, tags: ['animation', 'riding', ...tags], vr: false, likes: 0, scenes: [{ axes: axisType === 'multi' ? ['twist'] : [] }] });
    const list = [c(1, 'multi', ['blowjob']), c(2, 'multi', ['ahegao']), c(3, 'multi', [])];
    expect(downloadQueue(list).map((x) => x.id)).toEqual([1, 2, 3]);
    expect(downloadQueue(list, {}).map((x) => x.id)).toEqual([1, 2, 3]);
    expect(downloadQueue(list, { ahegao: 2, blowjob: -1.5 }).map((x) => x.id)).toEqual([2, 3, 1]);
  });
});

describe('screenPick', () => {
  it('takes a picked post past the animation rule, never on the live quota', () => {
    const s = screenPick(topic(['pmv', 'real'], '[JJULEZ] Some PMV', 5), { favourites: ['JJULEZ'] });
    expect(s).toMatchObject({ ok: true, picked: true, liveQuota: false, favourite: 'JJULEZ' });
    expect(screenPick(topic(['real', 'riding'], 'Somebody rides', 9))).toMatchObject({ ok: true, live: true, liveQuota: false });
  });

  it('still refuses exclusions, the content rules, posts the library links or the user turned down, and library scenes', () => {
    expect(screenPick(topic(['pmv', 'futanari'], 'X', 6))).toEqual({ ok: false, reason: 'excluded' });
    expect(screenPick(topic(['sfm', 'pmv'], '2B X WW - BJ', 6))).toMatchObject({ ok: true, picked: true });
    expect(screenPick(topic(['pmv', 'sfw'], 'X', 6))).toEqual({ ok: false, reason: 'not-sexual' });
    expect(screenPick(topic(['pmv'], 'X', 7), { knownTopics: new Set([7]) })).toEqual({ ok: false, reason: 'known' });
    expect(screenPick(topic(['pmv'], 'JJULEZ - Work That Body', 8), { sceneNames: ['jjulez work that body'] })).toEqual({ ok: false, reason: 'in-library' });
  });
});

describe('wantedScenes', () => {
  it('takes only scenes with hand-made axes, VR creampie PMVs included', () => {
    const scenes = [{ axes: ['twist'] }, { axes: [] }];
    expect(wantedScenes({ scenes })).toEqual([{ axes: ['twist'] }]);
    expect(wantedScenes({ vr: true, tags: ['creampie', 'hmv'], artistInLibrary: true, scenes })).toEqual([{ axes: ['twist'] }]);
  });
});

describe('scan state', () => {
  // A feed of `n` posts, newest first (ids n..1), 30 per page.
  const feed = (n) => {
    const reads = [];
    const listPage = async (tag, page) => {
      reads.push(page);
      const ids = Array.from({ length: n }, (_, i) => n - i).slice(page * 30, page * 30 + 30);
      return { topics: ids.map((id) => ({ id })), more: (page + 1) * 30 < n };
    };
    return { reads, listPage };
  };
  const markAll = (state, topics) => topics.forEach((t) => markTopic(state, t.id, 'candidate', 0));

  it('reads new pages up to the cap on a first run, then backfills from there', async () => {
    const state = emptyScoutState();
    const f = feed(300);
    const r = await listFeed(state, 'mmd', f.listPage, { maxNew: 3, backfill: 2 });
    expect(f.reads).toEqual([0, 1, 2, 3, 4]);
    expect(r.topics).toHaveLength(150);
    expect(state.feeds.mmd).toEqual({ backfillPage: 5, exhausted: false });
  });

  it('stops at the first page with nothing new, and the backfill goes on from its cursor', async () => {
    const state = emptyScoutState();
    await listFeed(state, 'mmd', feed(300).listPage, { maxNew: 3, backfill: 2 }).then((r) => markAll(state, r.topics));
    const f = feed(310); // ten new posts on top
    const r = await listFeed(state, 'mmd', f.listPage, { maxNew: 3, backfill: 2 });
    expect(f.reads).toEqual([0, 1, 5, 6]);
    expect(r.topics.filter((t) => !seenTopic(state, t.id)).map((t) => t.id).slice(0, 10)).toEqual([310, 309, 308, 307, 306, 305, 304, 303, 302, 301]);
    expect(state.feeds.mmd.backfillPage).toBe(7);
  });

  it('marks a feed done at its end and stops backfilling it', async () => {
    const state = emptyScoutState();
    const f = feed(70);
    await listFeed(state, 'vam', f.listPage, { maxNew: 1, backfill: 5 });
    expect(f.reads).toEqual([0, 1, 2]);
    expect(state.feeds.vam.exhausted).toBe(true);
  });

  it('judges posts again after a rules change, and reads a foreign state as empty', () => {
    const state = emptyScoutState();
    markTopic(state, 7, 'not-sexual');
    expect(seenTopic(state, 7)).toBe(true);
    state.topics[7].rules = RULES_VERSION - 1;
    expect(seenTopic(state, 7)).toBe(false);
    expect(normalizeScoutState({ version: 99 })).toEqual(emptyScoutState());
  });

  // Like feed(), with post `id` last active `id - 100` days after 2025-01-01.
  const DAY = 864e5;
  const bumpedOf = (id) => new Date(Date.UTC(2025, 0, 1) + (id - 100) * DAY).toISOString();
  const datedFeed = (n) => {
    const f = feed(n);
    return { reads: f.reads, listPage: async (tag, page) => {
      const r = await f.listPage(tag, page);
      return { ...r, topics: r.topics.map((t) => ({ ...t, bumped: bumpedOf(t.id) })) };
    } };
  };

  it('keeps posts a stopped run listed but never judged, though the cursor moved past them', async () => {
    const state = emptyScoutState();
    const f = feed(300);
    const limited = async (tag, page) => { if (page === 3) throw new Error('Rate limited'); return f.listPage(tag, page); };
    await expect(listFeed(state, 'mmd', limited, { maxNew: 2, backfill: 3, keepPending: true })).rejects.toThrow('Rate limited');
    expect(state.feeds.mmd.backfillPage).toBe(3);
    const left = normalizeScoutState(JSON.parse(JSON.stringify(state))); // as the next run reads it
    expect(pendingTopics(left).map((t) => t.id)).toEqual(Array.from({ length: 90 }, (_, i) => 300 - i));
    markTopic(left, 300, 'candidate');
    markTopic(left, 299, 'no-video');
    expect(pendingTopics(left)).toHaveLength(88);
    expect(normalizeScoutState({ version: 1, feeds: {}, topics: {} }).pending).toEqual({});
  });

  it('backfills down to --since, skips older posts, and stays there until an earlier date', async () => {
    const state = emptyScoutState();
    const f = datedFeed(300); // ids 100 and up are active since 2025-01-01, page 7 (ids 90..61) is all older
    const r = await listFeed(state, 'multi-axis', f.listPage, { maxNew: 1, backfill: 50, since: '2025-01-01' });
    expect(f.reads).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
    expect(r.topics.map((t) => t.id)).toEqual(Array.from({ length: 201 }, (_, i) => 300 - i));
    expect(state.feeds['multi-axis']).toMatchObject({ backfillPage: 6, doneSince: '2025-01-01', exhausted: false }); // back on the first page with older posts
    markAll(state, r.topics);

    const plain = datedFeed(300);
    await listFeed(state, 'multi-axis', plain.listPage, { maxNew: 1, backfill: 2 });
    expect(plain.reads).toEqual([0]);
    const later = datedFeed(300);
    await listFeed(state, 'multi-axis', later.listPage, { maxNew: 1, backfill: 2, since: '2025-02-01' });
    expect(later.reads).toEqual([0]);

    const earlier = datedFeed(300);
    const more = await listFeed(state, 'multi-axis', earlier.listPage, { maxNew: 1, backfill: 2, since: '2024-12-01' });
    expect(earlier.reads).toEqual([0, 6, 7]);
    expect(more.topics.filter((t) => !seenTopic(state, t.id)).map((t) => t.id)).toEqual(Array.from({ length: 31 }, (_, i) => 99 - i)); // down to id 69 = 2024-12-01
  });

  it('judges a no-video or single-axis post again after new activity, other verdicts never', async () => {
    const state = emptyScoutState();
    const at = Date.parse('2026-10-01T00:00:00Z');
    markTopic(state, 5, 'no-video', at);
    markTopic(state, 6, 'single-axis', at);
    markTopic(state, 7, 'excluded', at);
    expect(needsJudging(state, { id: 5, bumped: '2026-10-02T00:00:00Z' })).toBe(true);
    expect(needsJudging(state, { id: 6, bumped: '2026-10-02T00:00:00Z' })).toBe(true);
    expect(needsJudging(state, { id: 5, bumped: '2026-09-30T00:00:00Z' })).toBe(false);
    expect(needsJudging(state, { id: 5 })).toBe(false);
    expect(needsJudging(state, { id: 7, bumped: '2026-10-02T00:00:00Z' })).toBe(false);
    expect(needsJudging(state, { id: 8 })).toBe(true);

    // A bumped no-video post on the first page keeps the scan reading, and waits as pending.
    const seen = emptyScoutState();
    markAll(seen, (await listFeed(emptyScoutState(), 'mmd', datedFeed(300).listPage, { maxNew: 10, backfill: 0 })).topics);
    const quiet = datedFeed(300);
    await listFeed(seen, 'mmd', quiet.listPage, { maxNew: 3, backfill: 0, keepPending: true });
    expect(quiet.reads).toEqual([0]);
    markTopic(seen, 290, 'no-video', 0); // judged long before its last activity
    const bumped = datedFeed(300);
    await listFeed(seen, 'mmd', bumped.listPage, { maxNew: 3, backfill: 0, keepPending: true });
    expect(bumped.reads).toEqual([0, 1]);
    expect(pendingTopics(seen).map((t) => t.id)).toEqual([290]);
  });

  it('keeps earlier candidates until every scene is fetched, drops posts turned down or breaking the content rules, and ranks monsters lower', () => {
    const c = (id, likes, ...videos) => ({ id, likes, title: 'X', tags: ['vaginal'], scenes: videos.map((name) => ({ video: { name } })) });
    const done = new Set(['1|a.mp4', '2|b.mp4']);
    const merged = mergeCandidates([c(1, 5, 'a.mp4'), c(2, 3, 'b.mp4', 'c.mp4'), c(3, 9, 'd.mp4'), { ...c(5, 8, 'f.mp4'), tags: ['tentacle', 'vaginal'] }],
      [c(4, 7, 'e.mp4'), c(2, 4, 'b.mp4', 'c.mp4'), { ...c(6, 6, 'g.mp4'), tags: ['titjob'] }],
      (id, v) => done.has(`${id}|${v}`), new Set([3]));
    expect(merged.map((x) => x.id)).toEqual([4, 2, 5]);
  });

  it('scores a saved list again under the current weights', () => {
    const saved = { id: 1, title: 'X', tags: ['vaginal'], likes: 0, axisType: 'multi', score: 1, scenes: [{ video: { name: 'a.mp4' } }] };
    const [c] = mergeCandidates([saved], [], () => false);
    expect(c.score).toBe(scoreCandidate({ multiAxis: true }));
  });

  it('scores saved and fresh candidates with the user\'s tag scores', () => {
    const c = (id, tags) => ({ id, title: `Scene ${id}`, tags: ['vaginal', ...tags], likes: 0, axisType: 'multi', score: 1, scenes: [{ video: { name: `${id}.mp4` } }] });
    const plain = mergeCandidates([c(1, ['pov']), c(2, ['ahegao'])], [c(3, [])], () => false);
    const scored = mergeCandidates([c(1, ['pov']), c(2, ['ahegao'])], [c(3, [])], () => false, new Set(), { ahegao: 3, POV: 1, pov: -2 });
    const by = (list) => Object.fromEntries(list.map((x) => [x.id, x.score]));
    expect(by(scored)).toEqual({ 1: by(plain)[1] - D.pov - 2, 2: by(plain)[2] + 3, 3: by(plain)[3] });
    expect(scored.map((x) => x.id)).toEqual([2, 3, 1]);
  });
});

describe('addToDiscovery', () => {
  it("adds the scout's folder as a source and the playlist once, then only new videos", () => {
    let n = 0;
    const newId = () => `id${++n}`;
    const config = { settings: { library: { sources: [{ id: 's', name: 'Other', path: '/GVP/Other' }] } }, playlists: [] };
    const folder = { name: 'EroScripts', path: '/GVP/EroScripts' };
    const first = addToDiscovery(config, ['/GVP/EroScripts/a/a.mp4'], { folder, newId, now: 1 });
    expect(first).toEqual({ sourceAdded: true, playlistCreated: true, added: ['/GVP/EroScripts/a/a.mp4'] });
    const again = addToDiscovery(config, ['/GVP/EroScripts/a/a.mp4', '/GVP/EroScripts/b/b.mp4'], { folder: { ...folder, path: '/GVP/EroScripts/' }, newId });
    expect(again).toEqual({ sourceAdded: false, playlistCreated: false, added: ['/GVP/EroScripts/b/b.mp4'] });
    expect(config.settings.library.sources.map((s) => [s.name, s.path])).toEqual([['Other', '/GVP/Other'], ['EroScripts', '/GVP/EroScripts']]);
    expect(config.playlists).toEqual([{ id: 'id2', name: DISCOVERY_NAME, createdAt: 1, videoPaths: ['/GVP/EroScripts/a/a.mp4', '/GVP/EroScripts/b/b.mp4'] }]);
  });

  it('uses an existing Discovery playlist whatever its case', () => {
    const config = { settings: { library: { sources: [{ path: '/in' }] } }, playlists: [{ id: 'p', name: ' discovery ', videoPaths: [] }] };
    expect(addToDiscovery(config, ['/in/x.mp4'], { folder: { name: 'FapTap', path: '/in' }, newId: () => 'new' }).playlistCreated).toBe(false);
    expect(config.playlists[0].videoPaths).toEqual(['/in/x.mp4']);
  });
});
