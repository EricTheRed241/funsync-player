/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run faptap:scout — which faptap.net videos qualify, which stream is
// taken and how FapTap's script becomes a funscript
// (scripts/faptap-scout-lib.mjs). No faptap.net.
import { describe, it, expect } from 'vitest';
import {
  tagSlug, videoTags, screenVideo, pickSource, csvToFunscript, vrSuffix, toCandidate, mergeCandidates, fetchedKey, FAPTAP_RULES_VERSION,
  livePerformerNames, hasLivePerformer, mergeCookies,
} from '../../scripts/faptap-scout-lib.mjs';
import { emptyScoutState, listFeed, markTopic, seenTopic, RULES_VERSION } from '../../scripts/eroscripts-scout-lib.mjs';

const video = (tagNames, extra = {}) => ({
  id: '1', name: 'Bulma Yoga', description: '', duration: 300, user: { username: 'Zoot' },
  tags: tagNames.map((name) => ({ name })), ...extra,
});
// Two focus themes, so tests about other rules aren't turned down as off-focus.
const FOCUS2 = ['Creampie', 'HMV'];

describe('tagSlug', () => {
  it('spells FapTap tags like EroScripts tags', () => {
    expect(tagSlug('Cum in Mouth')).toBe('cum-in-mouth');
    expect(tagSlug(' Doggystyle ')).toBe('doggystyle');
    expect(tagSlug('Five Nights at Freddy\'s')).toBe('five-nights-at-freddy-s');
  });
});

describe('videoTags', () => {
  it('uses the tags the library already has for FapTap spellings', () => {
    const tags = videoTags(video(['Cum Inside Pussy', 'Reverse Cowgirl Position', 'Animated 3d', 'PMV Compilation', 'Creampie', 'Big Tits']));
    expect(tags).toEqual(['creampie', 'reverse-cowgirl', '3d', 'pmv', 'big-tits']);
  });

  it("adds a character tag's franchise", () => {
    expect(videoTags(video(['Mercy (Overwatch)', 'Tifa Lockhart (Final Fantasy)']))).toEqual(['mercy-overwatch', 'overwatch', 'tifa-lockhart-final-fantasy', 'final-fantasy']);
  });
});

describe('screenVideo', () => {
  it.each([
    ['a futa tag', video(['Animated', 'Futa', 'Riding'])],
    ['a hyphenated furry tag', video(['Animation', 'gay furry animation', 'Sex'])],
    ['a furry tag that does not say furry', video(['Animated', 'scalie', 'Sex'])],
    ['futa in the title', video(['Hentai', 'Riding'], { name: 'Futa Elf rides' })],
    ['loli in the description', video(['Hentai', 'Riding'], { description: '<p>loli edit</p>' })],
  ])('never takes %s', (_, v) => {
    expect(screenVideo(v)).toEqual({ ok: false, reason: 'excluded' });
  });

  it('takes animated videos, FapTap tag spellings included, but not live action', () => {
    expect(screenVideo(video(['Animated 3d', ...FOCUS2])).ok).toBe(true);
    expect(screenVideo(video(['animated 2d', ...FOCUS2])).ok).toBe(true);
    expect(screenVideo(video(['ai-video', ...FOCUS2])).ok).toBe(true);
    expect(screenVideo(video(['Hentai', 'amateur anal', ...FOCUS2])).reason).toBe('not-animated');
    expect(screenVideo(video(['Animation', 'OnlyFans', ...FOCUS2])).reason).toBe('not-animated');
    expect(screenVideo(video(['POV', 'Creampie'])).reason).toBe('not-animated');
  });

  it('needs two of PMV / HMV, creampie, reverse cowgirl and VR, by tag, title or VR flag', () => {
    expect(screenVideo(video(['Hentai', 'HMV', 'Cum Inside Pussy'])).focus).toEqual(['PMV / HMV', 'creampie']);
    expect(screenVideo(video(['Hentai', 'Fap Hero', 'reverse cowgirl pov'])).ok).toBe(true);
    expect(screenVideo(video(['Hentai', 'Creampie'], { vr: true })).focus).toEqual(['creampie', 'VR']);
    expect(screenVideo(video(['Hentai', 'Virtual Reality', 'Reverse Cowgirl'])).ok).toBe(true);
    expect(screenVideo(video(['Hentai', 'Riding'], { name: 'Bulma reverse cowgirl creampie' })).ok).toBe(true);
    expect(screenVideo(video(['Hentai', 'Reverse Cowgirl'])).reason).toBe('off-focus');
    expect(screenVideo(video(['Hentai', 'HMV'])).reason).toBe('off-focus');
    expect(screenVideo(video(['Hentai', 'Cowgirl', 'Blowjob'])).reason).toBe('off-focus');
  });

  it('never takes an audio roleplay', () => {
    expect(screenVideo(video(['Hentai', ...FOCUS2, 'asmr audio roleplay'])).reason).toBe('audio-only');
    expect(screenVideo(video(['Hentai', ...FOCUS2, 'lewdlexiaudio'])).reason).toBe('audio-only');
    expect(screenVideo(video(['Hentai', ...FOCUS2], { name: 'Checking On Your Teacher [Erotic Audio Roleplay]' })).reason).toBe('audio-only');
    expect(screenVideo(video(['Hentai', ...FOCUS2, 'Audio', 'Audio Based'])).ok).toBe(true);
  });

  it('takes a music video by a creator already in the library without an animation tag', () => {
    const v = video(['PMV', 'Cock Hero', 'Creampie'], { name: 'Sunset Cock Hero', user: { username: 'Bastati' } });
    expect(screenVideo(v, { creators: ['bastati'] })).toMatchObject({ ok: true, music: true, artist: 'bastati' });
    expect(screenVideo(v, { creators: [] }).reason).toBe('not-animated');
  });

  it('does not take a genre folder (Cock Hero, PMV) for a creator', () => {
    const v = video(['PMV', 'Blowjob'], { name: 'NON STOP - Cock Hero', user: { username: 'someone' } });
    expect(screenVideo(v, { creators: ['cock hero', 'pmv'] }).reason).toBe('not-animated');
  });

  it('turns down a live video tagged as animation when its title names a FapTap model or pornstar', () => {
    const livePerformers = livePerformerNames([
      { name: 'Molly Little', category: 'model' }, { name: 'Kendra Sunderland', category: 'pornstar' },
      { name: 'Moecode', category: 'none' }, { name: 'Ava', category: 'model' },
    ]);
    expect(livePerformers).toEqual(['molly little', 'kendra sunderland']);
    expect(screenVideo(video(['Animation', 'Blowjob'], { name: 'Molly Little Dirty Talk Blowjob' }), { livePerformers }).reason).toBe('not-animated');
    expect(screenVideo(video(['Animation', ...FOCUS2], { name: 'Ava rides Ganondorf' }), { livePerformers }).ok).toBe(true);
  });

  it('spots a model or pornstar listed as the performer', () => {
    expect(hasLivePerformer({ performers: [{ name: 'Molly Little', category: 'model' }] })).toBe(true);
    expect(hasLivePerformer({ performers: [{ name: 'Moecode', category: '' }] })).toBe(false);
    expect(hasLivePerformer({ performers: null })).toBe(false);
  });

  it('needs sex and at least a minute', () => {
    expect(screenVideo(video(['Animated', 'dance', 'HMV', 'VR'], { name: 'Bulma dances' })).reason).toBe('not-sexual');
    expect(screenVideo(video(['Animated', 'non-nude', ...FOCUS2])).reason).toBe('not-sexual');
    expect(screenVideo(video(['Animated', ...FOCUS2], { duration: 59 })).reason).toBe('short');
    expect(screenVideo(video(['Animated', ...FOCUS2], { duration: 60 })).ok).toBe(true);
  });

  it('skips a video the library or the EroScripts list already has under its own title', () => {
    const v = video(['Animated', ...FOCUS2], { name: 'YOGA TIME with Bulma! - Zoot Interactive' });
    expect(screenVideo(v, { knownTitles: ['zoot yoga time with bulma'] }).reason).toBe('in-library');
    expect(screenVideo(v, { knownTitles: ['zoot yoga time with chichi'] }).ok).toBe(true);
  });

  it('reports VR from the flag or the tag, and the preferred tags', () => {
    expect(screenVideo(video(['Animated', 'Creampie', 'Riding'], { vr: true }))).toMatchObject({ vr: true, preferred: ['creampie', 'riding'] });
    expect(screenVideo(video(['Animated', 'VR', 'Creampie'])).vr).toBe(true);
  });
});

describe('pickSource', () => {
  const src = (...qualities) => qualities.map((q) => ({ url: `https://cdn/${q}p/video.m3u8`, quality: String(q), format: 'hls' }));

  it('takes the best stream within the cap', () => {
    expect(pickSource(src(2160, 1080, 720), false).height).toBe(2160);
    expect(pickSource(src(4320, 2160), false).height).toBe(2160);
    expect(pickSource(src(4320, 2160), true).height).toBe(4320);
  });

  it('turns down preview-only uploads: under 720p flat, under 1080p VR', () => {
    expect(pickSource(src(480, 240), false)).toBeNull();
    expect(pickSource(src(720), false).height).toBe(720);
    expect(pickSource(src(720), true)).toBeNull();
    expect(pickSource([], false)).toBeNull();
  });
});

describe('csvToFunscript', () => {
  it('keeps time order, one action per millisecond, positions within 0..100', () => {
    const s = csvToFunscript('500,50\n100,120\nms,pos\n\n300,-4\n500,70\r\n');
    expect(s.actions).toEqual([{ at: 100, pos: 100 }, { at: 300, pos: 0 }, { at: 500, pos: 70 }]);
    expect(s).toMatchObject({ version: '1.0', inverted: false, range: 100 });
  });
});

describe('vrSuffix', () => {
  it('names the projection the way the headset reads it', () => {
    expect(vrSuffix('STEREO_180_LR')).toBe('_180_LR');
    expect(vrSuffix('STEREO_180_TB_SPHERICAL')).toBe('_180_TB');
    expect(vrSuffix('STEREO_FISHEYE_200_LR')).toBe('_MKX200');
    expect(vrSuffix('MONO_FLAT')).toBe('');
  });
});

describe('mergeCandidates', () => {
  it('keeps waiting candidates still in focus, drops fetched, rejected or off-focus ones, best first', () => {
    const c = (id, score, tags = ['creampie', 'hmv']) => ({ id, score, tags, title: '' });
    const list = mergeCandidates([c('1', 1), c('2', 5), c('4', 9, ['reverse-cowgirl'])], [c('3', 3), c('1', 4)], (id) => id === '2');
    expect(list.map((x) => [x.id, x.score])).toEqual([['1', 4], ['3', 3]]);
  });

  it('ranks VR and preferred tags above plain videos', () => {
    const plain = toCandidate(video(['Animated', 'HMV', 'PMV']), screenVideo(video(['Animated', 'HMV', 'Reverse Cowgirl'])));
    const vr = toCandidate(video(['Animated', 'Creampie'], { vr: true }), screenVideo(video(['Animated', 'Creampie'], { vr: true })));
    expect(vr.score).toBeGreaterThan(plain.score);
    expect(plain).toMatchObject({ id: '1', url: 'https://faptap.net/v/1', author: 'Zoot', focus: ['PMV / HMV', 'reverse cowgirl'] });
    expect(fetchedKey(plain.id)).toBe('faptap|1');
  });
});

describe('mergeCookies', () => {
  it('replaces cookies by name, drops cleared ones, and keeps the rest', () => {
    const jar = mergeCookies('', ['auth=Y3NyZg==; Path=/; Max-Age=86400', 'theme=dark; Path=/']);
    expect(jar).toBe('auth=Y3NyZg==; theme=dark');
    expect(mergeCookies(jar, ['auth=session2; Path=/; HttpOnly', 'theme=; Max-Age=0'])).toBe('auth=session2');
    expect(mergeCookies(jar, [])).toBe(jar);
  });
});

describe('scan state with the FapTap rules version', () => {
  it('counts a video as judged only under its own rules version', () => {
    const state = emptyScoutState();
    markTopic(state, '7', 'candidate', 0, FAPTAP_RULES_VERSION + 100);
    expect(seenTopic(state, '7', FAPTAP_RULES_VERSION + 100)).toBe(true);
    expect(seenTopic(state, '7', RULES_VERSION + 200)).toBe(false);
  });

  it('stops reading new pages at a page judged under the given rules', async () => {
    const state = emptyScoutState();
    const rules = 42;
    for (const id of ['a', 'b']) markTopic(state, id, 'candidate', 0, rules);
    const pages = [[{ id: 'a' }, { id: 'b' }], [{ id: 'c' }]];
    const read = async (_feed, page) => ({ topics: pages[page] || [], more: page < pages.length - 1 });
    expect((await listFeed(state, 'all', read, { maxNew: 5, backfill: 0, rules })).pagesRead).toBe(1);
    expect((await listFeed(emptyScoutState(), 'all', read, { maxNew: 5, backfill: 0, rules })).pagesRead).toBe(2);
  });
});
