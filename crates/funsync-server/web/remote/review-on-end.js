// Review on end: when the phone's own player reaches the end of an unrated
// video, a rating sheet covers the player before the Up Next card. Rating
// it sends the rating; "Later" files the video under the Needs review
// playlist (which the desktop empties as videos get rated, and HereSphere
// lists first). Either way the player then goes on to Up Next. Rated videos
// get no sheet.

import { createRatingControl, updateRatingControl } from './triage.js';
import { t } from './i18n.js';

/**
 * Same name as NEEDS_REVIEW_PLAYLIST in renderer/js/content-preferences.js
 * and backend/services/vr_sections.py (the phone can't import either).
 */
export const NEEDS_REVIEW_PLAYLIST = 'Needs review';

/** True when an ended video should get the rating sheet: no rating (> 0) yet. */
export function needsReviewPrompt(video) {
  return !!video && !(Number(video.rating) > 0);
}

/**
 * Show the rating sheet over `container` and resolve once the user rated
 * the video (`'rated'`) or chose Later (`'later'`). The heart only toggles
 * the favourite; the sheet stays until a rating or Later.
 *
 * @param {object} o
 * @param {HTMLElement} o.container
 * @param {{rating?: number, favorite?: boolean}} o.video
 * @param {(change: {rating?: number, favorite?: boolean}) => Promise<unknown>} o.rate
 * @param {() => Promise<unknown>} o.later — file the video under Needs review
 * @returns {Promise<'rated'|'later'>}
 */
export function openReviewSheet({ container, video, rate, later }) {
  return new Promise((resolve) => {
    const sheet = document.createElement('div');
    sheet.className = 'review-sheet';
    const title = document.createElement('div');
    title.className = 'review-sheet__title';
    title.textContent = t('webRemote.review.title');

    let favorite = !!video.favorite;
    let done = false;
    const finish = (outcome) => {
      if (done) return;
      done = true;
      sheet.remove();
      resolve(outcome);
    };
    const control = createRatingControl({
      rating: 0,
      favorite,
      onRate: (rating) => {
        if (!(rating > 0) || done) return;
        updateRatingControl(control, { rating, favorite });
        Promise.resolve(rate({ rating })).finally(() => finish('rated'));
      },
      onFavorite: (next) => {
        favorite = next;
        updateRatingControl(control, { rating: 0, favorite });
        rate({ favorite: next });
      },
    });

    const laterBtn = document.createElement('button');
    laterBtn.type = 'button';
    laterBtn.className = 'review-sheet__later';
    laterBtn.textContent = t('webRemote.review.later');
    laterBtn.addEventListener('click', () => {
      if (done) return;
      laterBtn.disabled = true;
      Promise.resolve(later()).finally(() => finish('later'));
    });

    sheet.append(title, control, laterBtn);
    container.appendChild(sheet);
  });
}
