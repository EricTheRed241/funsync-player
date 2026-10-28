// Library edits from the backend — ratings, favourites and playlist changes
// made in HereSphere or on the web remote, plus the web remote's tag scores
// and Recalculate scores.
//
// The desktop owns the library (config.json is written only by Electron), so
// the backend never applies these itself. It queues each one and sends it
// down the observer socket as `{type: 'library-edit', edit}`; this module
// applies it through the data service (which persists it and emits the
// usual `rating:changed` / `playlist:changed` events, so every open view
// updates live), re-registers ratings + playlists with the backend, and only
// then acks `{type: 'library-edit-ack', seq}`. Edits are applied strictly in
// arrival order — `apply` chains them — because "add to new playlist X" must
// finish creating X before the next edit can find it.
//
// Every edit is idempotent, so the backend replaying an edit whose ack was
// lost (desktop closed mid-flight) does no harm. Protocol and queue:
// backend/services/library_edits.py.

import { normalizeTagScore, tagKey } from './preference-score.js';
import { TAG_SCORES_KEY } from './library-scores.js';

const KIND_RATING = 'rating';
const KIND_PLAYLIST_ADD = 'playlist-add';
const KIND_PLAYLIST_REMOVE = 'playlist-remove';
const KIND_TAG_SCORE = 'tag-score';
const KIND_RECALCULATE_SCORES = 'recalculate-scores';
const PENDING_ID_PREFIX = 'pending:';
const VIDEO_KINDS = new Set([KIND_RATING, KIND_PLAYLIST_ADD, KIND_PLAYLIST_REMOVE]);

function sameName(a, b) {
  return String(a || '').trim().toLocaleLowerCase() === String(b || '').trim().toLocaleLowerCase();
}

/**
 * The playlist an edit targets: by id, else by (case-insensitive) name.
 * A backend placeholder id (`pending:<name>`) resolves by name.
 */
export function findPlaylist(playlists, playlistId, name) {
  let id = playlistId;
  let wanted = name;
  if (typeof id === 'string' && id.startsWith(PENDING_ID_PREFIX)) {
    wanted = wanted || id.slice(PENDING_ID_PREFIX.length);
    id = null;
  }
  const list = Array.isArray(playlists) ? playlists : [];
  if (id) {
    const byId = list.find((p) => p.id === id);
    if (byId) return byId;
  }
  if (wanted && String(wanted).trim()) {
    return list.find((p) => sameName(p.name, wanted)) || null;
  }
  return null;
}

/**
 * Store a tag score in `library.tagScores` (tag → score; 0 removes the tag).
 * Returns `{tag, score}` as stored; throws on a missing tag or a score that
 * isn't a number.
 *
 * @param {object} data DataService (get, set)
 */
export function setTagScore(data, tag, score) {
  const key = tagKey(tag);
  if (!key) throw new Error('tag score without a tag');
  const value = normalizeTagScore(score);
  if (value === null) throw new Error(`invalid tag score ${JSON.stringify(score)}`);
  const current = data.get(TAG_SCORES_KEY) || {};
  if (value === 0 ? !Object.hasOwn(current, key) : current[key] === value) return { tag: key, score: value };
  const next = { ...current };
  if (value === 0) delete next[key];
  else next[key] = value;
  data.set(TAG_SCORES_KEY, next);
  return { tag: key, score: value };
}

/**
 * Apply one edit through `data` (the DataService). Resolves to a short
 * description for the log; throws when the edit can't be applied.
 *
 * @param {object} edit
 * @param {object} data — getPlaylists/addPlaylist/addVideoToPlaylist/removeVideoFromPlaylist/setRating/get/set
 * @param {{recalculateScores?: () => Promise<object|null>}} [hooks] — score the
 *   library again (and store it); run after a tag score and on request
 */
export async function applyLibraryEdit(edit, data, { recalculateScores } = {}) {
  if (!edit) throw new Error('empty edit');
  if (VIDEO_KINDS.has(edit.kind) && (typeof edit.path !== 'string' || !edit.path)) throw new Error('edit without a video path');
  switch (edit.kind) {
    case KIND_TAG_SCORE: {
      const { tag, score } = setTagScore(data, edit.tag, edit.score);
      const r = recalculateScores ? await recalculateScores() : null;
      return `tag score "${tag}" = ${score}${r ? `, ${r.videos} videos scored again` : ''}`;
    }
    case KIND_RECALCULATE_SCORES: {
      const r = recalculateScores ? await recalculateScores() : null;
      return r ? `${r.videos} videos scored again${r.changed ? '' : ' (no change)'}` : 'scores not recalculated: library not scanned yet';
    }
    case KIND_RATING: {
      const change = {};
      if (edit.rating !== null && edit.rating !== undefined) change.rating = edit.rating;
      if (edit.favorite !== null && edit.favorite !== undefined) change.favorite = edit.favorite === true;
      data.setRating(edit.path, change);
      return `rating ${JSON.stringify(change)}`;
    }
    case KIND_PLAYLIST_ADD: {
      let playlist = findPlaylist(data.getPlaylists(), edit.playlistId, edit.name);
      if (!playlist) {
        const fromId = typeof edit.playlistId === 'string' && edit.playlistId.startsWith(PENDING_ID_PREFIX)
          ? edit.playlistId.slice(PENDING_ID_PREFIX.length) : '';
        const name = String(edit.name || fromId).trim();
        if (!name) throw new Error('playlist not found and no name to create it with');
        playlist = await data.addPlaylist(name);
      }
      if (!(playlist.videoPaths || []).includes(edit.path)) {
        data.addVideoToPlaylist(playlist.id, edit.path);
      }
      return `added to playlist "${playlist.name}"`;
    }
    case KIND_PLAYLIST_REMOVE: {
      const playlist = findPlaylist(data.getPlaylists(), edit.playlistId, edit.name);
      if (playlist && (playlist.videoPaths || []).includes(edit.path)) {
        data.removeVideoFromPlaylist(playlist.id, edit.path);
        return `removed from playlist "${playlist.name}"`;
      }
      return 'already not in the playlist';
    }
    default:
      throw new Error(`unknown edit kind ${JSON.stringify(edit.kind)}`);
  }
}

/**
 * Serial applier: `apply(edit)` queues behind every earlier edit.
 *
 * @param {object} o
 * @param {object} o.data — DataService
 * @param {() => Promise<void>} [o.publish] — re-register ratings, playlists and scores
 * @param {(seq: number, error?: string) => void} o.ack
 * @param {() => Promise<object|null>} [o.recalculateScores] — see applyLibraryEdit
 */
export class LibraryEditApplier {
  constructor({ data, publish, ack, recalculateScores }) {
    this._data = data;
    this._publish = publish || (async () => {});
    this._ack = ack;
    this._hooks = { recalculateScores };
    this._chain = Promise.resolve();
  }

  /** Queue `edit`; the returned promise settles once it is acked. */
  apply(edit) {
    this._chain = this._chain.then(() => this._applyOne(edit));
    return this._chain;
  }

  async _applyOne(edit) {
    const seq = edit?.seq;
    let error;
    try {
      const what = await applyLibraryEdit(edit, this._data, this._hooks);
      console.log(`[LibraryEdits] #${seq} from ${edit.source || '?'}: ${what}`);
    } catch (err) {
      error = err?.message || String(err);
      console.warn(`[LibraryEdits] #${seq} rejected: ${error}`);
    }
    // Backend state first, then the ack, so the edit never vanishes from the
    // headset / phone between leaving the queue and the registration landing.
    try { await this._publish(); } catch { /* ack anyway — the backend folds it in */ }
    if (Number.isInteger(seq)) this._ack(seq, error);
  }
}
