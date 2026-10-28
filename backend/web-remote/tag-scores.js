// Tag scores from the phone: the user's own score per post tag
// (`library.tagScores` on the desktop; a tag without one counts 0), added to
// every video's ◆ preference score, plus Recalculate scores.
//
// Like triage, the phone never edits the library: each change is a library
// edit the desktop applies (queued while it isn't connected), and the desktop
// scores the library again after applying a tag score. `GET
// /api/remote/tag-scores` already includes edits still waiting for the
// desktop and names their tags (`pending`); `recalculating` stays true until
// the desktop has scored the library again.

import { postJson } from './triage.js';
import { t } from './i18n.js';

// Same limits as the desktop (renderer/js/preference-score.js) and the
// backend (services/library_edits.py).
export const TAG_SCORE_MAX = 10;
export const TAG_SCORE_STEP = 0.5;

/** `score` moved one step up (`direction` 1) or down (-1), on the 0.5 grid, within ±TAG_SCORE_MAX. */
export function stepTagScore(score, direction) {
  const n = (Number(score) || 0) + Math.sign(direction) * TAG_SCORE_STEP;
  const stepped = Math.round(n / TAG_SCORE_STEP) * TAG_SCORE_STEP;
  return Math.min(TAG_SCORE_MAX, Math.max(-TAG_SCORE_MAX, stepped)) + 0;
}

/** A score as shown: `0`, `+1.5`, `−2`. */
export function formatTagScore(score) {
  const n = Number(score) || 0;
  if (n === 0) return '0';
  return `${n > 0 ? '+' : '−'}${Math.abs(n)}`;
}

/** The server's tag-score state, defaults filled in. */
export function parseTagScores(data) {
  const scores = {};
  for (const [tag, score] of Object.entries(data?.scores || {})) {
    if (typeof score === 'number' && Number.isFinite(score)) scores[tag] = score;
  }
  return {
    scores,
    pending: new Set(Array.isArray(data?.pending) ? data.pending : []),
    recalculating: data?.recalculating === true,
  };
}

/** Current tag scores; throws on an HTTP or network error. */
export async function fetchTagScores(fetchImpl = globalThis.fetch) {
  const resp = await fetchImpl('/api/remote/tag-scores');
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return parseTagScores(await resp.json());
}

export function setTagScore(tag, score, fetchImpl) {
  return postJson('/api/remote/tag-score', { tag, score }, fetchImpl);
}

export function requestRecalculation(fetchImpl) {
  return postJson('/api/remote/recalculate-scores', {}, fetchImpl);
}

/**
 * Poll the tag-score state until `done(state)` holds. Resolves to that
 * state, or null when `timeoutMs` passes first. Fetch errors just wait for
 * the next poll.
 */
export async function pollTagScores(done, {
  fetchImpl = globalThis.fetch, intervalMs = 1000, timeoutMs = 20000,
  sleep = (ms) => new Promise((r) => setTimeout(r, ms)), now = () => Date.now(),
} = {}) {
  const end = now() + timeoutMs;
  while (now() < end) {
    await sleep(intervalMs);
    try {
      const state = await fetchTagScores(fetchImpl);
      if (done(state)) return state;
    } catch { /* backend busy — next poll */ }
  }
  return null;
}

/**
 * Compact −/score/+ editor for one tag. `onStep(next)` is called with the
 * new score on a tap; the caller updates the editor via `updateTagScoreEditor`.
 * Taps don't reach the row (which opens the tag's videos).
 */
export function createTagScoreEditor({ tag, score = 0, pending = false, onStep } = {}) {
  const root = document.createElement('div');
  root.className = 'tag-score';
  root.dataset.tag = tag;
  root.setAttribute('role', 'group');
  root.addEventListener('click', (e) => e.stopPropagation());

  const button = (dir, text, key) => {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = `tag-score__btn tag-score__btn--${dir > 0 ? 'up' : 'down'}`;
    b.textContent = text;
    b.setAttribute('aria-label', t(key, { tag }));
    b.addEventListener('click', () => {
      const next = stepTagScore(Number(root.dataset.score) || 0, dir);
      if (next !== (Number(root.dataset.score) || 0)) onStep?.(next);
    });
    return b;
  };
  const value = document.createElement('span');
  value.className = 'tag-score__value';
  root.append(button(-1, '−', 'webRemote.tagScores.decrease'), value, button(1, '+', 'webRemote.tagScores.increase'));
  updateTagScoreEditor(root, { score, pending });
  return root;
}

export function updateTagScoreEditor(root, { score = 0, pending = false } = {}) {
  const n = Number(score) || 0;
  root.dataset.score = String(n);
  root.classList.toggle('tag-score--pending', !!pending);
  root.classList.toggle('tag-score--positive', n > 0);
  root.classList.toggle('tag-score--negative', n < 0);
  const value = root.querySelector('.tag-score__value');
  if (value) value.textContent = formatTagScore(n);
  const label = t('webRemote.tagScores.scoreLabel', { tag: root.dataset.tag, score: formatTagScore(n) });
  root.setAttribute('aria-label', pending ? `${label} (${t('webRemote.tagScores.saving')})` : label);
  root.title = pending ? t('webRemote.tagScores.saving') : '';
  root.querySelector('.tag-score__btn--down')?.toggleAttribute('disabled', n <= -TAG_SCORE_MAX);
  root.querySelector('.tag-score__btn--up')?.toggleAttribute('disabled', n >= TAG_SCORE_MAX);
}
