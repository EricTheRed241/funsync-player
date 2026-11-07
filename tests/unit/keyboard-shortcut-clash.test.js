/**
 * @vitest-environment node
 * Reads source text, no DOM. See notes/CLAUDE.md "Test environments".
 */
// Keyboard shortcut clash guard.
//
// Dave, 2026-08-15: "we should need a list of the current keyboard shortcuts
// in the knowledge base for easy reference and no overlap in future as well
// as when they are available".
//
// The reference lives in the vault (Player Systems/Keyboard Shortcuts.md).
// This file is the half a document cannot do: it fails the build when a new
// binding collides with an existing one.
//
// It reads the SAME data the `?` overlay renders, so a shortcut that exists
// in code but was never added to the help is invisible here — which is
// itself the point: undocumented bindings are how clashes get introduced.
// If you add a binding, add it to keyboard-help.js, and this will tell you
// whether it is free.
//
// Sharing one chord within the player context is a bug: one of the two
// actions can never fire.
//
// Reads keyboard-help.js as TEXT rather than importing it. Importing pulls
// in Modal and dataService, which touch IPC at module load and hang the
// worker — and executing app code to lint a table is the wrong trade anyway.
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HELP_SRC = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), '../../renderer/js/keyboard-help.js'),
  'utf8',
);

/** Rows look like: ['Ctrl+Z', t('kbd.editorUndo')] */
const ROW_RE = /\['([^']+)',\s*t\('kbd\.([A-Za-z]+)'\)\]/g;
const TITLE_RE = /title:\s*t\('kbd\.([A-Za-z]+)'\)/;

/** Slice the source down to the player shortcut builder. */
function sectionsOf(src) {
  const pi = src.indexOf('export function getPlayerShortcutGroups');
  if (pi < 0) throw new Error('keyboard-help.js builder not found');
  return { player: src.slice(pi) };
}

function rowsOf(section) {
  const rows = [];
  let group = '(ungrouped)';
  for (const line of section.split(/\r?\n/)) {
    const tm = line.match(TITLE_RE);
    if (tm) group = tm[1];
    ROW_RE.lastIndex = 0;
    let m;
    while ((m = ROW_RE.exec(line))) rows.push({ group, keys: m[1], action: m[2] });
  }
  return rows;
}

/**
 * Expand a displayed binding into canonical chords.
 *
 * "Shift+Up / Down" means Shift+Up AND Shift+Down — a modifier carries to
 * later alternatives that do not state their own. Without that rule this
 * check invents clashes that do not exist, which is exactly how the first
 * draft of it wasted an hour.
 */
export function expandBinding(label) {
  const out = [];
  let carried = [];
  for (const raw of String(label).split(/\s*\/\s*/)) {
    let part = raw.trim();
    if (!part) continue;
    const mods = [];
    for (const m of ['Ctrl', 'Shift', 'Alt']) {
      const re = new RegExp(`${m}\\+`);
      if (re.test(part)) {
        mods.push(m);
        part = part.replace(re, '');
      }
    }
    const applied = mods.length ? mods : carried;
    if (mods.length) carried = mods;
    const base = part.trim();
    if (!base) continue;
    out.push(applied.length ? [...applied, base].join('+') : base);
  }
  return out;
}

function bindingMap(section) {
  const map = new Map();
  for (const { group, keys, action } of rowsOf(section)) {
    for (const chord of expandBinding(keys)) {
      if (!map.has(chord)) map.set(chord, []);
      map.get(chord).push(`${group} → ${action}`);
    }
  }
  return map;
}

const SECTIONS = sectionsOf(HELP_SRC);

// Chords that legitimately appear twice in one context because the handler
// disambiguates them at runtime by state, not by chord. Each needs a reason.
const ALLOWED_DUPLICATES = {
  player: {
    // Esc is layered: it clears an A-B loop if one is set, otherwise it
    // leaves the player. Both are documented so the user can find either.
    Esc: 2,
  },
};

describe('expandBinding', () => {
  it('carries a modifier across alternatives', () => {
    expect(expandBinding('Shift+Up / Down')).toEqual(['Shift+Up', 'Shift+Down']);
    expect(expandBinding('Ctrl+Left / Right')).toEqual(['Ctrl+Left', 'Ctrl+Right']);
  });

  it('lets a later alternative override the carried modifier', () => {
    expect(expandBinding('Ctrl+Y / Ctrl+Shift+Z')).toEqual(['Ctrl+Y', 'Ctrl+Shift+Z']);
  });

  it('leaves unmodified alternatives alone', () => {
    expect(expandBinding('F / F11')).toEqual(['F', 'F11']);
    expect(expandBinding('Space / K')).toEqual(['Space', 'K']);
  });
});

describe('no chord is bound twice in the same context', () => {
  for (const name of ['player']) {
    it(`${name}`, () => {
      const map = bindingMap(SECTIONS[name]);
      const allowed = ALLOWED_DUPLICATES[name] || {};
      const clashes = [];
      for (const [chord, uses] of map) {
        if (uses.length > (allowed[chord] || 1)) {
          clashes.push(`${chord} is bound ${uses.length}x: ${uses.join(' | ')}`);
        }
      }
      expect(clashes, `\n${clashes.join('\n')}\n`).toEqual([]);
    });
  }
});
