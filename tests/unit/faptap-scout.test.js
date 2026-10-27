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
  livePerformerNames, hasLivePerformer, mergeCookies, withPerformers, faptapArtist, faptapSceneName, thumbnailUrl,
} from '../../scripts/faptap-scout-lib.mjs';
import { emptyScoutState, listFeed, markTopic, seenTopic, RULES_VERSION } from '../../scripts/eroscripts-scout-lib.mjs';

const video = (tagNames, extra = {}) => ({
  id: '1', name: 'Bulma Yoga', description: '', duration: 300, user: { username: 'Zoot' },
  tags: tagNames.map((name) => ({ name })), ...extra,
});
// Two focus themes, so tests about other rules aren't turned down as off-focus.
const FOCUS2 = ['Creampie', 'HMV'];

describe('naming: Artist - Title (metadata)', () => {
  const ctx = { favourites: ['ScyllaHMV', 'Noodledude', 'Candy Love'], creatorNames: ['Zoot'] };
  const name = (title, extra = {}) => faptapSceneName({ title, uploader: 'Scripter42', ...extra }, ctx);

  it('files a scene under the artist, not the scripter who uploaded it', () => {
    expect(name('MAI MASHER (ScyllaHMV)')).toEqual({ artist: 'ScyllaHMV', creator: 'ScyllaHMV', stem: 'ScyllaHMV - MAI MASHER' });
    expect(name('Cute Faces & Goated Throats (by ScyllaHMV)').stem).toBe('ScyllaHMV - Cute Faces & Goated Throats');
    expect(name('Scylla Compilation HMV (Godlike)').artist).toBe('ScyllaHMV');
    expect(name('Fit babe rides', { performers: ['Candy Love'] }).stem).toBe('Candy Love - Fit babe rides');
  });

  it('falls back to the title\'s own artist, a library creator, a model or pornstar, then the uploader', () => {
    expect(name('[Kinzq] Black Cat RCG - Extended').stem).toBe('Kinzq - Black Cat RCG - Extended');
    expect(faptapArtist({ title: 'Ride with Zoot tonight', uploader: 'u' }, ctx)).toBe('Zoot');
    expect(faptapArtist({ title: 'Fap Hero Era R5', uploader: 'u', performers: [{ name: 'zoot', category: '' }] }, ctx)).toBe('Zoot');
    expect(faptapArtist({ title: 'Late night', uploader: 'u', performers: [{ name: 'Some Model', category: 'model' }] }, ctx)).toBe('Some Model');
    expect(faptapArtist({ title: 'Eternum - Alex Scene', uploader: 'plvg', performers: [{ name: 'Alex Bardot', category: '' }] }, ctx)).toBe('plvg');
    expect(name('Fap Hero - The Spire').stem).toBe('Scripter42 - Fap Hero - The Spire');
  });

  it('keeps a title that is only the artist and a bracket readable', () => {
    expect(faptapSceneName({ title: 'THEOBROBINE (Fap Hero)', uploader: 'u' }, { creatorNames: ['Theobrobine'] }).stem).toBe('Theobrobine - Fap Hero');
  });

  it('does not take an upload label for an artist, and keeps another leading name as metadata', () => {
    expect(name('[Script Overhaul] Cock Hero / Clench Trainer SX Tapes', { tags: ['noodledude'] }).artist).toBe('Noodledude');
    expect(name('[Poppet] I Want Your Cum HMV', { tags: ['noodledude'] }).stem).toBe('Noodledude - I Want Your Cum HMV (Poppet)');
  });

  it('ends a VR scene with the projection the headset reads', () => {
    expect(name('Kiriko rides (ScyllaHMV)', { vr: true, projection: 'STEREO_180_LR' }).stem).toBe('ScyllaHMV - Kiriko rides_180_LR');
  });
});

describe('thumbnailUrl', () => {
  it('makes FapTap thumbnails absolute and refuses anything else', () => {
    expect(thumbnailUrl({ thumbnail_url: 'thumbnails/17692.webp' })).toBe('https://faptap.net/api/assets/thumbnails/17692.webp');
    expect(thumbnailUrl({ thumbnail_url: 'https://vz-1.b-cdn.net/abc/thumbnail.jpg' })).toBe('https://vz-1.b-cdn.net/abc/thumbnail.jpg');
    expect(thumbnailUrl({ thumbnail_url: '../secrets' })).toBeNull();
    expect(thumbnailUrl({})).toBeNull();
  });
});

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

  it('takes animated videos, FapTap tag spellings included; live action only waits for the live quota', () => {
    expect(screenVideo(video(['Animated 3d', ...FOCUS2]))).toMatchObject({ ok: true, live: false, liveQuota: false });
    expect(screenVideo(video(['animated 2d', ...FOCUS2])).live).toBe(false);
    expect(screenVideo(video(['ai-video', ...FOCUS2])).live).toBe(false);
    expect(screenVideo(video(['Hentai', 'amateur anal', ...FOCUS2]))).toMatchObject({ ok: true, live: true, liveQuota: true });
    expect(screenVideo(video(['Animation', 'OnlyFans', ...FOCUS2])).liveQuota).toBe(true);
    expect(screenVideo(video(['POV', 'Creampie', 'HMV'])).liveQuota).toBe(true);
  });

  it('takes live action by a favourite (title, uploader, performer or tag) without the quota or the focus rule', () => {
    const favourites = ['Noodledude', 'NoFaceGirl', 'Candy Love'];
    expect(screenVideo(video(['PMV', 'tiktok'], { name: 'FOLLOW ME Noodledude' }), { favourites })).toMatchObject({ ok: true, live: true, favourite: 'Noodledude', liveQuota: false });
    expect(screenVideo(video(['Amateur', 'POV'], { name: 'Fit babe rides', performers: [{ name: 'NoFaceGirl' }] }), { favourites }).favourite).toBe('NoFaceGirl');
    expect(screenVideo(video(['Amateur', 'Blowjob', 'noodledude'], { name: 'IWYC' }), { favourites }).favourite).toBe('Noodledude');
    expect(screenVideo(video(['Amateur', 'POV'], { name: 'Candy Lovers' }), { favourites }).reason).toBe('off-focus');
  });

  it('finds live action and favourites among the performers once the details are known', () => {
    const s = screenVideo(video(['Animation', ...FOCUS2], { name: 'Late night' }));
    expect(withPerformers(s, [{ name: 'Molly Little', category: 'model' }], [])).toMatchObject({ live: true, liveQuota: true });
    expect(withPerformers(s, [{ name: 'Candy Love', category: 'model' }], ['Candy Love'])).toMatchObject({ live: true, favourite: 'Candy Love', liveQuota: false });
    expect(withPerformers(s, [{ name: 'Moecode', category: '' }], [])).toMatchObject({ live: false, liveQuota: false });
  });

  it('lets a video picked by id past the animation, focus and sex rules, but not the exclusions', () => {
    expect(screenVideo(video(['Amateur', 'POV'], { name: 'Pov boobfuck' }), { picked: true })).toMatchObject({ ok: true, picked: true, liveQuota: false });
    expect(screenVideo(video(['Amateur', 'Futa']), { picked: true }).reason).toBe('excluded');
    expect(screenVideo(video(['Amateur'], { duration: 30 }), { picked: true }).reason).toBe('short');
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

  it('takes a music video by a creator already in the library without an animation tag, outside the quota', () => {
    const v = video(['PMV', 'Cock Hero', 'Creampie'], { name: 'Sunset Cock Hero', user: { username: 'Bastati' } });
    expect(screenVideo(v, { creators: ['bastati'] })).toMatchObject({ ok: true, music: true, artist: 'bastati', liveQuota: false });
    expect(screenVideo(v, { creators: [] }).liveQuota).toBe(true);
  });

  it('does not take a genre folder (Cock Hero, PMV) for a creator', () => {
    const v = video(['PMV', 'Blowjob', 'Creampie'], { name: 'NON STOP - Cock Hero', user: { username: 'someone' } });
    expect(screenVideo(v, { creators: ['cock hero', 'pmv'] })).toMatchObject({ artist: null, liveQuota: true });
  });

  it('counts a video as live action when its title names a FapTap model or pornstar', () => {
    const livePerformers = livePerformerNames([
      { name: 'Molly Little', category: 'model' }, { name: 'Kendra Sunderland', category: 'pornstar' },
      { name: 'Moecode', category: 'none' }, { name: 'Ava', category: 'model' },
    ]);
    expect(livePerformers).toEqual(['molly little', 'kendra sunderland']);
    expect(screenVideo(video(['Animation', ...FOCUS2], { name: 'Molly Little Dirty Talk Blowjob' }), { livePerformers }).live).toBe(true);
    expect(screenVideo(video(['Animation', ...FOCUS2], { name: 'Ava rides Ganondorf' }), { livePerformers }).live).toBe(false);
  });

  it('spots a model or pornstar listed as the performer', () => {
    expect(hasLivePerformer({ performers: [{ name: 'Molly Little', category: 'model' }] })).toBe(true);
    expect(hasLivePerformer({ performers: [{ name: 'Moecode', category: '' }] })).toBe(false);
    expect(hasLivePerformer({ performers: null })).toBe(false);
  });

  it('needs sex and at least two minutes', () => {
    expect(screenVideo(video(['Animated', 'dance', 'HMV', 'VR'], { name: 'Bulma dances' })).reason).toBe('not-sexual');
    expect(screenVideo(video(['Animated', 'non-nude', ...FOCUS2])).reason).toBe('not-sexual');
    expect(screenVideo(video(['Animated', ...FOCUS2], { duration: 119 })).reason).toBe('short');
    expect(screenVideo(video(['Animated', ...FOCUS2], { duration: 120 })).ok).toBe(true);
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

  it('keeps one of several uploads of the same video: the best stream, then the most liked', () => {
    const c = (id, quality, likes, title = 'Agapilord - HMV - SEXUAL') => ({ id, score: 5, tags: ['creampie', 'hmv'], title, duration: 190, vr: false, quality, likes });
    const list = mergeCandidates([c('1', 720, 50)], [c('2', 1080, 3), c('3', 1080, 9), c('4', 1080, 1, 'Something Else')], () => false);
    expect(list.map((x) => x.id).sort()).toEqual(['3', '4']);
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
