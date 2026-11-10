/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Favourites in the Discovery scouts (scripts/scout-live.mjs).
import { describe, it, expect } from 'vitest';
import { favouriteIn } from '../../scripts/scout-live.mjs';

describe('favouriteIn', () => {
  const favourites = ['Candy Love', 'NoFaceGirl', 'ScyllaHMV', 'Noodledude'];

  it('matches whole words in any text, spaced or run together', () => {
    expect(favouriteIn(['INSANE compilation + Candy Love GODLIKE'], favourites)).toBe('Candy Love');
    expect(favouriteIn(['Amateur Babe No Face Girl rides'], favourites)).toBe('NoFaceGirl');
    expect(favouriteIn(['The Hana Song Experience [ScyllaHMV]'], favourites)).toBe('ScyllaHMV');
    expect(favouriteIn(['Damn Right PMV', 'noodledude'], favourites)).toBe('Noodledude');
  });

  it('does not match inside other names', () => {
    expect(favouriteIn(['Candy Lovers in Paris', 'Love Candy'], favourites)).toBeNull();
    expect(favouriteIn([null, '', undefined], favourites)).toBeNull();
  });
});
