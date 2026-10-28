"""Library edits that originate in the backend, on their way to the desktop.

The desktop owns the library: `config.json` is written only by Electron, and
the backend learns state from `POST /api/media/register` pushes. A rating,
favourite or playlist change made in HereSphere or on the web remote therefore
cannot be stored here. It becomes an EDIT:

    {"seq": 7, "kind": "rating", "path": "/v/a.mp4",
     "rating": 4.5, "favorite": true, "source": "heresphere", "ts": ...}
    {"seq": 8, "kind": "playlist-add", "path": "/v/a.mp4",
     "playlistId": "pl-1" | None, "name": "Triage", ...}
    {"seq": 9, "kind": "playlist-remove", "path": "/v/a.mp4",
     "playlistId": "pl-1" | None, "name": "Triage", ...}
    {"seq": 10, "kind": "tag-score", "tag": "riding", "score": 1.5, ...}
    {"seq": 11, "kind": "recalculate-scores", ...}
    {"seq": 12, "kind": "favourite-artist", "artist": "Mira", "favourite": true, ...}
    {"seq": 13, "kind": "favourite-score", "score": 5.0, ...}

A tag score is the user's weight for a post tag; the backend holds every
tag's score as the desktop registers it (starting scores included, absent =
0), added to every scene's preference score; a recalculation asks the
desktop to score the library again now (renderer/js/library-scores.js).
A favourite-artist edit marks an artist (matched case-insensitively, see
`artist_key`) as a favourite or not; a favourite-score edit sets the bonus a
favourite artist's scenes get (renderer/js/favourite-artists.js). None of
these names a video.

Edits wait in one ordered queue (`queue`). The renderer's observer socket
(`routes/remote_sync.py`) receives each as `{"type": "library-edit", "edit":
...}`, applies it through the data service (which persists it), re-registers
ratings + playlists, and answers `{"type": "library-edit-ack", "seq": n}`.
Only the ack removes an edit. While the desktop is not connected edits simply
stay queued, and every reconnect replays the whole queue in order, so nothing
made while the desktop was down is lost.

Every edit is idempotent (set a rating, add-if-missing, remove-if-present,
create-by-name-if-missing, set a tag score, recalculate, set or clear a
favourite artist, set the favourite bonus), so a replay after a lost ack is
harmless.

The queue is mirrored to `<userData>/pending-library-edits.json` when the
backend knows the userData dir, so it also survives a backend restart. That
file is the backend's own; `config.json` is never touched from Python.

Rejection: the queue holds at most MAX_PENDING edits. Past that, `enqueue`
raises QueueFull and the HTTP layer answers 503 — a desktop that has been
away for that long should not be buried in a backlog nobody remembers making.

Until the desktop has applied an edit, readers see it anyway: `effective_*`
overlay the pending edits on the last registered state, so the headset's
sections and the phone's lists reflect a change the moment it is made.
"""

import copy
import json
import logging
import math
import os
import threading
import time
import unicodedata

log = logging.getLogger("funsync.library_edits")

KIND_RATING = "rating"
KIND_PLAYLIST_ADD = "playlist-add"
KIND_PLAYLIST_REMOVE = "playlist-remove"
KIND_TAG_SCORE = "tag-score"
KIND_RECALCULATE_SCORES = "recalculate-scores"
KIND_FAVOURITE_ARTIST = "favourite-artist"
KIND_FAVOURITE_SCORE = "favourite-score"
KINDS = (
    KIND_RATING, KIND_PLAYLIST_ADD, KIND_PLAYLIST_REMOVE, KIND_TAG_SCORE, KIND_RECALCULATE_SCORES,
    KIND_FAVOURITE_ARTIST, KIND_FAVOURITE_SCORE,
)

MAX_PENDING = 1000
RATING_MAX = 5
RATING_STEP = 0.5
# Tag scores: -10 … +10 in steps of 0.5 (renderer/js/preference-score.js
# TAG_SCORE_MAX / TAG_SCORE_STEP).
TAG_SCORE_MAX = 10
TAG_SCORE_STEP = 0.5
PENDING_ID_PREFIX = "pending:"
QUEUE_FILE = "pending-library-edits.json"


class QueueFull(Exception):
    """The desktop has not drained the queue and it reached MAX_PENDING."""


def normalize_rating(value):
    """`value` as a valid rating (0–5 in 0.5 steps), else None.

    Same rule as `renderer/js/ratings.js::normalizeRating`: nothing is
    rounded or clamped, an off-step or out-of-range value is rejected.
    """
    if isinstance(value, bool) or value is None:
        return None
    if isinstance(value, str):
        try:
            value = float(value.strip())
        except ValueError:
            return None
    if not isinstance(value, (int, float)):
        return None
    value = float(value)
    if value != value or value < 0 or value > RATING_MAX:  # NaN fails the first test
        return None
    if (value / RATING_STEP) != int(value / RATING_STEP):
        return None
    return value


def rating_entry(ratings, path):
    """`(rating, favorite)` for `path` from a ratings map; `(0.0, False)` when absent."""
    raw = (ratings or {}).get(path) if path else None
    if not isinstance(raw, dict):
        return 0.0, False
    return normalize_rating(raw.get("rating")) or 0.0, raw.get("favorite") is True


def tag_key(tag):
    """The key a tag's score is stored under: trimmed, lower case (as
    `renderer/js/preference-score.js::tagKey`); '' for a non-string."""
    return tag.strip().lower() if isinstance(tag, str) else ""


def normalize_tag_score(value):
    """`value` as a tag score, else None.

    Same rule as `renderer/js/preference-score.js::normalizeTagScore`: a
    finite number (or numeric string) rounded to the nearest 0.5 (halves up)
    and clamped to -10 … +10.
    """
    if isinstance(value, bool) or not isinstance(value, (int, float, str)):
        return None
    try:
        value = float(value.strip() if isinstance(value, str) else value)
    except (ValueError, OverflowError):
        return None
    if not math.isfinite(value):
        return None
    stepped = math.floor(value / TAG_SCORE_STEP + 0.5) * TAG_SCORE_STEP
    return min(TAG_SCORE_MAX, max(-TAG_SCORE_MAX, stepped)) + 0.0


def clean_tag_scores(scores):
    """A `{tag: score}` map with valid keys and scores only; zeros dropped."""
    out = {}
    for tag, raw in (scores.items() if isinstance(scores, dict) else ()):
        key, score = tag_key(tag), normalize_tag_score(raw)
        if key and score:
            out[key] = score
    return out


def artist_key(name):
    """The key two spellings of an artist compare by: NFC, trimmed, case
    folded (as `renderer/js/favourite-artists.js::fold`); '' for a
    non-string."""
    return unicodedata.normalize("NFC", name).strip().casefold() if isinstance(name, str) else ""


def _same_name(a, b):
    return (a or "").strip().casefold() == (b or "").strip().casefold()


def find_playlist(playlists, playlist_id=None, name=None):
    """The playlist matching `playlist_id`, else the one named `name`
    (case-insensitive), else None. A `pending:<name>` id resolves by name."""
    if playlist_id and str(playlist_id).startswith(PENDING_ID_PREFIX):
        name = name or str(playlist_id)[len(PENDING_ID_PREFIX):]
        playlist_id = None
    if playlist_id:
        for p in playlists:
            if p.get("id") == playlist_id:
                return p
    if name and name.strip():
        for p in playlists:
            if _same_name(p.get("name"), name):
                return p
    return None


def apply_to_ratings(ratings, edit):
    """Apply one rating edit to a ratings map in place."""
    path = edit["path"]
    rating, favorite = rating_entry(ratings, path)
    if edit.get("rating") is not None:
        rating = edit["rating"]
    if edit.get("favorite") is not None:
        favorite = bool(edit["favorite"])
    if rating == 0 and not favorite:
        ratings.pop(path, None)
    else:
        ratings[path] = {"rating": rating, "favorite": favorite, "updatedAt": edit.get("ts") or 0}


def apply_to_playlists(playlists, edit):
    """Apply one playlist edit to a playlist list in place. An add naming a
    playlist that doesn't exist yet creates a `pending: True` placeholder."""
    path = edit["path"]
    target = find_playlist(playlists, edit.get("playlistId"), edit.get("name"))
    if edit["kind"] == KIND_PLAYLIST_ADD:
        if target is None:
            name = (edit.get("name") or "").strip()
            if not name:
                return
            target = {
                "id": PENDING_ID_PREFIX + name,
                "name": name,
                "videoPaths": [],
                "createdAt": edit.get("ts") or 0,
                "pending": True,
            }
            playlists.append(target)
        paths = target.setdefault("videoPaths", [])
        if path not in paths:
            paths.append(path)
    elif edit["kind"] == KIND_PLAYLIST_REMOVE and target is not None:
        target["videoPaths"] = [p for p in target.get("videoPaths") or [] if p != path]


def apply_to_tag_scores(scores, edit):
    """Apply one tag-score edit to a `{tag: score}` map in place; 0 removes
    the tag."""
    key = tag_key(edit.get("tag"))
    score = normalize_tag_score(edit.get("score"))
    if not key or score is None:
        return
    if score == 0:
        scores.pop(key, None)
    else:
        scores[key] = score


def apply_to_favourite_artists(names, edit):
    """Apply one favourite-artist edit to a list of favourite names in place:
    every spelling of the artist goes, and a favourite comes back (last) as
    the edit spells it."""
    artist = edit.get("artist")
    key = artist_key(artist)
    favourite = edit.get("favourite")
    if not key or not isinstance(favourite, bool):
        return
    names[:] = [n for n in names if artist_key(n) != key]
    if favourite:
        names.append(artist.strip())


def effective_ratings(base, pending):
    """The registered ratings with the pending rating edits applied."""
    out = dict(base or {})
    for edit in pending:
        if edit["kind"] == KIND_RATING:
            apply_to_ratings(out, edit)
    return out


def effective_playlists(base, pending):
    """The registered playlists with the pending playlist edits applied."""
    edits = [e for e in pending if e["kind"] in (KIND_PLAYLIST_ADD, KIND_PLAYLIST_REMOVE)]
    if not edits:
        return list(base or [])
    out = copy.deepcopy(list(base or []))
    for edit in edits:
        apply_to_playlists(out, edit)
    return out


def effective_tag_scores(base, pending):
    """The registered tag scores with the pending tag-score edits applied."""
    out = dict(base or {})
    for edit in pending:
        if edit["kind"] == KIND_TAG_SCORE:
            apply_to_tag_scores(out, edit)
    return out


def effective_favourite_artists(base, pending):
    """The registered favourite artists with the pending favourite-artist
    edits applied."""
    out = list(base or [])
    for edit in pending:
        if edit["kind"] == KIND_FAVOURITE_ARTIST:
            apply_to_favourite_artists(out, edit)
    return out


def effective_favourite_score(base_score, pending):
    """The registered favourite-artist bonus, or the last pending
    favourite-score edit's."""
    score = base_score
    for edit in pending:
        if edit["kind"] == KIND_FAVOURITE_SCORE:
            normalized = normalize_tag_score(edit.get("score"))
            if normalized is not None:
                score = normalized
    return score



def make_rating_edit(path, rating=None, favorite=None, source=""):
    """Validated rating edit (without seq). Raises ValueError on a bad
    rating or when there is nothing to change."""
    if rating is None and favorite is None:
        raise ValueError("nothing to change")
    if rating is not None:
        normalized = normalize_rating(rating)
        if normalized is None:
            raise ValueError(f"invalid rating {rating!r}: 0-5 in steps of 0.5")
        rating = normalized
    if favorite is not None and not isinstance(favorite, bool):
        raise ValueError("favorite must be true or false")
    return {"kind": KIND_RATING, "path": path, "rating": rating, "favorite": favorite, "source": source}


def make_playlist_edit(kind, path, playlist_id=None, name=None, source=""):
    """Validated playlist edit (without seq). Raises ValueError when neither
    a playlist id nor a name is given."""
    if kind not in (KIND_PLAYLIST_ADD, KIND_PLAYLIST_REMOVE):
        raise ValueError(f"unknown playlist edit {kind!r}")
    name = (name or "").strip() or None
    if playlist_id and str(playlist_id).startswith(PENDING_ID_PREFIX):
        name = name or str(playlist_id)[len(PENDING_ID_PREFIX):]
        playlist_id = None
    if not playlist_id and not name:
        raise ValueError("playlistId or name required")
    return {"kind": kind, "path": path, "playlistId": playlist_id or None, "name": name, "source": source}


def make_tag_score_edit(tag, score, source=""):
    """Validated tag-score edit (without seq): the score rounded to 0.5 and
    clamped to ±TAG_SCORE_MAX. Raises ValueError on a missing tag or a
    score that isn't a number."""
    key = tag_key(tag)
    if not key:
        raise ValueError("tag required")
    normalized = normalize_tag_score(score)
    if normalized is None:
        raise ValueError(f"invalid score {score!r}: a number, -{TAG_SCORE_MAX} to {TAG_SCORE_MAX} in steps of {TAG_SCORE_STEP}")
    return {"kind": KIND_TAG_SCORE, "tag": key, "score": normalized, "source": source}


def make_recalculate_edit(source=""):
    """Edit asking the desktop to score the library again now."""
    return {"kind": KIND_RECALCULATE_SCORES, "source": source}


def make_favourite_artist_edit(artist, favourite, source=""):
    """Validated favourite-artist edit (without seq): the artist name
    trimmed, its spelling kept. Raises ValueError on a missing name or a
    favourite flag that isn't true / false."""
    if not isinstance(artist, str) or not artist.strip():
        raise ValueError("artist required")
    if not isinstance(favourite, bool):
        raise ValueError("favourite must be true or false")
    return {"kind": KIND_FAVOURITE_ARTIST, "artist": artist.strip(), "favourite": favourite, "source": source}


def make_favourite_score_edit(score, source=""):
    """Validated favourite-artist bonus edit (without seq): stepped and
    clamped like a tag score. Raises ValueError when it isn't a number."""
    normalized = normalize_tag_score(score)
    if normalized is None:
        raise ValueError(f"invalid score {score!r}: a number, -{TAG_SCORE_MAX} to {TAG_SCORE_MAX} in steps of {TAG_SCORE_STEP}")
    return {"kind": KIND_FAVOURITE_SCORE, "score": normalized, "source": source}



class LibraryEditQueue:
    """Ordered, optionally persisted queue of edits awaiting the desktop's ack."""

    def __init__(self, persist_dir=None):
        self._lock = threading.Lock()
        self._pending = []
        self._next_seq = 1
        self._persist_path = None
        if persist_dir:
            self.set_persist_dir(persist_dir)

    def set_persist_dir(self, directory):
        """Mirror the queue to `<directory>/pending-library-edits.json` and
        load whatever an earlier run left there."""
        with self._lock:
            self._persist_path = os.path.join(directory, QUEUE_FILE)
            try:
                with open(self._persist_path, "r", encoding="utf-8") as f:
                    data = json.load(f)
                pending = [e for e in data.get("pending") or [] if isinstance(e, dict) and e.get("kind") in KINDS]
                self._pending = pending
                last = max((e.get("seq") or 0 for e in pending), default=0)
                self._next_seq = max(int(data.get("nextSeq") or 1), last + 1)
                if pending:
                    log.info("[edits] %d library edit(s) from the last run await the desktop", len(pending))
            except FileNotFoundError:
                pass
            except (OSError, ValueError, TypeError) as e:
                log.warning("[edits] Could not read %s: %s", self._persist_path, e)

    def _save(self):
        if not self._persist_path:
            return
        tmp = self._persist_path + ".tmp"
        try:
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump({"nextSeq": self._next_seq, "pending": self._pending}, f)
            os.replace(tmp, self._persist_path)
        except OSError as e:
            log.warning("[edits] Could not write %s: %s", self._persist_path, e)

    def enqueue(self, edit):
        """Stamp `edit` with `seq` + `ts`, append it, and return it.
        Raises QueueFull at MAX_PENDING."""
        with self._lock:
            if len(self._pending) >= MAX_PENDING:
                raise QueueFull(f"{MAX_PENDING} library edits are already waiting for the desktop app")
            stamped = {**edit, "seq": self._next_seq, "ts": int(time.time() * 1000)}
            self._next_seq += 1
            self._pending.append(stamped)
            self._save()
            return dict(stamped)

    def ack(self, seq):
        """Drop the edit with `seq`; returns it, or None when unknown
        (a duplicate ack after a replay)."""
        with self._lock:
            for i, edit in enumerate(self._pending):
                if edit.get("seq") == seq:
                    del self._pending[i]
                    self._save()
                    return edit
            return None

    def pending(self):
        """Copy of the waiting edits, oldest first."""
        with self._lock:
            return [dict(e) for e in self._pending]

    def clear(self):
        """Empty the queue and reset numbering (tests)."""
        with self._lock:
            self._pending = []
            self._next_seq = 1
            self._persist_path = None


queue = LibraryEditQueue()
