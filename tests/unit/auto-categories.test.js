/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Categories made from EroScripts post tags (renderer/js/auto-categories.js):
// what gets created and assigned, and that the user's own edits stick.
import { describe, it, expect } from 'vitest';
import {
  planAutoCategories, applyAutoCategories, planVRCategory, applyVRCategory, planAxisCategories, AUTO_CATEGORIES_KEY,
} from '../../renderer/js/auto-categories.js';
import { AUTO_CATEGORIES } from '../../renderer/js/content-preferences.js';
import { SCRIPT_TAGS_KEY } from '../../renderer/js/script-tags.js';

const ALL = AUTO_CATEGORIES.length;
const cat = (id, name) => ({ id, name });

describe('planAutoCategories', () => {
  it('fills each category from its tags, reusing a category of the same name', () => {
    const r = planAutoCategories({
      categories: [cat('c', ' creampie '), cat('r', 'Riding'), cat('v', 'VR')],
      scriptTags: { '/a.mp4': { tags: ['anal-creampie', 'reverse-cowgirl'] }, '/b.mp4': { tags: ['osr2'] }, '/gone.mp4': { tags: ['vr'] } },
      videoPaths: ['/a.mp4', '/b.mp4'],
    });
    expect(r.create.map((d) => d.key)).not.toContain('creampie');
    expect(r.create).toHaveLength(ALL - 3);
    expect(r.pairs).toEqual([['/a.mp4', 'c'], ['/a.mp4', 'r']]);
    expect(r.state.assigned).toEqual({ '/a.mp4': ['creampie', 'riding'] });
  });

  it('never makes a deleted category again, and never re-adds a video taken out by hand', () => {
    const state = { created: AUTO_CATEGORIES.map((d) => d.key), assigned: { '/a.mp4': ['creampie'] } };
    const r = planAutoCategories({
      categories: [cat('c', 'Creampie')],
      scriptTags: { '/a.mp4': { tags: ['creampie', 'vr'] } },
      videoPaths: ['/a.mp4'],
      state,
    });
    expect(r.create).toEqual([]);
    expect(r.pairs).toEqual([]);
    // VR was deleted: nothing assigned, and not recorded, so a restored VR category gets it later.
    expect(r.state.assigned['/a.mp4']).toEqual(['creampie']);
  });

  it('records a video already in the category without assigning it twice', () => {
    const r = planAutoCategories({
      categories: [cat('c', 'Creampie')],
      videoCategories: { '/a.mp4': ['c'] },
      scriptTags: { '/a.mp4': { tags: ['creampie'] } },
      videoPaths: ['/a.mp4'],
    });
    expect(r.pairs).toEqual([]);
    expect(r.state.assigned).toEqual({ '/a.mp4': ['creampie'] });
  });
});

describe('applyAutoCategories', () => {
  function data(scriptTags) {
    const store = { [SCRIPT_TAGS_KEY]: scriptTags };
    const categories = [];
    const assigned = [];
    let n = 0;
    return {
      assigned,
      get: (k) => store[k],
      set: (k, v) => { store[k] = v; },
      getCategories: () => categories,
      addCategory: async (name, color) => { const c = { id: `id${++n}`, name, color }; categories.push(c); return c; },
      assignCategories: (pairs) => assigned.push(...pairs),
      _cache: { videoCategories: {} },
    };
  }

  it('creates the categories, assigns once, and a second run does nothing', async () => {
    const d = data({ '/a.mp4': { tags: ['creampie', 'pmv'] } });
    expect(await applyAutoCategories(d, ['/a.mp4'])).toEqual({ created: ALL, assigned: 2 });
    const names = Object.fromEntries(d.getCategories().map((c) => [c.id, c.name]));
    expect(d.assigned.map(([p, id]) => [p, names[id]])).toEqual([['/a.mp4', 'Creampie'], ['/a.mp4', 'PMV / HMV']]);
    expect(d.get(AUTO_CATEGORIES_KEY).assigned).toEqual({ '/a.mp4': ['creampie', 'pmv'] });
    expect(await applyAutoCategories(d, ['/a.mp4'])).toEqual({ created: 0, assigned: 0 });
  });
});

describe('VR category from the videos', () => {
  it('ignores the post tag "vr" (a "2D/VR" post no longer makes a flat video VR)', () => {
    const r = planAutoCategories({
      categories: [cat('v', 'VR')],
      scriptTags: { '/flat.mp4': { tags: ['vr', 'non-vr'] } },
      videoPaths: ['/flat.mp4'],
    });
    expect(r.pairs).toEqual([]);
    expect(r.state.assigned).toEqual({});
  });

  it('adds VR videos once, takes out flat ones the app put there, leaves the user\'s alone', () => {
    const r = planVRCategory({
      categories: [cat('c', 'Creampie'), cat('v', ' vr ')],
      videoCategories: {
        '/old-tag-flat.mp4': ['v', 'c'], // put in VR by the old tag rule, decided flat
        '/manual-flat.mp4': ['v'], // put in VR by hand, decided flat
        '/manual-vr.mp4': ['v'], // put in VR by hand, decided VR
        '/old-tag-vr.mp4': ['v'],
      },
      flags: {
        '/new-vr.mp4': true,
        '/old-tag-flat.mp4': false,
        '/manual-flat.mp4': false,
        '/manual-vr.mp4': true,
        '/old-tag-vr.mp4': true,
        '/taken-out.mp4': true,
        '/plain-flat.mp4': false,
      },
      state: {
        created: ['vr'],
        assigned: {
          '/old-tag-flat.mp4': ['creampie', 'vr'],
          '/old-tag-vr.mp4': ['vr'],
          '/taken-out.mp4': ['vr'], // auto-assigned, then removed by hand
        },
      },
    });
    expect(r.add).toEqual([['/new-vr.mp4', 'v']]);
    expect(r.remove).toEqual([['/old-tag-flat.mp4', 'v']]);
    expect(r.state.created).toEqual(['vr']);
    expect(r.state.assigned).toEqual({
      '/old-tag-flat.mp4': ['creampie'],
      '/old-tag-vr.mp4': ['vr'],
      '/taken-out.mp4': ['vr'],
      '/new-vr.mp4': ['vr'],
      '/manual-vr.mp4': ['vr-manual'],
    });
  });

  it('never takes out a video that was already in the category when it was found VR', () => {
    const r = planVRCategory({
      categories: [cat('v', 'VR')],
      videoCategories: { '/a.mp4': ['v'] },
      flags: { '/a.mp4': false },
      state: { assigned: { '/a.mp4': ['vr-manual'] } },
    });
    expect(r.remove).toEqual([]);
  });

  it('leaves videos out of the flags (frame size still unknown) alone, and does nothing without the category', () => {
    const state = { assigned: { '/a.mp4': ['vr'] } };
    const r = planVRCategory({ categories: [cat('v', 'VR')], videoCategories: { '/a.mp4': ['v'] }, flags: {}, state });
    expect(r).toEqual({ add: [], remove: [], state: { assigned: { '/a.mp4': ['vr'] } } });
    expect(planVRCategory({ categories: [], flags: { '/b.mp4': true }, state })).toEqual({ add: [], remove: [], state });
  });

  it('applies the plan to the library and records it once', () => {
    const store = { [AUTO_CATEGORIES_KEY]: { created: ['vr'], assigned: { '/flat.mp4': ['vr'] } } };
    const videoCategories = { '/flat.mp4': ['v'] };
    const assigned = [];
    const unassigned = [];
    const d = {
      get: (k) => store[k],
      set: (k, v) => { store[k] = v; },
      getCategories: () => [cat('v', 'VR')],
      assignCategories: (pairs) => assigned.push(...pairs),
      unassignCategory: (p, id) => unassigned.push([p, id]),
      _cache: { videoCategories },
    };
    const flags = { '/vr.mp4': true, '/flat.mp4': false };
    expect(applyVRCategory(d, flags)).toEqual({ added: 1, removed: 1 });
    expect(assigned).toEqual([['/vr.mp4', 'v']]);
    expect(unassigned).toEqual([['/flat.mp4', 'v']]);
    expect(store[AUTO_CATEGORIES_KEY].assigned).toEqual({ '/vr.mp4': ['vr'] });
    videoCategories['/vr.mp4'] = ['v'];
    videoCategories['/flat.mp4'] = [];
    expect(applyVRCategory(d, flags)).toEqual({ added: 0, removed: 0 });
  });
});

describe('planAxisCategories', () => {
  const cats = [cat('h', 'Hand-made axes'), cat('g', 'Generated axes'), cat('s', 'Single-axis')];

  it('puts each video in the category of its kind once, and leaves undecided videos alone', () => {
    const r = planAxisCategories({
      categories: cats,
      videoCategories: { '/b.mp4': ['g'] },
      flags: { '/a.mp4': 'handmade', '/b.mp4': 'generated', '/c.mp4': 'single' },
      state: { assigned: { '/x.mp4': ['creampie'] } },
    });
    expect(r.add).toEqual([['/a.mp4', 'h'], ['/c.mp4', 's']]);
    expect(r.remove).toEqual([]);
    expect(r.state.assigned).toEqual({
      '/x.mp4': ['creampie'], '/a.mp4': ['axes-handmade'], '/b.mp4': ['axes-generated'], '/c.mp4': ['axes-single'],
    });
  });

  it('moves a video whose kind changed, and a video taken out by hand stays out', () => {
    const r = planAxisCategories({
      categories: cats,
      videoCategories: { '/up.mp4': ['s', 'other'], '/out.mp4': [] },
      flags: { '/up.mp4': 'handmade', '/out.mp4': 'generated' },
      state: { assigned: { '/up.mp4': ['creampie', 'axes-single'], '/out.mp4': ['axes-generated'] } },
    });
    expect(r.add).toEqual([['/up.mp4', 'h']]);
    expect(r.remove).toEqual([['/up.mp4', 's']]);
    expect(r.state.assigned).toEqual({ '/up.mp4': ['creampie', 'axes-handmade'], '/out.mp4': ['axes-generated'] });
  });

  it('does nothing for a category the user deleted', () => {
    const r = planAxisCategories({ categories: [cat('g', 'Generated axes')], flags: { '/a.mp4': 'handmade' } });
    expect(r.add).toEqual([]);
    expect(r.state.assigned).toEqual({});
  });
});
