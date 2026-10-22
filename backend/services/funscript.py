"""Funscript parsing, CSV conversion, and multi-axis companion naming."""

import json
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


def has_handmade_axes(script_path: str, filenames) -> bool:
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
