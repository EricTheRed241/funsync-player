// Rating stars + favourite heart — one widget for library cards, list rows
// and the player.
//
// Five stars, half-star precision: clicking the LEFT half of a star sets
// n - 0.5, the right half sets n. Clicking the value that is already set
// clears the rating (there is no separate "clear" button to find). Clicks
// never reach the card underneath, so rating a video doesn't play it.
//
// The widget only renders and reports; the caller persists through
// `dataService.setRating` and calls `updateRatingControl` when the value
// changes (including from another surface — `rating:changed`).

import { icon, Star, Heart } from './icons.js';
import { t } from './i18n.js';
import { RATING_MAX } from './ratings.js';

/**
 * Rating chosen by a click on star `index` (0-based) at horizontal
 * `fraction` (0..1) of its width. Left half → index + 0.5, right half →
 * index + 1. Choosing the current value clears to 0.
 *
 * @param {number} index
 * @param {number} fraction
 * @param {number} [current=0]
 * @returns {number}
 */
export function ratingFromPointer(index, fraction, current = 0) {
  const i = Math.max(0, Math.min(RATING_MAX - 1, Math.floor(Number(index) || 0)));
  const value = (Number(fraction) < 0.5 ? i + 0.5 : i + 1);
  return value === current ? 0 : value;
}

/** 'full' | 'half' | 'empty' for star `index` at `rating`. */
export function starFill(index, rating) {
  const r = Number(rating) || 0;
  if (r >= index + 1) return 'full';
  if (r >= index + 0.5) return 'half';
  return 'empty';
}

/** Localised accessible label for a rating, e.g. "4.5 stars" / "Not rated". */
export function ratingLabel(rating) {
  return rating > 0 ? t('rating.stars', { n: rating }) : t('rating.unrated');
}

/**
 * Build the control.
 *
 * @param {object} o
 * @param {number} [o.rating=0]
 * @param {boolean} [o.favorite=false]
 * @param {(rating: number) => void} o.onRate
 * @param {(favorite: boolean) => void} o.onFavorite
 * @param {number} [o.size=14] — icon size in px
 * @param {string} [o.className] — extra class for placement
 * @returns {HTMLElement}
 */
export function createRatingControl({ rating = 0, favorite = false, onRate, onFavorite, size = 14, className } = {}) {
  const root = document.createElement('div');
  root.className = className ? `rating-control ${className}` : 'rating-control';
  // Nothing inside may start playback / selection on the card.
  root.addEventListener('click', (e) => e.stopPropagation());
  root.addEventListener('dblclick', (e) => e.stopPropagation());

  const heart = document.createElement('button');
  heart.type = 'button';
  heart.className = 'rating-control__heart';
  heart.appendChild(icon(Heart, { width: size, height: size }));
  heart.addEventListener('click', () => {
    onFavorite?.(!root.classList.contains('rating-control--favorite'));
  });

  const stars = document.createElement('div');
  stars.className = 'rating-control__stars';
  stars.setAttribute('role', 'group');
  for (let i = 0; i < RATING_MAX; i += 1) {
    const star = document.createElement('button');
    star.type = 'button';
    star.className = 'rating-control__star';
    star.dataset.index = String(i);
    // Outline underneath, filled copy on top clipped by the fill class.
    const fill = document.createElement('span');
    fill.className = 'rating-control__fill';
    fill.appendChild(icon(Star, { width: size, height: size }));
    star.append(icon(Star, { width: size, height: size }), fill);
    star.addEventListener('click', (e) => {
      const rect = star.getBoundingClientRect();
      const fraction = rect.width > 0 ? (e.clientX - rect.left) / rect.width : 1;
      // Keyboard activation has no pointer position: whole star.
      const value = ratingFromPointer(i, e.detail === 0 ? 1 : fraction, Number(root.dataset.rating) || 0);
      onRate?.(value);
    });
    stars.appendChild(star);
  }

  root.append(stars, heart);
  updateRatingControl(root, { rating, favorite });
  return root;
}

/** Redraw an existing control for a new value. */
export function updateRatingControl(root, { rating = 0, favorite = false } = {}) {
  if (!root) return;
  const r = Number(rating) || 0;
  root.dataset.rating = String(r);
  root.classList.toggle('rating-control--rated', r > 0);
  root.classList.toggle('rating-control--favorite', !!favorite);
  const label = ratingLabel(r);
  const stars = root.querySelector('.rating-control__stars');
  if (stars) {
    stars.setAttribute('aria-label', label);
    stars.title = label;
  }
  root.querySelectorAll('.rating-control__star').forEach((star, i) => {
    star.dataset.fill = starFill(i, r);
    star.setAttribute('aria-label', t('rating.setStars', { n: i + 1 }));
  });
  const heart = root.querySelector('.rating-control__heart');
  if (heart) {
    const text = favorite ? t('rating.unfavorite') : t('rating.favorite');
    heart.title = text;
    heart.setAttribute('aria-label', text);
    heart.setAttribute('aria-pressed', favorite ? 'true' : 'false');
  }
}
