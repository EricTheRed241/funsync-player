// Association shape helpers.
//
// `library.associations[videoPath]` keeps all three configs in parallel
// with an `active` pointer so switching between modes is lossless and the
// last-active mode restores on reopen:
//
//   {
//     active: 'single' | 'multi' | 'custom' | null,
//     single: '<path>' | null,
//     multi:  { main, axes, buttplugVib } | null,
//     custom: { routes: [...] } | null,
//
//     // Back-compat mirror fields (so if a user downgrades the app to a
//     // pre-refactor version, the OLD read path still sees the active
//     // config for custom and multi. Single-axis downgrade regresses to
//     // auto-pairing only — old code can't read an object as a string.)
//     mode?: 'custom',
//     routes?: [...],
//     main?: '<path>',
//     axes?: { ... },
//     buttplugVib?: boolean,
//   }
//
// Every read point in the app should go through `normalizeAssociation()`
// first; every write through `buildAssociationEntry()`.

/** @typedef {'single'|'multi'|'custom'|null} ActiveMode */

/**
 * Canonicalise a stored association entry: normalise missing slots to null
 * and rebuild the mirror fields. Anything without the `active` marker (or
 * null / undefined) becomes an empty entry. Pure — no settings I/O.
 *
 * @param {*} raw  stored entry / null / undefined
 * @returns {{
 *   active: ActiveMode,
 *   single: string|null,
 *   multi: object|null,
 *   custom: object|null,
 *   [k: string]: unknown
 * }}
 */
export function normalizeAssociation(raw) {
  if (!raw || typeof raw !== 'object' || !('active' in raw)) {
    return _buildEntry(null, null, null, null);
  }
  return _buildEntry(
    raw.active ?? null,
    raw.single ?? null,
    raw.multi ?? null,
    raw.custom ?? null,
  );
}

/**
 * Build a new-shape entry with mirror fields for downgrade safety.
 * Any slot value of null is preserved verbatim.
 *
 * @param {ActiveMode} active
 * @param {string|null} single
 * @param {object|null} multi
 * @param {object|null} custom
 */
export function buildAssociationEntry(active, single, multi, custom) {
  return _buildEntry(active, single, multi, custom);
}

function _buildEntry(active, single, multi, custom) {
  const entry = { active, single, multi, custom };
  // Mirror the active slot's old-shape fields onto the top level so a
  // user who downgrades to a pre-refactor FunSync version still sees
  // their active config.
  //   - active=custom → {mode:'custom', routes} at top
  //   - active=multi  → {main, axes, buttplugVib} at top
  //   - active=single → can't mirror (old code needs a string, not an
  //                     object) — accept the regression for that case
  if (active === 'custom' && custom) {
    entry.mode = 'custom';
    entry.routes = custom.routes || [];
  } else if (active === 'multi' && multi) {
    entry.main = multi.main || null;
    entry.axes = multi.axes || {};
    entry.buttplugVib = !!multi.buttplugVib;
  }
  return entry;
}

/**
 * Get the active config payload from a normalised entry. Returns null if
 * no slot is active, or the active slot is empty.
 *
 * @returns {{kind: Exclude<ActiveMode, null>, config: *} | null}
 */
export function resolveActiveConfig(entry) {
  if (!entry || !entry.active) return null;
  const value = entry[entry.active];
  if (value == null) return null;
  return { kind: entry.active, config: value };
}

/**
 * Return true if the entry has a usable config at the given slot.
 * "Usable" means: single has a non-empty path; multi has main or any axis;
 * custom has at least one route with a scriptPath.
 */
export function hasConfigForMode(entry, mode) {
  if (!entry) return false;
  const v = entry[mode];
  if (!v) return false;
  if (mode === 'single') return typeof v === 'string' && v.length > 0;
  if (mode === 'multi') {
    return !!v.main || Object.values(v.axes || {}).some(Boolean);
  }
  if (mode === 'custom') {
    return Array.isArray(v.routes) && v.routes.some(r => r?.scriptPath);
  }
  return false;
}
