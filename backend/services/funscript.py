"""Funscript parsing, CSV conversion, and multi-axis companion naming."""

import json
import math
from typing import Any


def parse_funscript(content: str) -> dict[str, Any]:
    """Parse a .funscript JSON file and return structured data.

    Args:
        content: Raw JSON string from .funscript file.

    Returns:
        Dict with keys: version, inverted, range, actions.
        Each action has 'at' (ms) and 'pos' (0-100).

    Raises:
        ValueError: If the funscript is invalid or missing required fields.
    """
    try:
        data = json.loads(content)
    except json.JSONDecodeError as e:
        raise ValueError(f"Invalid JSON in funscript: {e}")

    if "actions" not in data:
        raise ValueError("Funscript missing 'actions' array")

    actions = data["actions"]
    if not isinstance(actions, list):
        raise ValueError("Funscript 'actions' must be an array")

    for i, action in enumerate(actions):
        if "at" not in action or "pos" not in action:
            raise ValueError(f"Action at index {i} missing 'at' or 'pos'")
        if not isinstance(action["at"], (int, float)):
            raise ValueError(f"Action at index {i}: 'at' must be a number")
        if not isinstance(action["pos"], (int, float)):
            raise ValueError(f"Action at index {i}: 'pos' must be a number")

    # Sort actions by timestamp
    actions.sort(key=lambda a: a["at"])

    return {
        "version": data.get("version", "1.0"),
        "inverted": data.get("inverted", False),
        "range": data.get("range", 100),
        "actions": actions,
    }


def funscript_to_csv(actions: list[dict]) -> str:
    """Convert funscript actions to CSV format for the Handy device.

    Args:
        actions: List of dicts with 'at' (ms) and 'pos' (0-100).

    Returns:
        CSV string with lines of "timestamp_ms,position".
    """
    lines = []
    for action in actions:
        at = int(action["at"])
        pos = max(0, min(100, int(action["pos"])))
        lines.append(f"{at},{pos}")
    return "\n".join(lines)


# --- Multi-axis companion naming ----------------------------------------
# Mirrors renderer/js/multi-axis.js (AXIS_DEFINITIONS, parseAxisSuffix,
# getBaseName): `Title.twist.funscript` is a companion axis of
# `Title.funscript`; `Title.generated.twist.funscript` is one the app
# generated rather than one a scripter made. Suffixes are case-insensitive
# and include each axis's aliases.

AXIS_SUFFIXES = frozenset({
    'surge', 'forward', 'l1',
    'sway', 'left', 'l2',
    'twist', 'yaw', 'r0',
    'roll', 'r1',
    'pitch', 'r2',
    'vib', 'vibrate', 'v0',
    'pump', 'v1',
    'valve', 'a0',
    'suck', 'suction', 'a1',
    'lube', 'a2',
})

_FUNSCRIPT_EXT = '.funscript'
_GENERATED_MARK = '.generated'


def _split_axis_name(filename: str):
    """`(base, axis, generated)` for a funscript filename, else None.
    `axis` is the lowercase suffix, or None for a main (single-axis) script."""
    if not filename.lower().endswith(_FUNSCRIPT_EXT):
        return None
    stem = filename[:-len(_FUNSCRIPT_EXT)]
    base, dot, suffix = stem.rpartition('.')
    if not dot or suffix.lower() not in AXIS_SUFFIXES:
        return stem, None, False
    generated = base.lower().endswith(_GENERATED_MARK)
    if generated:
        base = base[:-len(_GENERATED_MARK)]
    return base, suffix.lower(), generated


def has_handmade_companions(script_path: str, filenames) -> bool:
    """True when `filenames` (one folder's listing) holds at least one
    companion axis file of the main script `script_path` that is NOT a
    `*.generated.<axis>.funscript`. macOS `._` AppleDouble files are ignored.
    """
    main = _split_axis_name(script_path.replace('\\', '/').rsplit('/', 1)[-1])
    if main is None:
        return False
    main_base = main[0].lower()
    for name in filenames:
        if name.startswith('._'):
            continue
        parts = _split_axis_name(name)
        if parts is None:
            continue
        base, axis, generated = parts
        if axis and not generated and base.lower() == main_base:
            return True
    return False


# --- Embedded multi-axis data ---------------------------------------------
# Mirrors renderer/js/embedded-multi-axis.js (`extractEmbeddedAxes`): extra
# axes stored INSIDE the main .funscript rather than in companion files.
#   A  `additional_axes`: [{axis: <name>, actions}]
#   B  `raw`: {<name>: {actions}}
#   E  `axes`: [{id: <TCode>, actions}]
#   C  top-level sibling keys: <name>: [actions] | {actions}
#   D  inline TCode keys on the main actions: {at, pos, R0: 80, ...}
#   F  `channels`: {<name or TCode>: [actions] | {actions}}
# Only the axis set matters here (Hand-made section and tag), so precedence
# between formats is irrelevant. An axis counts only with >= 2 actions.

# TCode id → axis suffix; L0 is the main stroke, never an extra axis.
_EMBEDDED_TCODE_AXES = {
    'L0': None, 'L1': 'surge', 'L2': 'sway',
    'R0': 'twist', 'R1': 'roll', 'R2': 'pitch',
    'V0': 'vib', 'V1': 'lube', 'V2': 'suction', 'A0': 'valve',
}
# Axis name (case-insensitive) → suffix.
_EMBEDDED_NAME_AXES = {
    'surge': 'surge', 'sway': 'sway',
    'twist': 'twist', 'rotate': 'twist',
    'roll': 'roll', 'pitch': 'pitch',
    'vib': 'vib', 'vibrate': 'vib', 'vibration': 'vib',
    'lube': 'lube', 'pump': 'pump',
    'suction': 'suction', 'suck': 'suction',
    'valve': 'valve',
}
# Top-level keys format C never treats as axes.
_EMBEDDED_SKIP_KEYS = frozenset({
    'actions', 'version', 'inverted', 'range', 'metadata', 'raw',
    'additional_axes', 'tcode', 'axes', 'channels',
    'cachedAt', '_sourcePath', 'csvHash',
})
_MIN_AXIS_ACTIONS = 2

# Lowercased JSON keys whose presence makes embedded axes possible. A
# script with none of them is skipped without parsing it.
_EMBEDDED_MARKERS = tuple(
    f'"{key}"'.encode()
    for key in (
        'additional_axes', 'raw', 'axes', 'channels',
        *_EMBEDDED_NAME_AXES,
        *(tcode.lower() for tcode, axis in _EMBEDDED_TCODE_AXES.items() if axis),
    )
)


def _is_time(value) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def _axis_actions(value):
    """The actions list when `value` is `[actions]` or `{actions: [...]}`
    holding at least `_MIN_AXIS_ACTIONS` actions that all have a numeric
    `at` (the renderer's `isValidActionArray`), else None."""
    if isinstance(value, dict):
        value = value.get('actions')
    if (
        isinstance(value, list)
        and len(value) >= _MIN_AXIS_ACTIONS
        and all(isinstance(a, dict) and _is_time(a.get('at')) for a in value)
    ):
        return value
    return None


def _axis_from_name(name):
    return _EMBEDDED_NAME_AXES.get(name.strip().lower()) if isinstance(name, str) else None


def _axis_from_tcode(tcode):
    return _EMBEDDED_TCODE_AXES.get(tcode.strip().upper()) if isinstance(tcode, str) else None


def may_have_embedded_axes(raw: bytes) -> bool:
    """Cheap pre-check on a script's raw bytes: False means
    `embedded_axes` would find nothing, so the parse can be skipped."""
    lowered = raw.lower()
    return any(marker in lowered for marker in _EMBEDDED_MARKERS)


def _list_field(parsed: dict, key: str) -> list:
    value = parsed.get(key)
    return value if isinstance(value, list) else []


def _dict_field(parsed: dict, key: str) -> dict:
    value = parsed.get(key)
    return value if isinstance(value, dict) else {}


def embedded_axes(parsed) -> list[str]:
    """Sorted axis suffixes with non-empty (>= 2 actions) data embedded in
    a parsed funscript."""
    if not isinstance(parsed, dict):
        return []
    found = set()

    # A: additional_axes [{axis, actions}]
    for entry in _list_field(parsed, 'additional_axes'):
        if isinstance(entry, dict) and _axis_actions(entry.get('actions')):
            found.add(_axis_from_name(entry.get('axis')))

    # B: raw {name: {actions}}
    for key, value in _dict_field(parsed, 'raw').items():
        if isinstance(value, dict) and _axis_actions(value):
            found.add(_axis_from_name(key))

    # F: channels {name or TCode: [actions] | {actions}}
    for key, value in _dict_field(parsed, 'channels').items():
        if _axis_actions(value):
            found.add(_axis_from_name(key) or _axis_from_tcode(key))

    # E: axes [{id: TCode, actions}]
    for entry in _list_field(parsed, 'axes'):
        if isinstance(entry, dict) and _axis_actions(entry.get('actions')):
            found.add(_axis_from_tcode(entry.get('id')))

    # C: sibling keys {name: [actions] | {actions}}
    for key, value in parsed.items():
        axis = None if key in _EMBEDDED_SKIP_KEYS else _axis_from_name(key)
        if axis and _axis_actions(value):
            found.add(axis)

    # D: inline TCode values on the main actions
    inline_counts: dict[str, int] = {}
    for action in _list_field(parsed, 'actions'):
        if not isinstance(action, dict) or not _is_time(action.get('at')):
            continue
        for key, value in action.items():
            axis = _EMBEDDED_TCODE_AXES.get(key)
            if axis and isinstance(value, (int, float)) and not isinstance(value, bool):
                inline_counts[axis] = inline_counts.get(axis, 0) + 1
    found.update(axis for axis, n in inline_counts.items() if n >= _MIN_AXIS_ACTIONS)

    found.discard(None)
    return sorted(found)
