"""FFmpeg/FFprobe services — metadata extraction and thumbnail generation."""

import subprocess
import json
import logging
import math
import os
import hashlib
import re
import shutil
import sys
import tempfile
import threading
import time
import atexit
from typing import Any

log = logging.getLogger("funsync.ffmpeg")


def _find_binary(name: str) -> str:
    """Find ffmpeg/ffprobe binary — in the project's ffmpeg dirs, or on PATH."""
    # Check sibling dev directories. Windows keeps ffmpeg in `ffmpeg/`;
    # Linux keeps it in `ffmpeg-linux/`. Probe both so runs work on either
    # OS without a reshuffle.
    project_root = os.path.join(os.path.dirname(__file__), "..", "..")
    exe_suffix = ".exe" if os.name == "nt" else ""
    dev_dirs = ("ffmpeg-linux", "ffmpeg") if os.name != "nt" else ("ffmpeg",)
    for d in dev_dirs:
        dev_path = os.path.join(project_root, d, name + exe_suffix)
        if os.path.exists(dev_path):
            return os.path.abspath(dev_path)
    # Fall back to PATH
    return name


FFPROBE = _find_binary("ffprobe")
FFMPEG = _find_binary("ffmpeg")


# Suppress the cmd.exe window that Windows creates for every ffmpeg /
# ffprobe subprocess when the parent is a GUI app. Without this flag the
# user sees console windows flashing during library scan (one per
# thumbnail / metadata probe / VR info probe). On non-Windows platforms
# the flag evaluates to 0 and is a no-op.
NO_WINDOW_CREATIONFLAGS = 0x08000000 if sys.platform == "win32" else 0


def run_silent(*args, **kwargs):
    """`subprocess.run` wrapper that hides the Windows console popup for
    GUI-parent subprocess calls. Use for every ffmpeg / ffprobe call
    anywhere in the backend — the default `subprocess.run` flashes a
    console per call."""
    if sys.platform == "win32":
        kwargs.setdefault("creationflags", NO_WINDOW_CREATIONFLAGS)
    return subprocess.run(*args, **kwargs)


# Metadata cache. Keyed by (path, size, mtime) so file edits invalidate
# naturally. The thumbnail-single endpoint calls get_metadata for EVERY
# thumbnail to know the duration; without caching, that doubles the
# per-thumbnail cost (ffprobe + ffmpeg).
#
# PERSISTED TO DISK (2026-08-03): the cache used to be in-memory only, so
# it died with the backend process — i.e. on EVERY app launch. Startup
# profiling on a 420-video library showed each visible card paying a fresh
# ~700ms ffprobe for a video probed dozens of times before (~7s of the
# ~13s startup; the thumbnail JPEGs themselves were disk-cached and
# near-free). The cache now loads lazily from a JSON file in the thumbnail
# cache directory (`thumb_cache_dir()`: the user-data dir's `thumb-cache`,
# else the temp dir) and is written back with a throttle.
#
# Concurrency: FastAPI runs sync route handlers in a threadpool, so
# get_metadata races with itself — all cache state is guarded by
# _metadata_cache_lock. The disk write is atomic (tmp file + os.replace,
# which is atomic on both Windows and POSIX) so a concurrent second
# backend instance can't observe a torn file.
#
# Loss window: the write-behind throttle means a hard kill (Windows
# taskkill /F skips atexit) can lose the last few seconds of NEW probes.
# Cost: those videos re-probe once next launch. Graceful exits (Linux
# SIGTERM, dev Ctrl-C) flush via atexit.
_metadata_cache: dict[str, dict[str, Any]] = {}
_metadata_cache_lock = threading.Lock()
_metadata_cache_loaded = False
_metadata_cache_dirty = 0          # new entries since last successful save
_metadata_cache_last_save = 0.0    # time.monotonic() of last save attempt

METADATA_CACHE_VERSION = 1
# ~200 bytes/entry → 5000 entries ≈ 1MB ceiling. Oldest-inserted pruned
# first on save; stale keys (moved/renamed/deleted files) age out this way.
METADATA_CACHE_MAX_ENTRIES = 5000
METADATA_CACHE_SAVE_INTERVAL_SEC = 2.0
METADATA_CACHE_SAVE_BATCH = 16     # force a save every N new entries


def _metadata_cache_path() -> str:
    # Resolved at call time (not import) so the user-data dir set at startup
    # applies, and tests that monkeypatch `tempfile` on this module isolate
    # the file, mirroring the thumbnail-cache tests.
    return os.path.join(thumb_cache_dir(), "metadata-cache-v1.json")


def _load_metadata_cache_locked() -> None:
    """Populate _metadata_cache from disk, once. Caller holds the lock.
    Missing/corrupt/wrong-version files are silently discarded — the cache
    is a pure accelerator and must never block a probe."""
    global _metadata_cache_loaded
    if _metadata_cache_loaded:
        return
    _metadata_cache_loaded = True
    try:
        with open(_metadata_cache_path(), encoding="utf-8") as f:
            payload = json.load(f)
        if (isinstance(payload, dict)
                and payload.get("version") == METADATA_CACHE_VERSION
                and isinstance(payload.get("entries"), dict)):
            for key, value in payload["entries"].items():
                if isinstance(key, str) and isinstance(value, dict):
                    _metadata_cache[key] = value
    except (OSError, ValueError):
        pass  # no cache yet / unreadable / corrupt JSON → start empty


def _save_metadata_cache(force: bool = False) -> None:
    """Write-behind persistence. Throttled so a cold library scan doesn't
    rewrite the file hundreds of times; `force` (atexit) flushes whatever
    is pending. Failures are swallowed — worst case is re-probing later."""
    global _metadata_cache_dirty, _metadata_cache_last_save
    with _metadata_cache_lock:
        if _metadata_cache_dirty == 0:
            return
        now = time.monotonic()
        if (not force
                and _metadata_cache_dirty < METADATA_CACHE_SAVE_BATCH
                and (now - _metadata_cache_last_save) < METADATA_CACHE_SAVE_INTERVAL_SEC):
            return
        # Prune oldest-inserted entries beyond the cap (dict preserves
        # insertion order; disk-loaded entries were inserted first).
        excess = len(_metadata_cache) - METADATA_CACHE_MAX_ENTRIES
        if excess > 0:
            for key in list(_metadata_cache)[:excess]:
                del _metadata_cache[key]
        snapshot = {"version": METADATA_CACHE_VERSION, "entries": dict(_metadata_cache)}
        _metadata_cache_dirty = 0
        _metadata_cache_last_save = now
    # Serialize + write OUTSIDE the lock — probes shouldn't wait on disk.
    try:
        path = _metadata_cache_path()
        os.makedirs(os.path.dirname(path), exist_ok=True)
        tmp_path = f"{path}.tmp-{os.getpid()}"
        with open(tmp_path, "w", encoding="utf-8") as f:
            json.dump(snapshot, f, ensure_ascii=False)
        os.replace(tmp_path, path)  # atomic on Windows AND POSIX
    except OSError:
        pass


atexit.register(lambda: _save_metadata_cache(force=True))


def _reset_metadata_cache_for_tests() -> None:
    """Test hook: drop all in-memory cache state so a test can simulate a
    backend restart (the disk file is untouched — point the module's
    `tempfile` at a tmp_path to isolate that too)."""
    global _metadata_cache_loaded, _metadata_cache_dirty, _metadata_cache_last_save
    with _metadata_cache_lock:
        _metadata_cache.clear()
        _metadata_cache_loaded = False
        _metadata_cache_dirty = 0
        _metadata_cache_last_save = 0.0

# Per-thumbnail ffmpeg decode budget. With the `-skip_frame nokey` strategy a
# single frame decodes in ~1s (5.7s max in the 8K-HEVC benchmark), stretching
# to ~15-20s only under heavy sibling contention. 25s clears that legitimate
# worst case with headroom, but is 2.4× faster than the old 60s at giving up
# on a slow/disconnected/hung drive (NAS dropout, spun-down HDD) — the report
# was a large multi-drive library freezing, where doomed reads held all four
# worker slots for a full minute each. Tune here if VR/8K thumbnails on very
# slow storage start timing out.
THUMBNAIL_TIMEOUT_SEC = 25


def get_metadata(video_path: str) -> dict[str, Any]:
    """Extract video metadata using ffprobe.

    Args:
        video_path: Absolute path to the video file.

    Returns:
        Dict with keys: duration, width, height, codec, profile, pixFmt,
        format, bitrate, fps.

    Raises:
        FileNotFoundError: If video file doesn't exist.
        RuntimeError: If ffprobe fails.
    """
    if not os.path.exists(video_path):
        raise FileNotFoundError(f"Video not found: {video_path}")

    # Cache lookup — most callers will hit this for the same files
    # repeatedly during a session (each thumbnail call previously
    # re-ran ffprobe wastefully). First call lazily loads the persisted
    # cache from disk so probes survive backend restarts (app launches).
    stat = os.stat(video_path)
    cache_key = f"{video_path}:{stat.st_size}:{stat.st_mtime}"
    with _metadata_cache_lock:
        _load_metadata_cache_locked()
        cached = _metadata_cache.get(cache_key)
    if cached is not None:
        return cached

    try:
        result = run_silent(
            [
                FFPROBE,
                "-v", "quiet",
                "-print_format", "json",
                "-show_format",
                "-show_streams",
                video_path,
            ],
            capture_output=True,
            text=True,
            # ffmpeg/ffprobe write UTF-8 to stdout/stderr (file paths, codec
            # messages). Without an explicit encoding here Python's reader
            # thread defaults to the OS locale — cp1252 on Windows — and any
            # byte that doesn't map (smart quotes, accented chars, raw codec
            # output) crashes the reader thread with UnicodeDecodeError.
            encoding="utf-8",
            errors="replace",
            timeout=30,
        )
    except FileNotFoundError:
        raise RuntimeError("ffprobe not found. Install ffmpeg to use metadata features.")

    if result.returncode != 0:
        raise RuntimeError(f"ffprobe failed: {result.stderr}")

    data = json.loads(result.stdout)

    # Find video stream
    video_stream = None
    for stream in data.get("streams", []):
        if stream.get("codec_type") == "video":
            video_stream = stream
            break

    fmt = data.get("format", {})

    metadata = {
        "duration": float(fmt.get("duration", 0)),
        "format": fmt.get("format_name", "unknown"),
        "bitrate": int(fmt.get("bit_rate", 0)),
    }

    if video_stream:
        metadata.update({
            "width": int(video_stream.get("width", 0)),
            "height": int(video_stream.get("height", 0)),
            "codec": video_stream.get("codec_name", "unknown"),
            # profile + pixel format distinguish the variants Chromium
            # can't decode (e.g. H.264 "High 10" / yuv422p / yuv444p) from
            # the ones it can (8-bit "High"/"Main" yuv420p). Surfaced in the
            # renderer's decode-error message so a Linux user sees exactly
            # why one H.264 file plays and another doesn't. Empty string
            # when ffprobe doesn't report them.
            "profile": video_stream.get("profile", "") or "",
            "pixFmt": video_stream.get("pix_fmt", "") or "",
            "fps": _parse_fps(video_stream.get("r_frame_rate", "0/1")),
        })

    global _metadata_cache_dirty
    with _metadata_cache_lock:
        _metadata_cache[cache_key] = metadata
        _metadata_cache_dirty += 1
    _save_metadata_cache()
    return metadata


def _parse_fps(rate_str: str) -> float:
    """Parse ffprobe frame rate string (e.g. '30/1' or '30000/1001')."""
    try:
        num, den = rate_str.split("/")
        return round(int(num) / int(den), 2) if int(den) != 0 else 0
    except (ValueError, ZeroDivisionError):
        return 0


def _get_video_hash(video_path: str) -> str:
    """Generate a short hash from the video file path + modification time for caching."""
    stat = os.stat(video_path)
    key = f"{video_path}:{stat.st_size}:{stat.st_mtime}"
    return hashlib.md5(key.encode()).hexdigest()[:12]


def generate_thumbnails(
    video_path: str,
    output_dir: str | None = None,
    interval: int = 10,
    width: int = 160,
    height: int = 90,
) -> dict[str, Any]:
    """Generate individual thumbnail images from a video at regular intervals.

    Args:
        video_path: Absolute path to the video file.
        output_dir: Directory to save thumbnails. Auto-generated if None.
        interval: Seconds between thumbnails.
        width: Thumbnail width in pixels.
        height: Thumbnail height in pixels.

    Returns:
        Dict with keys:
            - thumbnails: list of {time: float, path: str}
            - interval: seconds between thumbnails
            - count: total number of thumbnails

    Raises:
        FileNotFoundError: If video file doesn't exist.
        RuntimeError: If ffmpeg fails.
    """
    if not os.path.exists(video_path):
        raise FileNotFoundError(f"Video not found: {video_path}")

    # Determine output directory
    video_hash = _get_video_hash(video_path)
    if output_dir is None:
        output_dir = os.path.join(tempfile.gettempdir(), "funsync_thumbs", video_hash)

    os.makedirs(output_dir, exist_ok=True)

    # Check if thumbnails already exist (cached)
    manifest_path = os.path.join(output_dir, "manifest.json")
    if os.path.exists(manifest_path):
        with open(manifest_path, "r") as f:
            manifest = json.load(f)
        # Verify files still exist
        if all(os.path.exists(t["path"]) for t in manifest.get("thumbnails", [])):
            return manifest

    # Get video duration first
    metadata = get_metadata(video_path)
    duration = metadata["duration"]

    if duration <= 0:
        return {"thumbnails": [], "interval": interval, "count": 0}

    # Generate thumbnails using ffmpeg
    output_pattern = os.path.join(output_dir, "thumb_%04d.jpg")

    try:
        result = run_silent(
            [
                FFMPEG,
                "-i", video_path,
                "-vf", f"fps=1/{interval},scale={width}:{height}",
                "-q:v", "5",
                "-y",
                output_pattern,
            ],
            capture_output=True,
            text=True,
            # See get_metadata above for why explicit utf-8/replace matters
            # on Windows — ffmpeg's stderr crashes the reader thread under
            # cp1252 when the input path or codec message has unmappable bytes.
            encoding="utf-8",
            errors="replace",
            timeout=120,
        )
    except FileNotFoundError:
        raise RuntimeError("ffmpeg not found. Install ffmpeg to use thumbnail features.")

    if result.returncode != 0:
        raise RuntimeError(f"ffmpeg thumbnail generation failed: {result.stderr}")

    # Collect generated files
    thumbnails = []
    idx = 1
    while True:
        thumb_path = os.path.join(output_dir, f"thumb_{idx:04d}.jpg")
        if not os.path.exists(thumb_path):
            break
        thumbnails.append({
            "time": (idx - 1) * interval,
            "path": thumb_path,
        })
        idx += 1

    manifest = {
        "thumbnails": thumbnails,
        "interval": interval,
        "count": len(thumbnails),
        "video_hash": video_hash,
    }

    # Cache the manifest
    with open(manifest_path, "w") as f:
        json.dump(manifest, f)

    return manifest


# ---------------------------------------------------------------------------
# Container remux (MKV → MP4)
#
# Chromium's <video> element can't demux the Matroska (.mkv) container even
# when the streams inside are formats it fully supports (H.264 + AAC). The
# user just gets "format not supported". The fix is to repackage the streams
# into an MP4 container WITHOUT re-encoding (`-c copy`) — instant, lossless —
# and play that instead. Only an audio track in a codec MP4/Chromium can't
# carry (AC3/DTS/FLAC/Opus/…) needs a cheap AAC transcode.
#
# Community-reported (2026-06): every H.264/AAC .mkv failed; converting to
# .mp4 by hand fixed it. This makes the app do that automatically on load.
# ---------------------------------------------------------------------------

# Video codecs we can stream-copy into MP4 AND Chromium can then decode.
# Deliberately conservative: e.g. mpeg4 (DivX/Xvid) would copy fine but
# Chromium can't decode it, so remuxing wouldn't actually help — better to
# fail clearly than produce a still-unplayable file.
_REMUX_VIDEO_COPYABLE = {"h264", "hevc", "h265", "av1"}
# Audio codecs already MP4/Chromium-friendly → copy as-is. Everything else
# (ac3, eac3, dts, truehd, flac, opus, vorbis, pcm_*) → transcode to AAC.
_REMUX_AUDIO_COPYABLE = {"aac", "mp3"}


def _probe_audio_codec(video_path: str) -> str | None:
    """Return the first audio stream's codec name (lowercase), or None if
    the file has no audio track. Best-effort — returns None on probe error
    so the caller treats it as 'no audio' rather than crashing the remux."""
    try:
        result = run_silent(
            [
                FFPROBE,
                "-v", "quiet",
                "-select_streams", "a:0",
                "-show_entries", "stream=codec_name",
                "-of", "default=nokey=1:noprint_wrappers=1",
                video_path,
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=30,
        )
    except (FileNotFoundError, subprocess.SubprocessError):
        return None
    name = (result.stdout or "").strip().lower()
    return name or None


def remux_to_mp4(video_path: str) -> dict[str, Any]:
    """Repackage a video into an MP4 container for browser playback.

    Stream-copies the video (and audio, when already MP4-friendly); only an
    incompatible audio track is transcoded to AAC. Output is cached on disk
    by content hash so re-opening the same file is instant.

    Args:
        video_path: Absolute path to the source video (typically .mkv).

    Returns:
        Dict with keys:
            path:   absolute path to the remuxed MP4
            cached: True if a prior remux was reused
            vcodec / acodec: detected source codecs (for logging)

    Raises:
        FileNotFoundError: source missing or empty.
        RuntimeError: ffmpeg failed, or the video codec can't be made
            playable by remuxing (caller should surface "unsupported").
    """
    if not os.path.exists(video_path):
        raise FileNotFoundError(f"Video not found: {video_path}")
    if os.path.getsize(video_path) == 0:
        raise FileNotFoundError(f"Video is empty (0 bytes): {video_path}")

    out_dir = os.path.join(tempfile.gettempdir(), "funsync_remux")
    os.makedirs(out_dir, exist_ok=True)
    # Hash includes size+mtime (via _get_video_hash) so an edited/replaced
    # source re-remuxes instead of serving a stale MP4.
    out_path = os.path.join(out_dir, f"{_get_video_hash(video_path)}.mp4")

    if os.path.exists(out_path) and os.path.getsize(out_path) > 0:
        return {"path": out_path, "cached": True}

    meta = get_metadata(video_path)
    vcodec = (meta.get("codec") or "").lower()
    if vcodec not in _REMUX_VIDEO_COPYABLE:
        raise RuntimeError(
            f"video codec '{vcodec or 'unknown'}' can't be remuxed to a "
            f"browser-playable MP4 (would need a full transcode)"
        )

    acodec = _probe_audio_codec(video_path)

    args = [
        FFMPEG, "-y",
        "-i", video_path,
        # First video + first audio (if any). `?` makes the audio map
        # optional so audio-less files don't fail the whole command.
        "-map", "0:v:0", "-map", "0:a:0?",
        "-c:v", "copy",
    ]
    if acodec is None:
        pass  # no audio track
    elif acodec in _REMUX_AUDIO_COPYABLE:
        args += ["-c:a", "copy"]
    else:
        args += ["-c:a", "aac", "-b:a", "192k"]
    # +faststart relocates the moov atom to the front so the file is
    # seekable/streamable immediately (matters when the same MP4 is later
    # served over HTTP to the VR / web-remote clients).
    args += ["-movflags", "+faststart", out_path]

    try:
        result = run_silent(
            args,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            # Stream-copy is I/O-bound and fast even for multi-GB files, but
            # a large file on slow/NAS storage (plus an AAC audio transcode)
            # can take a while. Generous ceiling; normal case is seconds.
            timeout=600,
        )
    except FileNotFoundError:
        raise RuntimeError("ffmpeg not found. Install ffmpeg to play this format.")
    except subprocess.TimeoutExpired:
        _safe_unlink(out_path)
        raise RuntimeError("remux timed out")

    if result.returncode != 0 or not os.path.exists(out_path) or os.path.getsize(out_path) == 0:
        _safe_unlink(out_path)  # don't leave a half-written file in the cache
        tail = (result.stderr or "")[-500:]
        raise RuntimeError(f"ffmpeg remux failed: {tail}")

    return {"path": out_path, "cached": False, "vcodec": vcodec, "acodec": acodec}


def _safe_unlink(path: str) -> None:
    """Remove a file, swallowing errors (used to clean up partial output)."""
    try:
        if os.path.exists(path):
            os.remove(path)
    except OSError:
        pass


# --- Single thumbnails (library cards, headset and phone tiles) -----------
#
# One persistent cache shared by the desktop (`POST /thumbnails/single`) and
# the headset / phone (`GET /api/media/thumb/{id}`), so every surface shows
# the same frame and a temp-dir purge doesn't regenerate the library.
# main.py points it at `<user-data-dir>/thumb-cache`; without a user-data
# dir (tests, bare dev runs) it falls back to the temp dir.
#
# File name: `<video id>_<stat key>_single_<marker>_<pct>_<width>[_<vr crop>].jpg`
#   video id  md5(path)[:12], the registry id (`routes.media._path_to_id`),
#             so the registration can drop files no registered video owns
#   stat key  md5(size:mtime)[:8], so a replaced file gets a new thumbnail
#   marker    THUMB_MARKER, `e` appended for exact (user-pinned) frames
# Bump THUMB_MARKER whenever the frame choice changes: v1 black thumbs, v2
# timed-out HEVC stubs, v3 software decode, v4 `-skip_frame nokey` without
# rejecting dark / blank / title-card frames; v5 rejects them (below).
THUMB_MARKER = "v5"
_thumb_cache_dir: str | None = None
_THUMB_FILE_RE = re.compile(
    r"^([0-9a-f]{12})_([0-9a-f]{8})_single_(v\d+)e?_\d{4}_\d+(?:_[a-z]+)?\.jpg$")

# Auto frames are checked on a THUMB_STATS_SIZE² grey copy decoded in the
# same ffmpeg run. A frame that is dark, blown out, flat or a title card
# (most pixels at one grey level: black or a plain background with some
# text) is retried deeper into the video; when every position fails the
# best-scoring one is kept. Pinned (exact) frames are never second-guessed.
THUMB_STATS_SIZE = 32
THUMB_DARK_MEAN = 16
THUMB_BRIGHT_MEAN = 240
THUMB_FLAT_SD = 8
THUMB_CARD_SHARE = 0.85
THUMB_CARD_NEAR = 10          # grey levels counted as "the dominant value"
THUMB_RETRY_PCTS = (0.25, 0.40, 0.60)

# VR tiles show one eye: the left half of a side-by-side frame, the top half
# of a top-bottom one. Wide-angle projections (180°, 360°, fisheye) also
# crop the middle of that eye, 70% × 50%, which stays inside a fisheye's
# image circle, so the tile is a picture rather than a distorted circle.
_EYE_CROPS = {"sbs": "crop=iw/2:ih:0:0", "tb": "crop=iw:ih/2:0:0"}
_CENTRE_CROP = "crop=trunc(iw*0.35)*2:trunc(ih*0.25)*2"
THUMB_VR_CROPS = {
    "sbs": [_EYE_CROPS["sbs"]],
    "sbsc": [_EYE_CROPS["sbs"], _CENTRE_CROP],
    "tb": [_EYE_CROPS["tb"]],
    "tbc": [_EYE_CROPS["tb"], _CENTRE_CROP],
}


def set_thumb_cache_dir(path: str | None) -> None:
    """Where generated thumbnails (and the metadata cache) live. The
    directory is created only when the first thumbnail is written."""
    global _thumb_cache_dir
    _thumb_cache_dir = path or None


def thumb_cache_dir() -> str:
    return _thumb_cache_dir or os.path.join(tempfile.gettempdir(), "funsync_thumbs")


def thumb_stat_key(video_path: str, st: os.stat_result | None = None) -> str:
    """Short hash of the file's size and mtime. Raises OSError (e.g.
    FileNotFoundError) when the file can't be statted."""
    st = st or os.stat(video_path)
    return hashlib.md5(f"{st.st_size}:{st.st_mtime_ns}".encode()).hexdigest()[:8]


def single_thumbnail_path(
    video_path: str,
    seek_pct: float = 0.1,
    width: int = 320,
    exact: bool = False,
    vr_crop: str | None = None,
    stat_key: str | None = None,
) -> str:
    """Cache file for these generation settings. Stats the video unless
    `stat_key` is given (raises OSError when it can't)."""
    video_id = hashlib.md5(video_path.encode("utf-8")).hexdigest()[:12]
    stat_key = stat_key or thumb_stat_key(video_path)
    marker = THUMB_MARKER + ("e" if exact else "")
    crop = f"_{vr_crop}" if vr_crop in THUMB_VR_CROPS else ""
    name = f"{video_id}_{stat_key}_single_{marker}_{int(seek_pct * 1000):04d}_{width}{crop}.jpg"
    return os.path.join(thumb_cache_dir(), name)


def prune_thumb_cache(keep: dict[str, set[str] | None]) -> int:
    """Delete generated thumbnails no registered video uses.

    `keep` maps every registered video's id to the file names it uses now
    (its auto or pinned frame; none when an uploaded poster replaces it),
    or to None when the video can't be statted right now (a drive spinning
    up): those keep whatever they have. Removed: thumbnails of unregistered
    videos, of an older version of a file, an older pin, VR crop or
    THUMB_MARKER, and any other JPEG in the cache (the old `<id>.jpg`
    headset names). Returns how many files were deleted."""
    root = thumb_cache_dir()
    try:
        names = os.listdir(root)
    except OSError:
        return 0
    removed = 0
    for name in names:
        if not name.endswith(".jpg"):
            continue
        m = _THUMB_FILE_RE.match(name)
        if m and m.group(1) in keep:
            used = keep[m.group(1)]
            if used is None or name in used:
                continue
        try:
            os.remove(os.path.join(root, name))
            removed += 1
        except OSError:
            pass
    return removed


def frame_stats(gray: bytes) -> dict[str, float] | None:
    """Mean, standard deviation and dominant share (fraction of pixels
    within THUMB_CARD_NEAR levels of the most common one) of 8-bit grey
    pixels; None for no pixels."""
    n = len(gray)
    if not n:
        return None
    hist = [0] * 256
    for b in gray:
        hist[b] += 1
    mean = sum(gray) / n
    sd = math.sqrt(sum(c * (v - mean) ** 2 for v, c in enumerate(hist)) / n)
    near = THUMB_CARD_NEAR
    dominant = max(sum(hist[max(0, v - near):v + near + 1]) for v in range(256)) / n
    return {"mean": mean, "sd": sd, "dominant": dominant}


def frame_problem(stats: dict[str, float] | None) -> str | None:
    """Why a frame makes a bad thumbnail ('dark', 'bright', 'flat',
    'title-card'), or None when it is fine or wasn't measured."""
    if stats is None:
        return None
    if stats["mean"] < THUMB_DARK_MEAN:
        return "dark"
    if stats["mean"] > THUMB_BRIGHT_MEAN:
        return "bright"
    if stats["sd"] < THUMB_FLAT_SD:
        return "flat"
    if stats["dominant"] >= THUMB_CARD_SHARE:
        return "title-card"
    return None


def _frame_score(stats: dict[str, float] | None) -> float:
    """Higher is a better thumbnail: contrast, spread over many grey levels."""
    if stats is None:
        return 0.0
    return stats["sd"] * (1.0 - stats["dominant"])


def _cropped_dims(width: float, height: float, vr_crop: str | None) -> tuple[float, float]:
    if vr_crop in ("sbs", "sbsc"):
        width /= 2
    elif vr_crop in ("tb", "tbc"):
        height /= 2
    if vr_crop in ("sbsc", "tbc"):
        width, height = width * 0.7, height * 0.5
    return width, height


def _grab_frame(video_path, seek_time, width, exact, vr_crop, out_path, timeout):
    """Decode one frame at `seek_time` into `out_path` (JPEG) and return its
    grey statistics (None when they couldn't be read). Raises RuntimeError
    on failure, subprocess.TimeoutExpired on a timeout."""
    chain = ",".join([*THUMB_VR_CROPS.get(vr_crop, []), f"scale={width}:-2"])
    s = THUMB_STATS_SIZE
    graph = f"[0:v]{chain},split=2[o][s];[s]scale={s}:{s}:flags=area,format=gray[g]"
    try:
        result = run_silent(
            [
                FFMPEG,
                "-v", "error",
                # `-skip_frame nokey` tells the decoder to discard all
                # non-keyframe (B/P) data. Combined with `-ss` fast-seek
                # (which already lands on/near a keyframe), the decoder
                # produces ONE keyframe near the target instead of decoding
                # forward through expensive B/P frames: for HEVC at 6K-8K
                # (typical for VR libraries) a 4-12× speedup vs plain
                # software decode, and it beats every hardware-accel variant
                # benchmarked (cuda, qsv, d3d11va), whose per-process GPU
                # init costs more than it saves on one frame. Portable: pure
                # software, identical on Windows/Linux/macOS.
                #
                # Benchmark on a 32-video VR library (H.264 1080p and HEVC
                # 6K-8K), concurrency 4, NVIDIA + Intel hybrid laptop:
                #   skip_frame nokey:  7.6s total  (0.9s avg, 5.7s max)
                #   hwaccel cuda:     33.6s total  (4.0s avg, 17.6s max)
                #   plain software:   43.8s total  (5.2s avg, 26.1s max)
                #   hwaccel auto:     62.5s total  (7.0s avg, 40.2s max)
                # See bench/bench_thumbnails.py. Must come BEFORE -i
                # (decoder option).
                #
                # Omitted in exact mode: the output must be the picked frame,
                # not the nearest keyframe. Without it, `-ss` before `-i` is
                # frame-accurate (seeks to the prior keyframe and decodes
                # forward to the timestamp).
                *([] if exact else ["-skip_frame", "nokey"]),
                # -ss BEFORE -i = fast seek via the container index.
                "-ss", str(seek_time),
                "-i", video_path,
                # One decoded frame feeds both outputs: the tile and a tiny
                # grey copy for the dark / blank / title-card check. NOTE: an
                # earlier revision used `thumbnail=N` to pick a frame; it
                # decodes N full-resolution frames and timed out on 8K HEVC
                # under the 8-way library queue. Checking one frame per
                # position costs one decode per attempt instead.
                "-filter_complex", graph,
                "-map", "[o]", "-frames:v", "1", "-q:v", "5",
                "-y", out_path,
                "-map", "[g]", "-frames:v", "1", "-f", "rawvideo", "pipe:1",
            ],
            capture_output=True,
            # Lowered from 60s so a slow/disconnected drive frees its worker
            # slot sooner (see THUMBNAIL_TIMEOUT_SEC).
            timeout=timeout,
        )
    except FileNotFoundError:
        raise RuntimeError("ffmpeg not found. Install ffmpeg to use thumbnail features.")
    if result.returncode != 0 or not os.path.exists(out_path):
        stderr = result.stderr.decode("utf-8", "replace") if isinstance(result.stderr, bytes) else result.stderr
        raise RuntimeError(f"ffmpeg single-thumbnail failed: {stderr}")
    gray = result.stdout
    if not isinstance(gray, bytes) or len(gray) != THUMB_STATS_SIZE * THUMB_STATS_SIZE:
        return None
    return frame_stats(gray)


def generate_single_thumbnail(
    video_path: str,
    seek_pct: float = 0.1,
    width: int = 320,
    exact: bool = False,
    vr_crop: str | None = None,
) -> dict[str, Any]:
    """Generate ONE thumbnail frame for a library card or headset tile.

    Cached on disk (see the cache notes above); later calls return the
    cached path immediately.

    Args:
        video_path: Absolute path to the video file.
        seek_pct: Where in the video to grab (0.0 - 1.0). Default 10%.
        width: Output width in pixels. Height auto-scales.
        exact: Grab the frame AT seek_pct exactly. Used when the user
            explicitly picked a frame ("Set thumbnail frame…") — the
            picker preview shows a frame-accurate seek, so the generated
            tile must match it (WYSIWYG). Disables the default-path
            heuristics that trade accuracy for speed or pick another
            frame: the 10s minimum seek (studio-ident avoidance),
            `-skip_frame nokey` (nearest-keyframe decode) and the bad-frame
            retries. The extra cost is decoding forward from the previous
            keyframe (one GOP), fine for a one-off user request but NOT for
            the bulk library queue, which stays on the fast default path.
        vr_crop: A THUMB_VR_CROPS key for VR video (left / top eye, plus
            the centre for wide-angle projections), or None for flat.

    Returns:
        Dict with:
            path: absolute path to generated JPEG
            duration: video duration in seconds (so caller can cache it)
            width / height: actual dimensions

    Raises:
        FileNotFoundError: video doesn't exist.
        RuntimeError: ffmpeg failed or timed out.
    """
    if not os.path.exists(video_path):
        raise FileNotFoundError(f"Video not found: {video_path}")

    # Skip 0-byte files (typically aborted/incomplete downloads). ffmpeg
    # would otherwise hang for the full subprocess timeout trying to
    # demux an empty MP4 ("moov atom not found") — the renderer's queue
    # then waits on a doomed decode while other thumbnails sit pending.
    # Treat as a missing file so the renderer falls back to its default
    # thumbnail without blocking other requests.
    if os.path.getsize(video_path) == 0:
        raise FileNotFoundError(f"Video is empty (0 bytes): {video_path}")

    vr_crop = vr_crop if vr_crop in THUMB_VR_CROPS else None
    output_path = single_thumbnail_path(video_path, seek_pct, width, exact, vr_crop)

    # Need duration to compute seek time AND to return alongside the
    # thumbnail (callers cache it on the video object). Cheap when
    # ffprobe-cached.
    metadata = get_metadata(video_path)
    duration = metadata.get("duration") or 0
    src_w, src_h = _cropped_dims(metadata.get("width") or 0, metadata.get("height") or 0, vr_crop)
    result = {
        "path": output_path,
        "duration": duration,
        "width": width,
        "height": round(width * src_h / src_w) if src_w and src_h else 0,
    }

    if os.path.exists(output_path):
        return result

    if duration <= 0:
        raise RuntimeError("Video has no duration; cannot grab thumbnail")

    # Pick a sensible seek time. Studio idents / fade-ins typically run
    # 5-10s, so for anything over a minute we clamp the seek to AT LEAST
    # 10s in (using `seek_pct` if it's later — e.g. 30s into a 5-minute
    # clip). Short clips just take the percentage mark verbatim so we
    # don't seek past the action. Inverted from the previous min(...,5s)
    # cap, which sampled directly INTO the studio ident window and
    # produced black thumbnails for most long videos.
    #
    # Exact mode: the user picked this timestamp on purpose — never
    # second-guess it (the clamp silently overrode any pick inside the
    # first 10s of a longer video).
    if exact:
        positions = [duration * seek_pct]
    else:
        first = max(10.0, duration * seek_pct) if duration > 60 else duration * seek_pct
        positions = [first, *(duration * p for p in THUMB_RETRY_PCTS)]

    # Every attempt together stays inside one THUMBNAIL_TIMEOUT_SEC budget,
    # so the retries never make a slow drive hold a worker longer.
    deadline = time.monotonic() + THUMBNAIL_TIMEOUT_SEC
    attempts = []   # (score, tmp path)
    rejected = []
    tmp_dir = tempfile.gettempdir()
    try:
        for i, seek_time in enumerate(positions):
            budget = THUMBNAIL_TIMEOUT_SEC if i == 0 else deadline - time.monotonic()
            if budget < 1:
                break
            tmp = os.path.join(tmp_dir, f"funsync-thumb-{os.getpid()}-{threading.get_ident()}-{i}.jpg")
            try:
                stats = _grab_frame(video_path, seek_time, width, exact, vr_crop, tmp, budget)
            except subprocess.TimeoutExpired:
                _safe_unlink(tmp)
                if not attempts:
                    raise RuntimeError(f"ffmpeg single-thumbnail timed out after {budget:.0f}s")
                break
            except RuntimeError:
                _safe_unlink(tmp)
                if i == 0:
                    raise
                continue
            attempts.append((_frame_score(stats), tmp))
            problem = None if exact else frame_problem(stats)
            if problem is None:
                break
            rejected.append(f"{problem} at {seek_time:.0f}s")

        score, best = max(attempts, key=lambda a: a[0])
        if rejected:
            log.info("thumbnail for %s: rejected %s; kept the frame scoring %.1f",
                     video_path, ", ".join(rejected), score)
        # The cache directory only appears once there is a thumbnail to put
        # in it; the temp name + rename means a concurrent reader never sees
        # a half-written JPEG.
        os.makedirs(os.path.dirname(output_path), exist_ok=True)
        part = f"{output_path}.{threading.get_ident()}.tmp"
        try:
            shutil.copyfile(best, part)
            os.replace(part, output_path)
        finally:
            _safe_unlink(part)
    finally:
        for _, tmp in attempts:
            _safe_unlink(tmp)

    return result


# Frame samples for matching an EroScripts preview image against the video
# (electron/preview-match.js hashes them). Samples span the middle 90% of
# the video: the first and last 5% are mostly idents, titles and credits.
#
# A preview is usually one shot of the video. Twelve evenly spaced seeks
# rarely land in that shot (a 4-minute HMV cuts every 2-3 s), so the
# sampler decodes every keyframe in the range instead: with
# `-skip_frame nokey` that is one sequential read of the file and costs
# about what twelve seeks do (1.5 s for a 150 MB 1080p file, 5 s for a
# 1 GB 6K HEVC one, on an external drive). The pass reads the whole file,
# though, and 8K VR files run to 10+ GB (3 minutes each on the user's
# drive), so files over SAMPLE_PASS_MAX_BYTES get SAMPLE_BIG_FILE_SEEKS
# single-frame seeks instead: VR scenes are long takes, where seeks do find
# the shot.
SAMPLE_EDGE_PCT = 0.05
SAMPLE_MAX_FRAMES = 400
SAMPLE_MAX_WIDTH = 512
SAMPLE_PASS_TIMEOUT_SEC = 90
SAMPLE_PASS_MAX_BYTES = 2_500_000_000
SAMPLE_BIG_FILE_SEEKS = 24


def sample_positions(count: int) -> list[float]:
    """`count` evenly spaced fractions of the duration, inside the middle 90%."""
    span = 1.0 - 2 * SAMPLE_EDGE_PCT
    return [SAMPLE_EDGE_PCT + span * (i + 0.5) / count for i in range(count)]


def _thin(items: list, limit: int) -> list:
    """At most `limit` items, evenly spread over `items`."""
    if len(items) <= limit:
        return items
    return [items[int(i * len(items) / limit)] for i in range(limit)]


def _keyframe_pass(video_path: str, duration: float, width: int, height: int) -> list[dict[str, Any]]:
    """Every keyframe of the middle 90%, as {"pct", "gray"}; [] on failure."""
    start = duration * SAMPLE_EDGE_PCT
    span = duration * (1 - 2 * SAMPLE_EDGE_PCT)
    frame_bytes = width * height
    try:
        result = run_silent(
            [
                FFMPEG,
                "-nostats", "-v", "info",
                "-skip_frame", "nokey",
                "-ss", f"{start:.3f}",
                "-t", f"{span:.3f}",
                "-i", video_path,
                "-an", "-sn", "-dn",
                "-fps_mode", "passthrough",
                "-vf", f"scale={width}:{height}:flags=area,format=gray,showinfo",
                "-f", "rawvideo",
                "pipe:1",
            ],
            capture_output=True,
            timeout=SAMPLE_PASS_TIMEOUT_SEC,
        )
    except subprocess.TimeoutExpired:
        return []
    if result.returncode != 0 or not result.stdout or len(result.stdout) % frame_bytes:
        return []
    # showinfo logs one line per frame, `pts_time:` relative to the seek.
    times = []
    for line in result.stderr.decode("utf-8", "replace").splitlines():
        if "Parsed_showinfo" in line and "pts_time:" in line:
            try:
                times.append(float(line.split("pts_time:")[1].split()[0]))
            except (IndexError, ValueError):
                times.append(None)
    count = len(result.stdout) // frame_bytes
    frames = []
    for i in range(count):
        t = times[i] if i < len(times) and times[i] is not None else span * (i + 0.5) / count
        pct = min(1.0, max(0.0, (start + t) / duration))
        frames.append({"pct": pct, "gray": result.stdout[i * frame_bytes:(i + 1) * frame_bytes]})
    return frames


def _seek_frames(video_path: str, duration: float, width: int, height: int, count: int) -> list[dict[str, Any]]:
    """`count` evenly spaced single-frame seeks; failed positions are left out."""
    frames = []
    for pct in sample_positions(count):
        try:
            result = run_silent(
                [
                    FFMPEG,
                    "-v", "error",
                    "-skip_frame", "nokey",
                    "-ss", f"{duration * pct:.3f}",
                    "-i", video_path,
                    "-frames:v", "1",
                    "-an", "-sn",
                    "-vf", f"scale={width}:{height}:flags=area,format=gray",
                    "-f", "rawvideo",
                    "pipe:1",
                ],
                capture_output=True,
                timeout=THUMBNAIL_TIMEOUT_SEC,
            )
        except subprocess.TimeoutExpired:
            continue
        if result.returncode == 0 and len(result.stdout) == width * height:
            frames.append({"pct": pct, "gray": result.stdout})
    return frames


def sample_gray_frames(video_path: str, min_count: int = 12, width: int = 128) -> dict[str, Any]:
    """Frames across the video as raw 8-bit grayscale.

    Every keyframe of the middle 90% (thinned to SAMPLE_MAX_FRAMES), or,
    when that yields fewer than `min_count` frames (long GOPs, a pass that
    failed or timed out), `min_count` evenly spaced seeks instead. Files
    over SAMPLE_PASS_MAX_BYTES skip the pass for SAMPLE_BIG_FILE_SEEKS seeks.

    Returns:
        Dict with width / height of every frame (the video scaled to
        `width`, aspect kept), duration, and frames: a list of
        {"pct": float, "gray": bytes} with width*height bytes each.

    Raises:
        FileNotFoundError: video doesn't exist or is empty.
        RuntimeError: no duration / dimensions, or no frame could be read.
    """
    if not os.path.exists(video_path) or os.path.getsize(video_path) == 0:
        raise FileNotFoundError(f"Video not found: {video_path}")
    min_count = max(1, min(int(min_count), SAMPLE_MAX_FRAMES))
    width = max(16, min(int(width), SAMPLE_MAX_WIDTH))

    metadata = get_metadata(video_path)
    duration = metadata.get("duration") or 0
    src_w = metadata.get("width") or 0
    src_h = metadata.get("height") or 0
    if duration <= 0 or not src_w or not src_h:
        raise RuntimeError("Video has no duration or dimensions; cannot sample frames")
    # Even height, as ffmpeg's `scale=W:-2` rounds it.
    height = max(2, int(round(width * src_h / src_w / 2)) * 2)

    try:
        if os.path.getsize(video_path) > SAMPLE_PASS_MAX_BYTES:
            frames = _seek_frames(video_path, duration, width, height, max(min_count, SAMPLE_BIG_FILE_SEEKS))
        else:
            frames = _keyframe_pass(video_path, duration, width, height)
            if len(frames) < min_count:
                frames = _seek_frames(video_path, duration, width, height, min_count)
    except FileNotFoundError:
        raise RuntimeError("ffmpeg not found. Install ffmpeg to use thumbnail features.")

    if not frames:
        raise RuntimeError("ffmpeg could not read any frame")
    return {"width": width, "height": height, "duration": duration, "frames": _thin(frames, SAMPLE_MAX_FRAMES)}
