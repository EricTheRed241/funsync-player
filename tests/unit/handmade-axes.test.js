/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Hand-made axis scripts always win: the shared rule.
import { describe, it, expect } from 'vitest';
import {
  handmadeAxisSuffixes,
  hasHandmadeAxes,
  generatedCompanionFiles,
  preferHandmadeAxes,
  filesBeside,
} from '../../renderer/js/handmade-axes.js';

const two = [{ at: 0, pos: 10 }, { at: 100, pos: 90 }];
const one = [{ at: 0, pos: 10 }];

describe('handmadeAxisSuffixes — companion files', () => {
  it('counts a companion that is not `.generated.`', () => {
    expect(handmadeAxisSuffixes({
      primaryPath: 'Title.funscript',
      allFiles: ['Title.funscript', 'Title.twist.funscript', 'Title.generated.roll.funscript'],
    })).toEqual(['twist']);
  });

  it('ignores generated companions, other titles and AppleDouble files', () => {
    const allFiles = [
      'Title.funscript',
      'Title.generated.twist.funscript',
      'Other.pitch.funscript',
      '._Title.roll.funscript',
    ];
    expect(hasHandmadeAxes({ primaryPath: 'Title.funscript', allFiles })).toBe(false);
  });

  it('reports canonical suffixes for alias spellings', () => {
    expect(handmadeAxisSuffixes({
      primaryPath: '/lib/T/Title.funscript',
      allFiles: ['/lib/T/Title.r1.funscript', '/lib/T/Title.yaw.funscript', '/lib/T/Title.suction.funscript'],
    })).toEqual(['roll', 'suck', 'twist']);
  });
});

describe('handmadeAxisSuffixes — embedded axes', () => {
  const formats = {
    'additional_axes (A)': { actions: two, additional_axes: [{ axis: 'twist', actions: two }] },
    'raw (B)': { actions: two, raw: { twist: { actions: two } } },
    'sibling keys (C)': { actions: two, twist: two },
    'inline TCode (D)': { actions: [{ at: 0, pos: 0, R0: 10 }, { at: 100, pos: 100, R0: 90 }] },
    'axes by TCode id (E)': { actions: two, axes: [{ id: 'R0', actions: two }] },
    'channels by name (F)': { actions: two, channels: { twist: { metadata: {}, actions: two } } },
    'channels by TCode id (F)': { actions: two, channels: { R0: two } },
  };
  for (const [label, parsed] of Object.entries(formats)) {
    it(`counts ${label}`, () => {
      expect(handmadeAxisSuffixes({ primaryPath: 'Title.funscript', parsed })).toEqual(['twist']);
    });
  }

  it('ignores an embedded axis with fewer than 2 actions, in every format', () => {
    const parsed = {
      actions: [{ at: 0, pos: 0, R1: 10 }, { at: 100, pos: 100 }],
      additional_axes: [{ axis: 'twist', actions: one }],
      raw: { pitch: { actions: one } },
      axes: [{ id: 'L1', actions: one }],
      channels: { sway: one, R2: { actions: [] } },
      vib: one,
    };
    expect(hasHandmadeAxes({ primaryPath: 'Title.funscript', parsed })).toBe(false);
  });

  it('unions companions and embedded axes', () => {
    expect(handmadeAxisSuffixes({
      primaryPath: 'Title.funscript',
      allFiles: ['Title.pitch.funscript'],
      parsed: { actions: two, channels: { roll: two, suction: two } },
    })).toEqual(['pitch', 'roll', 'suck']);
  });

  it('a plain stroke script with no companions has none', () => {
    expect(hasHandmadeAxes({ primaryPath: 'Title.funscript', allFiles: ['Title.funscript'], parsed: { actions: two } }))
      .toBe(false);
    expect(hasHandmadeAxes({ primaryPath: 'Title.funscript', parsed: null })).toBe(false);
  });
});

describe('generatedCompanionFiles', () => {
  it("lists only the script's own generated companions", () => {
    expect(generatedCompanionFiles('Title.funscript', [
      'Title.generated.twist.funscript',
      'Title.twist.funscript',
      'Title_hard.generated.roll.funscript',
      '._Title.generated.pitch.funscript',
      'Title.GENERATED.Pitch.funscript',
    ])).toEqual(['Title.generated.twist.funscript', 'Title.GENERATED.Pitch.funscript']);
  });
});

describe('preferHandmadeAxes', () => {
  const dir = '/lib/T';
  it('drops generated paths and fills from hand-made companions', () => {
    const axes = { twist: `${dir}/Title.generated.twist.funscript`, roll: `${dir}/Title.generated.roll.funscript` };
    const allFiles = [`${dir}/Title.generated.twist.funscript`, `${dir}/Title.generated.roll.funscript`, `${dir}/Title.twist.funscript`];
    expect(preferHandmadeAxes(axes, { primaryPath: `${dir}/Title.funscript`, allFiles }))
      .toEqual({ twist: `${dir}/Title.twist.funscript` });
  });

  it('drops every generated path when the axes are embedded', () => {
    const axes = { twist: `${dir}/Title.generated.twist.funscript` };
    const parsed = { actions: two, channels: { twist: two } };
    expect(preferHandmadeAxes(axes, { primaryPath: `${dir}/Title.funscript`, parsed })).toEqual({});
  });

  it('returns the same object when there is nothing hand-made', () => {
    const axes = { twist: `${dir}/Title.generated.twist.funscript` };
    expect(preferHandmadeAxes(axes, { primaryPath: `${dir}/Title.funscript`, allFiles: Object.values(axes) })).toBe(axes);
  });

  it('keeps a user-picked hand-made file for an axis', () => {
    const axes = { twist: '/elsewhere/Custom.twist.funscript' };
    const allFiles = [`${dir}/Title.twist.funscript`];
    expect(preferHandmadeAxes(axes, { primaryPath: `${dir}/Title.funscript`, allFiles })).toBe(axes);
  });
});

describe('filesBeside', () => {
  it('keeps files in the folders of the anchors only', () => {
    const all = ['/a/T.twist.funscript', '/b/T.twist.funscript', 'C:\\v\\T.roll.funscript'];
    expect(filesBeside(all, '/a/T.funscript', null, 'C:\\v\\T.mp4'))
      .toEqual(['/a/T.twist.funscript', 'C:\\v\\T.roll.funscript']);
  });
});
