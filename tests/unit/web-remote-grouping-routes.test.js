/**
 * @vitest-environment node
 * Web remote grouping detail routes (backend/web-remote/grouping-routes.js).
 * Tapping a category used to build `#categorie/<id>` (the plural minus its
 * last letter), which no route matched, so the phone fell back to the
 * library; the detail route itself turned `category` into `categorys`.
 */
import { describe, it, expect } from 'vitest';
import {
  groupingDetailHash, parseGroupingDetailHash,
} from '../../backend/web-remote/grouping-routes.js';

describe('grouping detail routes', () => {
  it('a tapped category opens that category', () => {
    const hash = groupingDetailHash('categories', 'cat-vr');
    expect(hash).toBe('category/cat-vr');
    expect(parseGroupingDetailHash(hash)).toEqual({ kind: 'categories', id: 'cat-vr' });
  });

  it.each([
    ['collections', 'collection/c1'],
    ['playlists', 'playlist/p1'],
    ['categories', 'category/x1'],
  ])('%s round-trip through the hash', (kind, expected) => {
    const id = expected.split('/')[1];
    expect(groupingDetailHash(kind, id)).toBe(expected);
    expect(parseGroupingDetailHash(expected)).toEqual({ kind, id });
  });

  it('keeps ids with slashes and dashes whole', () => {
    expect(parseGroupingDetailHash('category/a/b-c')).toEqual({ kind: 'categories', id: 'a/b-c' });
  });

  it('other hashes are not grouping details', () => {
    for (const hash of ['', 'categories', 'categorie/x', 'categorys/x', 'play/abc', 'folder/x', 'category/']) {
      expect(parseGroupingDetailHash(hash)).toBeNull();
    }
  });

  it('rejects an unknown kind instead of building a dead route', () => {
    expect(() => groupingDetailHash('tags', 'x')).toThrow();
  });
});
