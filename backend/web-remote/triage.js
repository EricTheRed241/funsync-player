// Triage from the phone: rating (half stars), favourite, add to / remove
// from a playlist.
//
// The phone never edits the library itself. Every write goes to the
// backend (`routes/remote.py`), which hands it to the desktop app as a
// library edit — queued while the desktop isn't connected and applied when
// it reconnects. A response with `delivered: false` is a success ("saved,
// applies when the desktop app reconnects"), not an error.

import { svgIcon } from './icons.js';
import { t } from './i18n.js';

export const RATING_MAX = 5;

/**
 * Rating chosen by a tap on star `index` (0-based) at horizontal
 * `fraction` (0..1) of its width: left half = index + 0.5, right half =
 * index + 1. Tapping the current value clears it to 0.
 */
export function ratingFromTap(index, fraction, current = 0) {
  const i = Math.max(0, Math.min(RATING_MAX - 1, Math.floor(Number(index) || 0)));
  const value = Number(fraction) < 0.5 ? i + 0.5 : i + 1;
  return value === current ? 0 : value;
}

/** 'full' | 'half' | 'empty' for star `index` at `rating`. */
export function starFill(index, rating) {
  const r = Number(rating) || 0;
  if (r >= index + 1) return 'full';
  if (r >= index + 0.5) return 'half';
  return 'empty';
}

/** Library rating filter: 'favorites' | 'top' (>= 4) | 'unrated' | 'all'. */
export function filterByRating(videos, filter) {
  const list = Array.isArray(videos) ? videos : [];
  switch (filter) {
    case 'favorites': return list.filter((v) => v.favorite === true);
    case 'top': return list.filter((v) => (v.rating || 0) >= 4);
    case 'unrated': return list.filter((v) => !(v.rating > 0));
    default: return list;
  }
}

/** Request bodies, kept pure so they're testable. */
export function ratingBody(videoId, change) {
  const body = { videoId };
  if (change && change.rating !== undefined) body.rating = change.rating;
  if (change && change.favorite !== undefined) body.favorite = !!change.favorite;
  return body;
}

export function playlistBody(videoId, { playlistId, name } = {}) {
  const body = { videoId };
  if (playlistId) body.playlistId = playlistId;
  const trimmed = typeof name === 'string' ? name.trim() : '';
  if (trimmed) body.name = trimmed;
  return body;
}

/** POST JSON; resolves `{ok, status, data}` and never throws on HTTP errors. */
export async function postJson(url, body, fetchImpl = globalThis.fetch) {
  try {
    const resp = await fetchImpl(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    let data = null;
    try { data = await resp.json(); } catch { /* empty body */ }
    return { ok: resp.ok, status: resp.status, data };
  } catch (err) {
    return { ok: false, status: 0, data: { detail: err?.message || String(err) } };
  }
}

export function setRating(videoId, change, fetchImpl) {
  return postJson('/api/remote/rating', ratingBody(videoId, change), fetchImpl);
}

export function addToPlaylist(videoId, target, fetchImpl) {
  return postJson('/api/remote/playlists/add-video', playlistBody(videoId, target), fetchImpl);
}

export function removeFromPlaylist(videoId, target, fetchImpl) {
  return postJson('/api/remote/playlists/remove-video', playlistBody(videoId, target), fetchImpl);
}

/**
 * Toast for a write's outcome: queued → "applies when the desktop
 * reconnects", error → the server's reason. Successful live writes stay
 * quiet unless `successText` is given.
 */
export function reportResult(result, successText) {
  if (!result.ok) {
    const detail = result.data?.detail;
    showToast(t('webRemote.triage.failed', { reason: typeof detail === 'string' ? detail : `HTTP ${result.status}` }), 'error');
  } else if (result.data?.delivered === false) {
    showToast(t('webRemote.triage.queued'), 'info');
  } else if (successText) {
    showToast(successText, 'success');
  }
}

let toastTimer = null;

/** One transient message at the bottom of the screen. */
export function showToast(message, kind = 'info') {
  let el = document.getElementById('triageToast');
  if (!el) {
    el = document.createElement('div');
    el.id = 'triageToast';
    el.className = 'triage-toast';
    el.setAttribute('role', 'status');
    el.setAttribute('aria-live', 'polite');
    document.body.appendChild(el);
  }
  el.textContent = message;
  el.dataset.kind = kind;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, 3500);
}

/**
 * Stars + heart. `onRate(value)` / `onFavorite(next)` are called on tap;
 * the caller updates the control (optimistically) via `updateRatingControl`.
 */
export function createRatingControl({ rating = 0, favorite = false, onRate, onFavorite } = {}) {
  const root = document.createElement('div');
  root.className = 'triage-rating';
  root.addEventListener('click', (e) => e.stopPropagation());

  const stars = document.createElement('div');
  stars.className = 'triage-rating__stars';
  stars.setAttribute('role', 'group');
  for (let i = 0; i < RATING_MAX; i += 1) {
    const star = document.createElement('button');
    star.type = 'button';
    star.className = 'triage-rating__star';
    const fill = document.createElement('span');
    fill.className = 'triage-rating__fill';
    fill.appendChild(svgIcon('star', 28));
    star.append(svgIcon('star', 28), fill);
    star.addEventListener('click', (e) => {
      const rect = star.getBoundingClientRect();
      const fraction = e.detail === 0 || rect.width === 0 ? 1 : (e.clientX - rect.left) / rect.width;
      onRate?.(ratingFromTap(i, fraction, Number(root.dataset.rating) || 0));
    });
    stars.appendChild(star);
  }

  const heart = document.createElement('button');
  heart.type = 'button';
  heart.className = 'triage-rating__heart';
  heart.appendChild(svgIcon('heart', 26));
  heart.addEventListener('click', () => {
    onFavorite?.(!root.classList.contains('triage-rating--favorite'));
  });

  root.append(stars, heart);
  updateRatingControl(root, { rating, favorite });
  return root;
}

export function updateRatingControl(root, { rating = 0, favorite = false } = {}) {
  const r = Number(rating) || 0;
  root.dataset.rating = String(r);
  root.classList.toggle('triage-rating--favorite', !!favorite);
  const stars = root.querySelector('.triage-rating__stars');
  const label = r > 0 ? t('webRemote.triage.stars', { n: r }) : t('webRemote.triage.unrated');
  stars?.setAttribute('aria-label', label);
  root.querySelectorAll('.triage-rating__star').forEach((star, i) => {
    star.dataset.fill = starFill(i, r);
    star.setAttribute('aria-label', t('webRemote.triage.setStars', { n: i + 1 }));
  });
  const heart = root.querySelector('.triage-rating__heart');
  if (heart) {
    const text = favorite ? t('webRemote.triage.unfavorite') : t('webRemote.triage.favorite');
    heart.setAttribute('aria-label', text);
    heart.setAttribute('aria-pressed', favorite ? 'true' : 'false');
  }
}
