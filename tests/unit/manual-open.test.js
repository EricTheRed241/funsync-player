/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// npm run manual:open — which posts and pages of the download list open,
// in what order (scripts/manual-open-lib.mjs).
import { describe, it, expect } from 'vitest';
import { parseManualList, postsToOpen, batchUrls, withoutCopySuffix } from '../../scripts/manual-open-lib.mjs';
import { manualListText } from '../../scripts/eroscripts-scout-lib.mjs';

const posts = [
  { id: 1, title: 'Anna L. (Hegre)', url: 'https://discuss.eroscripts.com/t/1', files: [
    { name: 'anna.mp4', size: 1.3e9, page: 'https://pixeldrain.com/u/A' },
    { name: 'anna SBS.mp4', size: 12e9, page: 'https://pixeldrain.com/u/B' },
  ] },
  { id: 2, title: 'Pack 2. (part 1)', url: 'https://discuss.eroscripts.com/t/2', files: [{ name: 'p.zip', size: 2e8, page: 'https://pixeldrain.com/u/C' }] },
];
const text = manualListText(posts, { folder: '/x/for-fs', generatedAt: 'now' });

describe('manual:open', () => {
  it('reads the posts, their pages and files back from the list the scout writes', () => {
    expect(parseManualList(text)).toEqual([
      { n: 1, title: 'Anna L. (Hegre)', url: 'https://discuss.eroscripts.com/t/1', id: 1, files: [
        { name: 'anna.mp4', page: 'https://pixeldrain.com/u/A' }, { name: 'anna SBS.mp4', page: 'https://pixeldrain.com/u/B' },
      ] },
      { n: 2, title: 'Pack 2. (part 1)', url: 'https://discuss.eroscripts.com/t/2', id: 2, files: [{ name: 'p.zip', page: 'https://pixeldrain.com/u/C' }] },
    ]);
  });

  it('skips files already saved (a browser copy suffix and another Unicode form too) and posts with nothing left', () => {
    const left = postsToOpen(parseManualList(text), ['anna (1).mp4', 'p.zip', 'anna SBS.mp4.crdownload']);
    expect(left.map((p) => [p.id, p.files.map((f) => f.name)])).toEqual([[1, ['anna SBS.mp4']]]);
    expect(withoutCopySuffix('x (12).mp4')).toBe('x.mp4');
    expect(withoutCopySuffix('(Rondoudou) KDA.mp4')).toBe('(Rondoudou) KDA.mp4');
  });

  it('opens each post next to its download pages', () => {
    expect(batchUrls(parseManualList(text))).toEqual([
      'https://discuss.eroscripts.com/t/1', 'https://pixeldrain.com/u/A', 'https://pixeldrain.com/u/B',
      'https://discuss.eroscripts.com/t/2', 'https://pixeldrain.com/u/C',
    ]);
  });
});
