"""Library sections and filter tags for the headset players.

HereSphere (`/heresphere`) and DeoVR (`/deovr`) both show the library as
named sections, and the scene details carry tags the headset can filter
on. Both endpoints take their rules from here so the two players always
agree.

Section names are plain text: HereSphere renders emoji as empty boxes.

Everything here works on fields already on the registry entry
(`avgSpeed`, `duration`, `dateAdded`, `handmadeAxes`), so a listing never
touches the filesystem. A video missing a field is simply left out of the
sections that depend on it.
"""

import time

# Speed buckets on the script's average speed (position units per second).
SLOW_BELOW_SPEED = 250    # avgSpeed < 250        → Slow
FAST_FROM_SPEED = 350     # 250 <= avgSpeed < 350 → Medium, >= 350 → Fast

# Length buckets on the video duration (seconds).
SHORT_BELOW_SECONDS = 5 * 60   # duration < 5 min           → Short
LONG_ABOVE_SECONDS = 15 * 60   # 5 min <= duration <= 15 min → Medium, > 15 min → Long

# "Recently Added" window on `dateAdded` (epoch milliseconds).
RECENT_DAYS = 14
_RECENT_WINDOW_MS = RECENT_DAYS * 24 * 60 * 60 * 1000

SPEED_SLOW, SPEED_MEDIUM, SPEED_FAST = 'Slow', 'Medium', 'Fast'
LENGTH_SHORT, LENGTH_MEDIUM, LENGTH_LONG = 'Short', 'Medium', 'Long'

SECTION_VR = 'VR Videos'
SECTION_FLAT = 'Flat Videos'
SECTION_SLOW = 'Slow'
SECTION_MEDIUM_SPEED = 'Medium Speed'
SECTION_FAST = 'Fast'
SECTION_SHORT = 'Short'
SECTION_MEDIUM_LENGTH = 'Medium Length'
SECTION_LONG = 'Long'
SECTION_HANDMADE_AXES = 'Hand-made Multi-Axis'
SECTION_RECENT = 'Recently Added'

# Display order; empty sections are dropped.
SECTION_ORDER = (
    SECTION_VR,
    SECTION_FLAT,
    SECTION_SLOW,
    SECTION_MEDIUM_SPEED,
    SECTION_FAST,
    SECTION_SHORT,
    SECTION_MEDIUM_LENGTH,
    SECTION_LONG,
    SECTION_HANDMADE_AXES,
    SECTION_RECENT,
)

_SPEED_SECTION = {
    SPEED_SLOW: SECTION_SLOW,
    SPEED_MEDIUM: SECTION_MEDIUM_SPEED,
    SPEED_FAST: SECTION_FAST,
}
_LENGTH_SECTION = {
    LENGTH_SHORT: SECTION_SHORT,
    LENGTH_MEDIUM: SECTION_MEDIUM_LENGTH,
    LENGTH_LONG: SECTION_LONG,
}

TAG_HANDMADE_AXES = 'Feature:Hand-made multi-axis'


def _positive_number(value):
    """`value` as a float when it is a real positive number, else None.
    0 means "not computed / failed" for both speed and duration."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return float(value) if value > 0 else None


def speed_bucket(video):
    """'Slow' / 'Medium' / 'Fast' from `avgSpeed`, or None without stats."""
    avg = _positive_number((video or {}).get('avgSpeed'))
    if avg is None:
        return None
    if avg < SLOW_BELOW_SPEED:
        return SPEED_SLOW
    if avg < FAST_FROM_SPEED:
        return SPEED_MEDIUM
    return SPEED_FAST


def length_bucket(video):
    """'Short' / 'Medium' / 'Long' from `duration`, or None when unknown."""
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


def video_sections(video, is_3d, now_ms):
    """Every section this video belongs to, in SECTION_ORDER."""
    names = [SECTION_VR if is_3d else SECTION_FLAT]
    speed = speed_bucket(video)
    if speed:
        names.append(_SPEED_SECTION[speed])
    length = length_bucket(video)
    if length:
        names.append(_LENGTH_SECTION[length])
    if (video or {}).get('handmadeAxes'):
        names.append(SECTION_HANDMADE_AXES)
    if is_recent(video, now_ms):
        names.append(SECTION_RECENT)
    return names


def build_sections(entries, now_ms=None):
    """Group listing items into sections.

    `entries` yields `(item, video, is_3d)`. Returns `[(name, [item, ...])]`
    in SECTION_ORDER with empty sections omitted; items keep the order
    they came in.
    """
    if now_ms is None:
        now_ms = time.time() * 1000
    grouped = {name: [] for name in SECTION_ORDER}
    for item, video, is_3d in entries:
        for name in video_sections(video, is_3d, now_ms):
            grouped[name].append(item)
    return [(name, grouped[name]) for name in SECTION_ORDER if grouped[name]]


def filter_tags(video, is_3d):
    """Scene-detail filter tags: format, speed, length, hand-made axes."""
    tags = [f"Format:{'VR' if is_3d else 'Flat'}"]
    speed = speed_bucket(video)
    if speed:
        tags.append(f'Speed:{speed}')
    length = length_bucket(video)
    if length:
        tags.append(f'Length:{length}')
    if (video or {}).get('handmadeAxes'):
        tags.append(TAG_HANDMADE_AXES)
    return tags
