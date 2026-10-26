/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Categories made from EroScripts post tags (renderer/js/auto-categories.js):
// what gets created and assigned, and that the user's own edits stick.
import { describe, it, expect } from 'vitest';
import { planAutoCategories, applyAutoCategories, AUTO_CATEGORIES, AUTO_CATEGORIES_KEY } from '../../renderer/js/auto-categories.js';
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
