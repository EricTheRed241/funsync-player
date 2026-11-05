// Queue panel: the shared queue's auto-filled items — their place among the
// sections and the skip action.
import { describe, it, expect, vi } from 'vitest';
import { QueuePanel } from '../../renderer/components/queue-panel.js';

function openPanel(state, opts = {}) {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const panel = new QueuePanel({ element: el, ...opts });
  panel.open();
  panel.setState(state);
  return el;
}

const sectionKinds = (el) => [...el.querySelectorAll('.queue-panel__section')]
  .map((s) => [...s.classList].find((c) => c.startsWith('queue-panel__section--')).slice('queue-panel__section--'.length));
const rowPaths = (el, kind) => [...el.querySelectorAll(`[data-queue-kind="${kind}"]`)].map((r) => r.dataset.queuePath);

describe('QueuePanel auto-filled section', () => {
  const base = {
    nowPlaying: '/now.mp4', userQueue: ['/mine.mp4'], autoQueue: ['/auto1.mp4', '/auto2.mp4'],
    upcoming: ['/lib1.mp4', '/lib2.mp4'], hasContext: true,
  };

  it('follows your own queue, in order, ahead of the library next', () => {
    const el = openPanel(base);
    expect(sectionKinds(el)).toEqual(['now-playing', 'user-queue', 'auto-queue', 'upcoming']);
    expect(rowPaths(el, 'auto-queue')).toEqual(['/auto1.mp4', '/auto2.mp4']);
  });

  it('comes after a Play All / playlist list, which plays out first', () => {
    const el = openPanel({ ...base, autoAfterUpcoming: true });
    expect(sectionKinds(el)).toEqual(['now-playing', 'user-queue', 'upcoming', 'auto-queue']);
  });

  it('skip removes the item without playing it; a row click plays it', () => {
    const onRemoveFromQueue = vi.fn();
    const onJump = vi.fn();
    const el = openPanel(base, { onRemoveFromQueue, onJump });
    const row = el.querySelector('[data-queue-kind="auto-queue"][data-queue-path="/auto2.mp4"]');
    row.querySelector('[data-queue-remove]').click();
    expect(onRemoveFromQueue).toHaveBeenCalledWith('/auto2.mp4');
    expect(onJump).not.toHaveBeenCalled();
    row.click();
    expect(onJump).toHaveBeenCalledWith('/auto2.mp4');
  });

  it('auto-filled items alone keep the panel out of its empty state', () => {
    const el = openPanel({ autoQueue: ['/auto1.mp4'], hasContext: false });
    expect(sectionKinds(el)).toEqual(['auto-queue']);
  });
});
