// The look of a watched video where watched videos sink (watched-lists.js):
// its card or row is dimmed (`is-watched`) and carries a small "Watched"
// badge. Shared by the library, categories and playlists views.

import { t } from './i18n.js';

/**
 * Dim `el` and append the badge to `parent` (default `el`).
 *
 * @param {HTMLElement} el card or row
 * @param {number} rating the video's rating (> 0), for the tooltip
 * @param {HTMLElement} [parent]
 */
export function markWatched(el, rating, parent = el) {
  el.classList.add('is-watched');
  const badge = document.createElement('span');
  badge.className = 'watched-badge';
  badge.textContent = t('rating.watched');
  badge.title = t('rating.watchedTitle', { n: rating });
  parent.appendChild(badge);
}
