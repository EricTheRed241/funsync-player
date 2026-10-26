"""Library sections and filter tags for the headset players.

HereSphere (`/heresphere`) and DeoVR (`/deovr`) both show the library as
named sections, and the scene details carry tags the headset can filter
on. Both endpoints take their rules from here so the two players always
agree.

Section names are plain text: HereSphere renders emoji as empty boxes.

Everything here works on fields already on the registry entry
(`avgSpeed`, `duration`, `dateAdded`, `handmadeCompanions`, `embeddedAxes`), so
a listing never touches the filesystem. A video missing a field is simply
left out of the sections that depend on it.

After those come the triage sections, from the desktop's ratings and
playlists (registered with the backend, pending headset/phone edits
included): Favorites, Top Rated (rating >= 4), Unrated, then one
`Playlist: <name>` per playlist in the desktop's playlist order, its videos
in playlist order.

"Unrated" is every video without a rating. The issue asked for watched-but-
unrated, but there is no reliable "watched" signal here: headset playback
never reaches the desktop's resume tracking, so watching a scene in
HereSphere — the case this queue exists for — would leave it out.
"""

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
SECTION_FAVORITES = 'Favorites'
SECTION_TOP_RATED = 'Top Rated'
SECTION_UNRATED = 'Unrated'
PLAYLIST_SECTION_PREFIX = 'Playlist: '

# Same threshold as the desktop (renderer/js/ratings.js TOP_RATED_MIN).
TOP_RATED_MIN = 4

# Display order; empty sections are dropped.
SECTION_ORDER = (
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

# After SECTION_ORDER, before the per-playlist sections.
TRIAGE_SECTION_ORDER = (SECTION_FAVORITES, SECTION_TOP_RATED, SECTION_UNRATED)

TAG_HANDMADE_AXES = 'Feature:Hand-made multi-axis'

# Scene tag per playlist the video is in. Adding / removing one of these in
# HereSphere adds the video to / removes it from that playlist.
PLAYLIST_TAG_PREFIX = 'Playlist:'
# `Category:<name>` scene tags, one per desktop category the video is in.
CATEGORY_TAG_PREFIX = 'Category:'


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


def video_sections(video, is_3d, now_ms):
    """Every section this video belongs to, in SECTION_ORDER."""
    names = [SECTION_VR if is_3d else SECTION_FLAT]
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


def build_sections(entries, now_ms=None, ratings=None, playlists=None):
    """Group listing items into sections.

    `entries` yields `(item, video, is_3d)`. Returns `[(name, [item, ...])]`:
    SECTION_ORDER, then Favorites / Top Rated / Unrated from `ratings`
    (videoPath → {rating, favorite}), then one `Playlist: <name>` per entry
    of `playlists` in the given order. Empty sections are omitted. Items keep
    the order they came in, except playlist sections, which follow the
    playlist's own order.
    """
    if now_ms is None:
        now_ms = time.time() * 1000
    ratings = ratings or {}
    grouped = {name: [] for name in SECTION_ORDER + TRIAGE_SECTION_ORDER}
    by_path = {}
    for item, video, is_3d in entries:
        for name in video_sections(video, is_3d, now_ms) + rating_sections(video, ratings):
            grouped[name].append(item)
        path = (video or {}).get('path')
        if path:
            by_path[path] = item
    out = [(name, grouped[name]) for name in SECTION_ORDER + TRIAGE_SECTION_ORDER if grouped[name]]
    for playlist in playlists or []:
        items = [by_path[p] for p in playlist.get('videoPaths') or [] if p in by_path]
        if items:
            out.append((playlist_section_name(playlist.get('name')), items))
    return out


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
