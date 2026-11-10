// The web remote's default sort and its one-time switch to it.
//
// Preference score, highest first, unscored last (user, 2026-11-10); the
// same as the desktop's DEFAULT_SORT in renderer/js/content-preferences.js
// (the PWA can't import renderer files, so it's mirrored here).
//
// The phone saves its sort in localStorage (`funsync.remote.ui`, written on
// every filter change, so it holds the old default `name:asc` even when the
// user never chose a sort). `migrateSortState` runs once per browser: a
// saved sort that looks like a default (none, `name:asc` or `dateAdded:desc`)
// becomes DEFAULT_SORT, and `sortDefault` records that it ran, so a sort the
// user picks afterwards (Name A-Z included) sticks.

export const DEFAULT_SORT = 'score:desc';
/** Stored as `sortDefault` once the switch ran. */
export const SORT_DEFAULT_VERSION = 1;

const DEFAULT_LOOKING = new Set(['name:asc', 'dateAdded:desc']);

/**
 * The saved UI state with the one-time default switch applied.
 *
 * @param {object} stored parsed `funsync.remote.ui` (may be `{}`)
 * @returns {{state: object, changed: boolean}} `changed`: the state should be saved
 */
export function migrateSortState(stored) {
  const state = stored && typeof stored === 'object' ? stored : {};
  if (state.sortDefault >= SORT_DEFAULT_VERSION) return { state, changed: false };
  const next = { ...state, sortDefault: SORT_DEFAULT_VERSION };
  if (!state.sort || DEFAULT_LOOKING.has(state.sort)) {
    const [field, dir] = DEFAULT_SORT.split(':');
    next.sort = DEFAULT_SORT;
    next.sortDirs = { ...(state.sortDirs || {}), [field]: dir };
  }
  return { state: next, changed: true };
}
