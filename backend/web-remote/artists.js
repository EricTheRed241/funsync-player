// Favourite artists from the phone: every artist of the library (one per
// artist folder) plus favourites with no videos, a star per artist, and the
// favourite-artist bonus: every video of a favourite gets it on top of its ◆
// preference score, and the scouts rank their posts higher.
//
// Like tag scores, the phone never edits the library: a star or a bonus is a
// library edit the desktop applies (queued while it isn't connected), and the
// desktop scores the library again after applying it. `GET
// /api/remote/artists` already includes edits still waiting for the desktop
// and flags them (`pending`, `scorePending`).

import { postJson } from './triage.js';
import { t } from './i18n.js';
import { svgIcon } from './icons.js';

/** An artist name as compared: NFC, trimmed, lower case (as the desktop does). */
export function foldArtist(name) {
  return String(name || '').normalize('NFC').trim().toLocaleLowerCase();
}

const finite = (n) => typeof n === 'number' && Number.isFinite(n);

/** The server's artist state, defaults filled in and entries without a name dropped. */
export function parseArtists(data) {
  const artists = [];
  for (const a of Array.isArray(data?.artists) ? data.artists : []) {
    const name = typeof a?.name === 'string' ? a.name.trim() : '';
    if (!name) continue;
    const videoIds = Array.isArray(a.videoIds) ? a.videoIds.filter((id) => typeof id === 'string') : [];
    artists.push({
      id: typeof a.id === 'string' && a.id ? a.id : foldArtist(name),
      name,
      videoCount: finite(a.videoCount) ? a.videoCount : videoIds.length,
      totalDuration: finite(a.totalDuration) ? a.totalDuration : 0,
      videoIds,
      favourite: a.favourite === true,
      pending: a.pending === true,
    });
  }
  const defaultScore = finite(data?.defaultScore) ? data.defaultScore : 0;
  return {
    artists,
    score: finite(data?.score) ? data.score : defaultScore,
    defaultScore,
    scorePending: data?.scorePending === true,
  };
}

/** Favourites first, then by name (case-insensitive). A new array. */
export function sortArtists(artists) {
  return [...artists].sort((a, b) => (b.favourite - a.favourite)
    || a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })
    || a.name.localeCompare(b.name));
}

/**
 * The artists as shown: the server's, with the stars still on their way to
 * the server (`unsent`: Map fold → {name, favourite}) applied and pending.
 * A new favourite the server doesn't know yet is added without videos.
 */
export function mergeArtistEdits(artists, unsent = new Map()) {
  const seen = new Set();
  const out = artists.map((a) => {
    const key = foldArtist(a.name);
    seen.add(key);
    const edit = unsent.get(key);
    return edit ? { ...a, favourite: edit.favourite, pending: true } : a;
  });
  for (const [key, edit] of unsent) {
    if (seen.has(key) || !edit.favourite) continue;
    out.push({ id: key, name: edit.name, videoCount: 0, totalDuration: 0, videoIds: [], favourite: true, pending: true });
  }
  return sortArtists(out);
}

/**
 * What changed between two states: names (folded) whose star the desktop
 * has applied since, and whether the bonus was applied.
 */
export function appliedArtistEdits(prev, next) {
  const stillPending = new Set(next.artists.filter((a) => a.pending).map((a) => foldArtist(a.name)));
  const artists = prev.artists.filter((a) => a.pending).map((a) => foldArtist(a.name))
    .filter((key) => !stillPending.has(key));
  return { artists, score: prev.scorePending && !next.scorePending };
}

/** True while any star or the bonus waits for the desktop. */
export function artistsPending(state) {
  return state.scorePending || state.artists.some((a) => a.pending);
}

/** Current artists; throws on an HTTP or network error. */
export async function fetchArtists(fetchImpl = globalThis.fetch) {
  const resp = await fetchImpl('/api/remote/artists');
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return parseArtists(await resp.json());
}

export function setFavouriteArtist(artist, favourite, fetchImpl) {
  return postJson('/api/remote/favourite-artist', { artist, favourite: !!favourite }, fetchImpl);
}

export function setFavouriteScore(score, fetchImpl) {
  return postJson('/api/remote/favourite-score', { score }, fetchImpl);
}

/**
 * Poll the artist state until `done(state)` holds. Resolves to that state,
 * or null when `timeoutMs` passes first. Fetch errors just wait for the
 * next poll.
 */
export async function pollArtists(done, {
  fetchImpl = globalThis.fetch, intervalMs = 1000, timeoutMs = 20000,
  sleep = (ms) => new Promise((r) => setTimeout(r, ms)), now = () => Date.now(),
} = {}) {
  const end = now() + timeoutMs;
  while (now() < end) {
    await sleep(intervalMs);
    try {
      const state = await fetchArtists(fetchImpl);
      if (done(state)) return state;
    } catch { /* backend busy — next poll */ }
  }
  return null;
}

/**
 * Star toggle for one artist. `onToggle(next)` is called with the new
 * favourite state on a tap; the caller updates the star via
 * `updateFavouriteStar`. Taps don't reach the row (which opens the artist).
 */
export function createFavouriteStar({ artist, favourite = false, pending = false, onToggle } = {}) {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'artist-star';
  button.dataset.artist = artist;
  button.appendChild(svgIcon('star', 22));
  button.addEventListener('click', (e) => {
    e.stopPropagation();
    onToggle?.(button.getAttribute('aria-pressed') !== 'true');
  });
  updateFavouriteStar(button, { favourite, pending });
  return button;
}

export function updateFavouriteStar(button, { favourite = false, pending = false } = {}) {
  const artist = button.dataset.artist;
  button.setAttribute('aria-pressed', favourite ? 'true' : 'false');
  button.classList.toggle('artist-star--on', !!favourite);
  button.classList.toggle('artist-star--pending', !!pending);
  const label = t(favourite ? 'webRemote.artists.unfavourite' : 'webRemote.artists.favourite', { artist });
  button.setAttribute('aria-label', pending ? `${label} (${t('webRemote.tagScores.saving')})` : label);
  button.title = pending ? t('webRemote.tagScores.saving') : '';
}
