"""Resolution/codec cache behind VR auto-detect and the VR scene endpoints.

An untagged VR master (no 180/SBS/studio token in its filename) is only put
in HereSphere's / DeoVR's "VR" section when its dimensions are known
(`routes.vr_api._dims_suggest_vr`). The library listings never probe, so
the dimensions have to be known before a headset asks for the library:

  * Registration (`queue_warmup`, called from `POST /api/media/register`)
    stats every registered video and fills the in-memory cache from the
    persisted file for each one whose size + mtime still match. Whatever is
    left goes to a single background ffprobe worker — no probe storm, and
    the request never waits on it.
  * Scene detail (`get_video_info`) probes the one requested video directly
    on its own thread, so a headset opening a scene never queues behind the
    warm-up backlog.
  * Every probe result is written to `<user-data-dir>/vr-dims-cache.json`
    (atomic tmp + rename), keyed by path and validated against the file's
    size and mtime, so a backend restart keeps the VR classification and a
    replaced file is probed again.
"""

import atexit
import collections
import json
import logging
import os
import stat
import threading
import time

log = logging.getLogger("funsync.video_info")

CACHE_FILE_NAME = "vr-dims-cache.json"
CACHE_VERSION = 1
# ~120 bytes/entry → 20000 entries ≈ 2.4 MB ceiling. Oldest-written pruned
# first on save, so entries for deleted/renamed files age out.
CACHE_MAX_ENTRIES = 20000
# The warm-up writes the file every N new results (and when it drains) so a
# hard kill mid-pass loses at most this many probes.
SAVE_BATCH = 20

# video_id → (width, height, codec). Read lock-free by the library listings;
# only ever holds results validated against the file's current size/mtime.
_video_info_cache: dict[str, tuple[int, int, str]] = {}

_lock = threading.Lock()                  # guards everything below
_save_lock = threading.Lock()             # serialises writes; taken BEFORE _lock
_cache_path: str | None = None
_store: dict[str, dict] = {}              # path → {size, mtimeNs, width, height, codec}
_dirty = 0                                # store changes not yet on disk
_failed: dict[str, tuple[int, int]] = {}  # path → (size, mtime_ns) ffprobe couldn't read
_queue: collections.deque = collections.deque()
_pending: set[str] = set()                # video IDs in _queue
_in_flight: str | None = None             # video ID the worker is probing
_worker_running = False


def probe_video_info(path):
    """Run ffprobe for `(width, height, codec)`, or None. Blocking."""
    try:
        from services.ffmpeg import _find_binary, run_silent
        ffprobe = _find_binary('ffprobe')
        result = run_silent(
            [ffprobe, '-v', 'quiet', '-print_format', 'json',
             '-show_streams', '-select_streams', 'v:0', path],
            capture_output=True, text=True,
            # Windows reader-thread crashes with UnicodeDecodeError when it
            # falls back to cp1252 — pin to utf-8 with replacement so
            # unmappable codec/path bytes don't kill the probe.
            encoding='utf-8', errors='replace',
            timeout=10,
        )
        if result.returncode == 0:
            data = json.loads(result.stdout)
            streams = data.get('streams', [])
            if streams:
                w = int(streams[0].get('width', 1920))
                h = int(streams[0].get('height', 1080))
                codec = streams[0].get('codec_name', 'h264')
                if codec in ('hevc', 'h265'):
                    codec = 'h265'
                elif codec in ('h264', 'avc', 'avc1'):
                    codec = 'h264'
                return w, h, codec
    except Exception:
        pass
    return None


def set_cache_dir(user_data_dir: str | None) -> None:
    """Point the persisted cache at `user_data_dir` and load it. Called once
    at startup with `--user-data-dir`; without one the cache is memory-only.
    A missing, unreadable or wrong-version file just means an empty cache."""
    global _cache_path, _dirty
    with _lock:
        _store.clear()
        _dirty = 0
        _cache_path = os.path.join(user_data_dir, CACHE_FILE_NAME) if user_data_dir else None
        if not _cache_path:
            return
        try:
            with open(_cache_path, encoding="utf-8") as f:
                payload = json.load(f)
        except (OSError, ValueError):
            return
        if not isinstance(payload, dict) or payload.get("version") != CACHE_VERSION:
            return
        entries = payload.get("entries")
        if not isinstance(entries, dict):
            return
        for path, e in entries.items():
            if _valid_entry(e):
                _store[path] = e
    log.info("loaded %d cached video resolution(s) from %s", len(_store), _cache_path)


def _valid_entry(e) -> bool:
    return (isinstance(e, dict)
            and all(isinstance(e.get(k), int) for k in ("size", "mtimeNs", "width", "height"))
            and e["width"] > 0 and e["height"] > 0
            and isinstance(e.get("codec"), str))


def cached_video_info(video_id):
    """`(width, height, codec)` if already known, else None. Never touches
    the disk — safe to call per video from the library listings."""
    return _video_info_cache.get(video_id)


def _file_sig(path):
    """`(size, mtime_ns)` of a regular file, or None if it isn't one."""
    try:
        st = os.stat(path)
    except (OSError, ValueError):
        return None
    if not stat.S_ISREG(st.st_mode):
        return None
    return st.st_size, st.st_mtime_ns


def _stored_info_locked(path, sig):
    """Persisted result for `path` if it was probed at this size/mtime."""
    e = _store.get(path)
    if e is not None and (e["size"], e["mtimeNs"]) == sig:
        return e["width"], e["height"], e["codec"]
    return None


def _resolve(video_id, path, sig):
    """Cached result or a fresh probe for one video; records either way.
    Returns `(info, probed)`. Blocking."""
    global _dirty
    with _lock:
        info = _stored_info_locked(path, sig)
        failed = _failed.get(path) == sig
    if info is not None:
        _video_info_cache[video_id] = info
        return info, False
    if failed:
        return None, False
    info = probe_video_info(path)
    with _lock:
        if info is None:
            _failed[path] = sig
        else:
            _failed.pop(path, None)
            _store.pop(path, None)   # re-insert last: pruning drops oldest writes
            _store[path] = {"size": sig[0], "mtimeNs": sig[1],
                            "width": int(info[0]), "height": int(info[1]), "codec": str(info[2])}
            _dirty += 1
    if info is not None:
        _video_info_cache[video_id] = info
    return info, True


def get_video_info(video_id, path):
    """On-demand lane (scene detail): `(width, height, codec)` for one video,
    probing it right now if nothing valid is cached. None if it can't be
    read. Blocking — call via `asyncio.to_thread`."""
    info = _video_info_cache.get(video_id)
    if info is not None:
        return info
    sig = _file_sig(path) if path else None
    if sig is None:
        return None
    info, probed = _resolve(video_id, path, sig)
    if probed:
        save()
    return info


def queue_warmup(targets) -> int:
    """Warm-up lane, run at registration with every registered video lacking
    dimensions as `(video_id, path)` pairs, most important first.

    Synchronously (stat only, no probe) refreshes the in-memory cache from
    the persisted file, so a restarted backend classifies everything it has
    seen before as soon as the library is registered. Videos with no valid
    entry are queued for the single background worker, replacing the
    previous queue — the registry is a full snapshot, so videos that left it
    are dropped. Returns the number queued."""
    global _worker_running
    to_probe = []
    for video_id, path in targets:
        sig = _file_sig(path) if path else None
        if sig is None:
            continue
        with _lock:
            info = _stored_info_locked(path, sig)
            failed = _failed.get(path) == sig
        if info is not None:
            _video_info_cache[video_id] = info
            continue
        # Missing, or probed before the file was replaced.
        _video_info_cache.pop(video_id, None)
        if not failed:
            to_probe.append((video_id, path))

    with _lock:
        _queue.clear()
        _pending.clear()
        for video_id, path in to_probe:
            if video_id == _in_flight or video_id in _pending:
                continue
            _pending.add(video_id)
            _queue.append((video_id, path))
        queued = len(_queue)
        if _queue and not _worker_running:
            _worker_running = True
            threading.Thread(target=_warmup_worker, name="dims-warmup", daemon=True).start()
    if queued:
        log.info("resolution warm-up queued %d video(s)", queued)
    return queued


def _warmup_worker():
    """Drain the warm-up queue one probe at a time; exits when it is empty
    (restarted by the next `queue_warmup`)."""
    global _worker_running, _in_flight
    started = time.monotonic()
    probed = 0
    while True:
        with _lock:
            if not _queue:
                _worker_running = False
                _in_flight = None
                break
            video_id, path = _queue.popleft()
            _pending.discard(video_id)
            _in_flight = video_id
            dirty = _dirty
        if dirty >= SAVE_BATCH:
            save()
        if video_id in _video_info_cache:
            continue   # the scene-detail lane got there first
        sig = _file_sig(path)
        if sig is None:
            continue
        try:
            _, did_probe = _resolve(video_id, path, sig)
            probed += did_probe
        except Exception:
            log.exception("resolution probe failed for %s", path)
    save()
    if probed:
        log.info("resolution warm-up probed %d video(s) in %.1fs", probed, time.monotonic() - started)


def save() -> None:
    """Write pending results to disk atomically (tmp + rename). Failures are
    logged and the results stay pending for the next save."""
    global _dirty
    with _save_lock:
        with _lock:
            if not _cache_path or _dirty == 0:
                return
            for path in list(_store)[:max(0, len(_store) - CACHE_MAX_ENTRIES)]:
                del _store[path]
            snapshot = {"version": CACHE_VERSION, "entries": dict(_store)}
            cache_path = _cache_path
            written = _dirty
            _dirty = 0
        tmp_path = f"{cache_path}.tmp-{os.getpid()}"
        try:
            os.makedirs(os.path.dirname(cache_path), exist_ok=True)
            with open(tmp_path, "w", encoding="utf-8") as f:
                json.dump(snapshot, f, ensure_ascii=False)
            os.replace(tmp_path, cache_path)
        except OSError as e:
            log.warning("could not write %s: %s", cache_path, e)
            with _lock:
                _dirty += written


atexit.register(save)


def _reset_for_tests() -> None:
    """Drop all state (simulates a restart); the file on disk is untouched."""
    global _cache_path, _dirty, _in_flight
    with _lock:
        _video_info_cache.clear()
        _store.clear()
        _failed.clear()
        _queue.clear()
        _pending.clear()
        _cache_path = None
        _dirty = 0
        _in_flight = None
