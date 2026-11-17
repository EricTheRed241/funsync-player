// Review on end in the web remote (backend/web-remote/review-on-end.js):
// which ended videos get the rating sheet, and what the sheet's choices do.
import { describe, it, expect, vi } from 'vitest';
import { NEEDS_REVIEW_PLAYLIST, needsReviewPrompt, openReviewSheet } from '../../backend/web-remote/review-on-end.js';
import { NEEDS_REVIEW_PLAYLIST as DESKTOP_NAME } from '../../renderer/js/content-preferences.js';

describe('needsReviewPrompt', () => {
  it.each([
    [{ rating: 0 }, true],
    [{}, true],
    [{ rating: 0, favorite: true }, true],
    [{ rating: 0.5 }, false],
    [{ rating: 4 }, false],
    [null, false],
  ])('%j → %s', (video, expected) => {
    expect(needsReviewPrompt(video)).toBe(expected);
  });

  it('uses the desktop playlist name', () => {
    expect(NEEDS_REVIEW_PLAYLIST).toBe(DESKTOP_NAME);
  });
});

describe('openReviewSheet', () => {
  const open = (video = { rating: 0 }) => {
    const container = document.createElement('div');
    const rate = vi.fn(async () => {});
    const later = vi.fn(async () => {});
    const done = openReviewSheet({ container, video, rate, later });
    return { container, rate, later, done };
  };

  it('a star rates the video and closes the sheet', async () => {
    const { container, rate, later, done } = open();
    container.querySelectorAll('.triage-rating__star')[3].click();
    await expect(done).resolves.toBe('rated');
    expect(rate).toHaveBeenCalledWith({ rating: 4 });
    expect(later).not.toHaveBeenCalled();
    expect(container.querySelector('.review-sheet')).toBeNull();
  });

  it('the heart only toggles the favourite; the sheet stays open', async () => {
    const { container, rate, done } = open();
    container.querySelector('.triage-rating__heart').click();
    expect(rate).toHaveBeenCalledWith({ favorite: true });
    expect(container.querySelector('.review-sheet')).not.toBeNull();
    container.querySelector('.review-sheet__later').click();
    await expect(done).resolves.toBe('later');
  });

  it('Later files the video and closes the sheet', async () => {
    const { container, rate, later, done } = open();
    container.querySelector('.review-sheet__later').click();
    await expect(done).resolves.toBe('later');
    expect(later).toHaveBeenCalledTimes(1);
    expect(rate).not.toHaveBeenCalled();
    expect(container.querySelector('.review-sheet')).toBeNull();
  });
});
