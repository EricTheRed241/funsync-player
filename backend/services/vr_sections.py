"""Library sections and filter tags for the headset players.

HereSphere (`/heresphere`) and DeoVR (`/deovr`) both show the library as
named sections, and the scene details carry tags the headset can filter
on. Both endpoints take their rules from here so the two players always
agree.

The first section is `Queue`: the shared play queue (the videos the user
queued, then the desktop's auto-fill), in queue order. The headset's own
next / previous walk the section a video was opened from, so starting from
Queue plays through it. Every queued scene carries the `Queue:Up next` tag.

Section names are plain text: HereSphere renders emoji as empty boxes.

Everything here works on fields already on the registry entry
(`avgSpeed`, `duration`, `dateAdded`, `handmadeCompanions`, `embeddedAxes`)
and the desktop's registered preference scores, so a listing never touches
the filesystem. A video missing a field is simply left out of the sections
that depend on it.

Order (user, 2026-11-10): the sections with no order of their own (the
score lists, VR, Flat, speed, length, Hand-made, and the app-made
playlists in `watched.playlistIds`, today Discovery) list by preference
score, highest first, unscored last, title breaking ties. Recently Added
lists the newest first. The score lists `Score 15+` and `Score 10-14.5` come
first after Queue: the videos whose score falls in that band (the desktop's
SCORE_BANDS, whose "Score below 10" collection has no section).

After those come the triage sections, from the desktop's ratings and
playlists (registered with the backend, pending headset/phone edits
included): Favorites, Top Rated (rating >= 4), Unrated, Watched, then one
`Playlist: <name>` per playlist in the desktop's playlist order, its videos
in playlist order.

Watched (user, 2026-11-10): a video counts as watched once it has a rating
(> 0); a favourite flag alone doesn't count. Headset playback never reaches
the desktop's resume tracking, so the rating is the one signal every player
shares, and a rating saved in the headset counts on the next listing. The
`Watched` section lists the rated videos newest rating first (`updatedAt`).
So that they aren't played again by accident, the browsing sections
(SECTION_ORDER) and the playlists the desktop names (`watched.playlistIds`,
today only Discovery) list the unwatched videos first, then the watched
ones, each group in its usual order, while the desktop setting is on
(`watched.sink`). Queue, the rating sections and the user's own playlists
keep their order. Watched last applies after the section's own order, so
each group stays by score (or date). Every watched scene carries the
read-only `Status:Watched` tag.
"""

import math
import time

from services import library_edits

# Speed buckets on the script's average speed (position units per second).
SLOW_BELOW_SPEED = 250    # avgSpeed < 250        → Slow
FAST_FROM_SPEED = 350     # 250 <= avgSpeed < 350 → Medium, >= 350 → Fast

# Length buckets on the video duration (seconds).
SHORT_BELOW_SECONDS = 5 * 60   # duration < 5 min           → Short
LONG_ABOVE_SECONDS = 15 * 60   # 5 min <= duration <= 15 min → Medium, > 15 min → Long

# "Recently Added" window on `dateAdded` (epoch milliseconds).
RECENT_DAYS = 14
_RECENT_WINDOW_MS = RECENT_DAYS * 24 * 60 * 60 * 1000

# Bucket labels double as the section names and the `Speed:` / `Length:`
# tag values, so the headset's sections and tag filters read the same.
SPEED_SLOW, SPEED_MEDIUM, SPEED_FAST = 'Slow', 'Medium', 'Fast'
LENGTH_SHORT, LENGTH_MEDIUM, LENGTH_LONG = 'Short (<5m)', 'Medium (5-15m)', 'Long (15m+)'

SECTION_VR = 'VR'
SECTION_FLAT = 'Flat'
SECTION_HANDMADE_AXES = 'Hand-made'
SECTION_RECENT = 'Recently Added'
SECTION_SCORE_ELITE = 'Score 20+'
SECTION_SCORE_TOP = 'Score 15+'
SECTION_SCORE_MID = 'Score 10-14.5'
SECTION_FAVORITES = 'Favorites'
SECTION_TOP_RATED = 'Top Rated'
SECTION_UNRATED = 'Unrated'
SECTION_WATCHED = 'Watched'
SECTION_QUEUE = 'Queue'
PLAYLIST_SECTION_PREFIX = 'Playlist: '

# Same threshold as the desktop (TOP_RATED_MIN in renderer/js/content-preferences.js).
TOP_RATED_MIN = 4

# Score lists: the desktop's SCORE_BANDS (renderer/js/content-preferences.js).
# score >= SCORE_ELITE_MIN → Score 20+ (and Score 15+ too: the lists overlap),
# score >= SCORE_TOP_MIN → Score 15+, SCORE_MID_MIN <= score < SCORE_TOP_MIN
# → Score 10-14.5; lower or unscored → none.
SCORE_ELITE_MIN = 20
SCORE_TOP_MIN = 15
SCORE_MID_MIN = 10

# Display order; empty sections are dropped.
SECTION_ORDER = (
    SECTION_SCORE_ELITE,
    SECTION_SCORE_TOP,
    SECTION_SCORE_MID,
    SECTION_VR,
    SECTION_FLAT,
    SPEED_SLOW,
    SPEED_MEDIUM,
    SPEED_FAST,
    LENGTH_SHORT,
    LENGTH_MEDIUM,
    LENGTH_LONG,
    SECTION_HANDMADE_AXES,
    SECTION_RECENT,
)

# The SECTION_ORDER sections listed by score, highest first (the rest of
# them: Recently Added, newest first).
SCORE_ORDERED_SECTIONS = frozenset(SECTION_ORDER) - {SECTION_RECENT}

# After SECTION_ORDER, before the per-playlist sections. None of them sink.
TRIAGE_SECTION_ORDER = (SECTION_FAVORITES, SECTION_TOP_RATED, SECTION_UNRATED, SECTION_WATCHED)

TAG_HANDMADE_AXES = 'Feature:Hand-made multi-axis'
# `Queue:Up next` scene tag on every queued video. Adding / removing it in
# HereSphere queues / unqueues the video.
QUEUE_TAG = 'Queue:Up next'

# Scene tag per playlist the video is in. Adding / removing one of these in
# HereSphere adds the video to / removes it from that playlist.
PLAYLIST_TAG_PREFIX = 'Playlist:'
# `Category:<name>` scene tags, one per desktop category the video is in.
CATEGORY_TAG_PREFIX = 'Category:'
# `Tag:<tag>` scene tags, one per chip tag (`services.video_tags`).
TAG_TAG_PREFIX = 'Tag:'
# `Score:<n>` scene tag: the desktop's preference score to the nearest 0.5,
# always with one decimal (`Score:7.5`, `Score:7.0`, `Score:-3.0`), so
# scenes with near-equal scores share a tag the headset can filter on.
SCORE_TAG_PREFIX = 'Score:'
# `Status:Watched` scene tag on every watched (rated) video. Read-only like
# `Category:`: the rating decides it.
STATUS_TAG_PREFIX = 'Status:'
WATCHED_TAG = f'{STATUS_TAG_PREFIX}Watched'


def _positive_number(value):
    """`value` as a float when it is a real positive number, else None.
    0 means "not computed / failed" for both speed and duration."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return float(value) if value > 0 else None


def speed_bucket(video):
    """SPEED_SLOW / SPEED_MEDIUM / SPEED_FAST from `avgSpeed`, or None
    without stats."""
    avg = _positive_number((video or {}).get('avgSpeed'))
    if avg is None:
        return None
    if avg < SLOW_BELOW_SPEED:
        return SPEED_SLOW
    if avg < FAST_FROM_SPEED:
        return SPEED_MEDIUM
    return SPEED_FAST


def length_bucket(video):
    """LENGTH_SHORT / LENGTH_MEDIUM / LENGTH_LONG from `duration`, or None
    when unknown."""
    duration = _positive_number((video or {}).get('duration'))
    if duration is None:
        return None
    if duration < SHORT_BELOW_SECONDS:
        return LENGTH_SHORT
    if duration <= LONG_ABOVE_SECONDS:
        return LENGTH_MEDIUM
    return LENGTH_LONG


def is_recent(video, now_ms):
    """True when `dateAdded` falls within the last RECENT_DAYS."""
    added = _positive_number((video or {}).get('dateAdded'))
    return added is not None and now_ms - added <= _RECENT_WINDOW_MS


def is_handmade_multi_axis(video):
    """True when the main script has extra axes a scripter made: a
    hand-made companion file (`handmadeCompanions`, set at registration) or
    non-empty axes embedded in the script itself (`embeddedAxes`, filled
    by the background pass registration starts)."""
    video = video or {}
    return bool(video.get('handmadeCompanions') or video.get('embeddedAxes'))


AXIS_HANDMADE = 'handmade'
AXIS_GENERATED = 'generated'
AXIS_SINGLE = 'single'


def axis_kind(video):
    """`handmade` (is_handmade_multi_axis), `generated` (only generated
    companion files, `generatedCompanions`), or `single`; None when the video
    has no script or its embedded-axes check hasn't run yet (no
    `embeddedAxes` key) and nothing else decides it."""
    video = video or {}
    if not video.get('funscriptPath'):
        return None
    if is_handmade_multi_axis(video):
        return AXIS_HANDMADE
    if 'embeddedAxes' not in video:
        return None
    return AXIS_GENERATED if video.get('generatedCompanions') else AXIS_SINGLE


def axis_flags(registry):
    """`{"flags": {path: kind}, "pending": n}` for every registered video
    with a script (`axis_kind`); undecided ones are counted in `pending`."""
    flags = {}
    pending = 0
    for v in registry.values():
        if not v.get('funscriptPath') or not v.get('path'):
            continue
        kind = axis_kind(v)
        if kind is None:
            pending += 1
        else:
            flags[v['path']] = kind
    return {'flags': flags, 'pending': pending}


def score_sections(score):
    """The score sections a preference score is in: [Score 20+, Score 15+],
    [Score 15+], [Score 10-14.5], or none (below SCORE_MID_MIN, or no score)."""
    if _score_value(score) is None:
        return []
    if score >= SCORE_ELITE_MIN:
        return [SECTION_SCORE_ELITE, SECTION_SCORE_TOP]
    if score >= SCORE_TOP_MIN:
        return [SECTION_SCORE_TOP]
    if score >= SCORE_MID_MIN:
        return [SECTION_SCORE_MID]
    return []


def _score_value(value):
    """`value` as a float when it is a real finite number, else None."""
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        return None
    return float(value)


def video_sections(video, is_3d, now_ms, score=None):
    """Every section this video belongs to, in SECTION_ORDER. `score` is its
    preference score (None = not scored)."""
    names = list(score_sections(score))
    names.append(SECTION_VR if is_3d else SECTION_FLAT)
    speed = speed_bucket(video)
    if speed:
        names.append(speed)
    length = length_bucket(video)
    if length:
        names.append(length)
    if is_handmade_multi_axis(video):
        names.append(SECTION_HANDMADE_AXES)
    if is_recent(video, now_ms):
        names.append(SECTION_RECENT)
    return names


def rating_sections(video, ratings):
    """Favorites / Top Rated / Unrated membership from the ratings map."""
    rating, favorite = library_edits.rating_entry(ratings, (video or {}).get('path'))
    names = []
    if favorite:
        names.append(SECTION_FAVORITES)
    if rating >= TOP_RATED_MIN:
        names.append(SECTION_TOP_RATED)
    if rating == 0:
        names.append(SECTION_UNRATED)
    return names


def is_watched(ratings, path):
    """True when `path` has a rating (> 0) in the ratings map. A favourite
    flag alone doesn't make a video watched."""
    return library_edits.rating_entry(ratings, path)[0] > 0


def watched_last(items, watched):
    """`items` with the ones `watched(item)` holds for moved to the end.
    Stable: each group keeps the order it came in. Returns a new list."""
    fresh, seen = [], []
    for item in items:
        (seen if watched(item) else fresh).append(item)
    return fresh + seen


def _updated_at(entry):
    value = entry.get('updatedAt') if isinstance(entry, dict) else None
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        return 0
    return value


def watched_paths(ratings):
    """Every watched path in the ratings map, newest rating first
    (`updatedAt`, missing = 0); the path breaks ties."""
    ratings = ratings or {}
    rated = [p for p in ratings if isinstance(p, str) and is_watched(ratings, p)]
    return sorted(rated, key=lambda p: (-_updated_at(ratings[p]), p))


def watched_tags(path, ratings):
    """`[Status:Watched]` when `path` is watched, else []. Read-only in the
    headset: editing it there changes nothing."""
    return [WATCHED_TAG] if path and is_watched(ratings, path) else []


def by_score(entries, score_of):
    """`entries` by preference score, highest first, unscored last; stable,
    so ties keep the order they came in (title). Returns a new list."""
    return sorted(entries, key=lambda e: (score_of(e) is None, -(score_of(e) or 0)))


def build_sections(entries, now_ms=None, ratings=None, playlists=None, queue_paths=None, watched=None, scores=None):
    """Group listing items into sections.

    `entries` yields `(item, video, is_3d)`, in title order. Returns
    `[(name, [item, ...])]`: `Queue` first (the entries named by
    `queue_paths`, in that order), then SECTION_ORDER, then Favorites / Top
    Rated / Unrated from `ratings` (videoPath → {rating, favorite,
    updatedAt}), Watched (the rated entries, newest rating first), then one
    `Playlist: <name>` per entry of `playlists` in the given order. Empty
    sections are omitted.

    `scores` is the desktop's preference score per video path (absent =
    not scored). The SCORE_ORDERED_SECTIONS and the playlists in
    `watched.playlistIds` (the app-made ones) list by score, highest first,
    unscored last; Recently Added lists the newest first (`dateAdded`); the
    queue, Watched and the other playlists follow their own order; the
    rating sections keep the order the entries came in.

    `watched` is the desktop's `{sink, playlistIds}` (`routes.media.
    get_watched`): with `sink` on, the SECTION_ORDER sections and the
    sections of the playlists in `playlistIds` list the unwatched items
    first, then the watched ones (`watched_last`), each group in the order
    above. None sinks nothing and orders no playlist by score.
    """
    if now_ms is None:
        now_ms = time.time() * 1000
    ratings = ratings or {}
    scores = scores or {}
    watched = watched or {}
    sink = watched.get('sink') is True
    app_playlists = set(watched.get('playlistIds') or [])
    # Items travel as (item, watched, score, dateAdded) until the end, so
    # ordering and sinking never have to look an item up again.
    grouped = {name: [] for name in SECTION_ORDER + TRIAGE_SECTION_ORDER}
    by_path = {}
    for item, video, is_3d in entries:
        video = video or {}
        path = video.get('path')
        score = _score_value(scores.get(path)) if path else None
        entry = (item, is_watched(ratings, path), score, _positive_number(video.get('dateAdded')) or 0)
        for name in video_sections(video, is_3d, now_ms, score) + rating_sections(video, ratings):
            grouped[name].append(entry)
        if path:
            by_path[path] = entry
    grouped[SECTION_WATCHED] = [by_path[p] for p in watched_paths(ratings) if p in by_path]
    for name in SCORE_ORDERED_SECTIONS:
        grouped[name] = by_score(grouped[name], lambda e: e[2])
    grouped[SECTION_RECENT].sort(key=lambda e: -e[3])

    def finish(group, sinks):
        if sinks:
            group = watched_last(group, lambda e: e[1])
        return [e[0] for e in group]

    out = []
    queued = [by_path[p] for p in queue_paths or [] if p in by_path]
    if queued:
        out.append((SECTION_QUEUE, finish(queued, False)))
    out.extend(
        (name, finish(grouped[name], sink and name in SECTION_ORDER))
        for name in SECTION_ORDER + TRIAGE_SECTION_ORDER if grouped[name]
    )
    for playlist in playlists or []:
        group = [by_path[p] for p in playlist.get('videoPaths') or [] if p in by_path]
        if not group:
            continue
        app_made = playlist.get('id') in app_playlists
        if app_made:
            group = by_score(group, lambda e: e[2])
        out.append((playlist_section_name(playlist.get('name')), finish(group, sink and app_made)))
    return out


def queued_paths(play_queue):
    """The play queue's paths in queue order: manual, then auto."""
    play_queue = play_queue or {}
    return list(play_queue.get('manual') or []) + list(play_queue.get('auto') or [])


def queue_tags(path, queued):
    """`[Queue:Up next]` when `path` is in `queued` (a collection of
    paths), else []."""
    return [QUEUE_TAG] if path and path in queued else []


def has_queue_tag(tags):
    """True when a HereSphere tag list (`[{"name": ...}, ...]`) carries
    `Queue:Up next` (trimmed, case-insensitive)."""
    key = QUEUE_TAG.casefold()
    return any(
        isinstance(tag, dict) and isinstance(tag.get('name'), str) and tag['name'].strip().casefold() == key
        for tag in tags or []
    )


def playlist_section_name(name):
    """`Playlist: <name>` — the headset section for one playlist."""
    return f'{PLAYLIST_SECTION_PREFIX}{(name or "").strip() or "(unnamed)"}'


def playlist_tags(path, playlists):
    """`Playlist:<name>` for every playlist holding `path`, in playlist order."""
    if not path:
        return []
    return [
        f'{PLAYLIST_TAG_PREFIX}{(p.get("name") or "").strip()}'
        for p in playlists or []
        if path in (p.get('videoPaths') or []) and (p.get('name') or '').strip()
    ]


def category_tags(path, categories, video_categories):
    """`Category:<name>` for every category holding `path`, in category order.
    Read-only in the headset: editing them there changes nothing."""
    if not path:
        return []
    ids = set((video_categories or {}).get(path) or [])
    return [
        f'{CATEGORY_TAG_PREFIX}{(c.get("name") or "").strip()}'
        for c in categories or []
        if c.get('id') in ids and (c.get('name') or '').strip()
    ]


def chip_tag_names(chip_tags):
    """`Tag:<tag>` for every chip tag, in the given order. Read-only in the
    headset like `Category:`."""
    return [f'{TAG_TAG_PREFIX}{tag}' for tag in chip_tags or []]


def score_tags(score):
    """`[Score:<n>]` for a preference score (nearest 0.5, halves rounded up,
    one decimal); [] when the video has none. Read-only like `Category:`."""
    if score is None:
        return []
    return [f'{SCORE_TAG_PREFIX}{math.floor(score * 2 + 0.5) / 2:.1f}']


def playlist_names_from_tags(tags):
    """Playlist names named by `Playlist:<name>` tags in a HereSphere tag
    list (`[{"name": ...}, ...]`), de-duplicated case-insensitively, in the
    order given. The prefix is matched case-insensitively; other tags are
    ignored."""
    names, seen = [], set()
    prefix = PLAYLIST_TAG_PREFIX.casefold()
    for tag in tags or []:
        raw = tag.get('name') if isinstance(tag, dict) else None
        if not isinstance(raw, str) or not raw.casefold().startswith(prefix):
            continue
        name = raw[len(PLAYLIST_TAG_PREFIX):].strip()
        key = name.casefold()
        if name and key not in seen:
            seen.add(key)
            names.append(name)
    return names


def filter_tags(video, is_3d):
    """Scene-detail filter tags: format, speed, length, hand-made axes."""
    tags = [f"Format:{'VR' if is_3d else 'Flat'}"]
    speed = speed_bucket(video)
    if speed:
        tags.append(f'Speed:{speed}')
    length = length_bucket(video)
    if length:
        tags.append(f'Length:{length}')
    if is_handmade_multi_axis(video):
        tags.append(TAG_HANDMADE_AXES)
    return tags
