/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The web remote's default sort and its one-time switch
// (backend/web-remote/sort-default.js).
import { describe, it, expect } from 'vitest';
import { DEFAULT_SORT, SORT_DEFAULT_VERSION, migrateSortState } from '../../backend/web-remote/sort-default.js';
import { DEFAULT_SORT as DESKTOP_DEFAULT_SORT } from '../../renderer/js/content-preferences.js';

describe('web remote default sort', () => {
  it('is the preference score, highest first, like the desktop', () => {
    expect(DEFAULT_SORT).toBe('score:desc');
    expect(DEFAULT_SORT).toBe(DESKTOP_DEFAULT_SORT);
  });

  it('starts a fresh browser on the score sort', () => {
    const { state, changed } = migrateSortState({});
    expect(changed).toBe(true);
    expect(state).toEqual({ sort: 'score:desc', sortDirs: { score: 'desc' }, sortDefault: SORT_DEFAULT_VERSION });
    expect(migrateSortState(null).state.sort).toBe('score:desc');
  });

  it('switches a saved default-looking sort once, keeping the rest', () => {
    for (const sort of ['name:asc', 'dateAdded:desc']) {
      const stored = { tab: 'matched', sort, sortDirs: { name: 'asc', score: 'asc' }, view: 'list' };
      const { state, changed } = migrateSortState(stored);
      expect(changed).toBe(true);
      expect(state).toEqual({
        tab: 'matched', sort: 'score:desc', sortDirs: { name: 'asc', score: 'desc' }, view: 'list', sortDefault: SORT_DEFAULT_VERSION,
      });
    }
  });

  it('keeps a sort the user chose before', () => {
    const { state, changed } = migrateSortState({ sort: 'duration:desc', sortDirs: { duration: 'desc' } });
    expect(changed).toBe(true); // records that it ran
    expect(state.sort).toBe('duration:desc');
    expect(state.sortDirs).toEqual({ duration: 'desc' });
  });

  it('runs only once: a later choice sticks, Name A-Z included', () => {
    const stored = { sort: 'name:asc', sortDefault: SORT_DEFAULT_VERSION };
    const { state, changed } = migrateSortState(stored);
    expect(changed).toBe(false);
    expect(state).toBe(stored);
    expect(state.sort).toBe('name:asc');
  });
});
