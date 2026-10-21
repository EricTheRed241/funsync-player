import { describe, it, expect } from 'vitest';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { underscoreVariant } = require('../../electron/script-variant.js');

describe('underscoreVariant', () => {
  it('labels Title_hardcore as a variant of Title', () => {
    expect(underscoreVariant('Marin - Summer Date_hardcore', ['Marin - Summer Date']))
      .toEqual({ stem: 'Marin - Summer Date', label: 'hardcore' });
  });

  it('keeps a funscript named after its own video as primary', () => {
    expect(underscoreVariant('2B City_4k', ['2B City_4k', '2B City_6k'])).toBeNull();
  });

  it('prefers the longest video stem', () => {
    expect(underscoreVariant('Scene_eve_alt', ['Scene', 'Scene_eve']))
      .toEqual({ stem: 'Scene_eve', label: 'alt' });
  });

  it('handles titles containing dots', () => {
    expect(underscoreVariant('Animex 3.0.1_hard', ['Animex 3.0.1']))
      .toEqual({ stem: 'Animex 3.0.1', label: 'hard' });
  });

  it('matches case-insensitively and keeps the label casing', () => {
    expect(underscoreVariant('title_HMV', ['Title'])).toEqual({ stem: 'Title', label: 'HMV' });
  });

  it('requires an underscore separator and a non-empty label', () => {
    expect(underscoreVariant('Title Extended', ['Title'])).toBeNull();
    expect(underscoreVariant('Title2', ['Title'])).toBeNull();
    expect(underscoreVariant('Title_', ['Title'])).toBeNull();
  });

  it('returns null with no videos in the folder', () => {
    expect(underscoreVariant('Title_hard', [])).toBeNull();
    expect(underscoreVariant('Title_hard', undefined)).toBeNull();
  });
});
