"""Looks at a video's frames to tell side-by-side stereo from a flat picture.

The VR decision (`routes.vr_api.detect_vr_format_for_video`) trusts the
desktop first, then the filename, then the frame shape. The frame shape
alone gets real libraries wrong (2026-11-06 scan of 459 videos): VR packed
into a 16:9 frame (5888×3312, 7680×4320 VaM renders) or under 3200 px wide
was served flat, and a 4320×2160 flat collage PMV was served as VR.

So videos the desktop and the filename say nothing about are also looked at:
four grey 128×128 frames (20/40/60/80 %), and per frame the correlation of
the left half's horizontal detail (|Δx|, so shading and letterbox bars don't
count) with the right half's, allowing ±4 px of parallax. The second-lowest
frame score decides, so one title card or fade can't. On that library every
side-by-side video scored ≥ 0.35 and every flat one ≤ 0.17, hence:

    ≥ STEREO_MIN → 'sbs'   (two eyes side by side)
    ≤ MONO_MAX   → 'mono'  (one picture)
    in between, too few usable frames, or ffmpeg failed → None (undecided)

One exception: a picture centred between dark side bars that each fill
≥ SIDE_BAR_MIN of the width (a square video in a 2:1 frame) is 'mono' — its
two halves mirror each other's bars and fool the correlation.

Top-bottom stereo is not detected: on the same library it didn't separate
from flat (the one top-bottom video scored below three flat ones).

Results are kept per path with the file's size + mtime in
`<userData>/vr-stereo-cache.json` and filled by one background thread
(`queue_checks`, run at registration); `cached_layout` never touches the
disk, so the listings can call it per video.
"""

import collections
import json
import logging
import os
import subprocess
import threading

from services.ffmpeg import FFMPEG, FFPROBE

log = logging.getLogger("funsync.stereo_detect")

CACHE_FILE_NAME = "vr-stereo-cache.json"
# Bump when the analysis changes so old results are looked at again.
ANALYSIS_VERSION = 1
SIZE = 128
FRACTIONS = (0.2, 0.4, 0.6, 0.8)
STEREO_MIN = 0.3
MONO_MAX = 0.2
SIDE_BAR_MIN = 0.2
SHIFTS = range(-4, 5)
FRAME_TIMEOUT_SEC = 60

_lock = threading.Lock()
_cache_path = None
_store: dict[str, dict] = {}      # path → {size, mtimeNs, layout, version}
_layouts: dict[str, str | None] = {}  # path → layout, only for current entries
_queue: collections.deque = collections.deque()
_queued: set[str] = set()
_worker_running = False


# --- analysis (pure) ----------------------------------------------------------

def _corr(a, b):
    n = len(a)
    if n == 0:
        return None
    ma, mb = sum(a) / n, sum(b) / n
    va = sum((x - ma) ** 2 for x in a)
    vb = sum((y - mb) ** 2 for y in b)
    if va <= 1e-9 or vb <= 1e-9:
        return None
    return sum((x - ma) * (y - mb) for x, y in zip(a, b)) / (va * vb) ** 0.5


def _side_bars(px, w, h):
    """Fraction of the width the dark, flat bars fill on the left / right."""
    def dead(x):
        col = [px[y * w + x] for y in range(h)]
        return max(col) - min(col) <= 12 and sum(col) / h <= 24
    left = 0
    while left < w and dead(left):
        left += 1
    right = 0
    while right < w and dead(w - 1 - right):
        right += 1
    return left / w, right / w


def frame_score(px, w=SIZE, h=SIZE):
    """Side-by-side score for one grey frame (bytes, w×h), or None when the
    frame is too flat to say (black, a fade). 'mono' when the picture sits
    between wide side bars."""
    if max(px) - min(px) < 40:
        return None
    left, right = _side_bars(px, w, h)
    if min(left, right) >= SIDE_BAR_MIN:
        return 'mono'
    grad = []
    for y in range(h):
        row = px[y * w:(y + 1) * w]
        grad.extend(abs(row[x + 1] - row[x]) for x in range(w - 1))
        grad.append(0)
    half = w // 2
    best = None
    for shift in SHIFTS:
        a, b = [], []
        for y in range(h):
            base = y * w
            for x in range(6, half - 6):
                a.append(grad[base + x])
                b.append(grad[base + half + x + shift])
        c = _corr(a, b)
        if c is not None and (best is None or c > best):
            best = c
    return best


def layout_from_scores(scores):
    """'sbs' / 'mono' / None from per-frame scores (see the module docstring)."""
    usable = [s for s in scores if s is not None]
    if any(s == 'mono' for s in usable):
        bars = sum(1 for s in usable if s == 'mono')
        if bars * 2 >= len(usable):
            return 'mono'
        usable = [s for s in usable if s != 'mono']
    if len(usable) < 2:
        return None
    decisive = sorted(usable)[1]
    if decisive >= STEREO_MIN:
        return 'sbs'
    if decisive <= MONO_MAX:
        return 'mono'
    return None


# --- ffmpeg ---------------------------------------------------------------------

def _duration(path):
    try:
        r = subprocess.run([FFPROBE, '-v', 'error', '-show_entries', 'format=duration',
                            '-of', 'default=nw=1:nk=1', path],
                           capture_output=True, text=True, timeout=FRAME_TIMEOUT_SEC)
        return float(r.stdout.strip())
    except (OSError, ValueError, subprocess.SubprocessError):
        return 0.0


def _frame(path, at):
    try:
        r = subprocess.run([FFMPEG, '-v', 'error', '-ss', f'{at:.2f}', '-i', path, '-frames:v', '1',
                            '-vf', f'scale={SIZE}:{SIZE}', '-f', 'rawvideo', '-pix_fmt', 'gray', '-'],
                           capture_output=True, timeout=FRAME_TIMEOUT_SEC)
    except (OSError, subprocess.SubprocessError):
        return None
    return r.stdout if len(r.stdout) == SIZE * SIZE else None


def analyze(path, duration=0.0):
    """'sbs' / 'mono' / None for one video file. Blocking (ffmpeg)."""
    duration = duration or _duration(path)
    if duration <= 0:
        return None
    frames = [_frame(path, duration * f) for f in FRACTIONS]
    return layout_from_scores([frame_score(f) for f in frames if f])


# --- cache + background worker ----------------------------------------------------

def _sig(path):
    try:
        st = os.stat(path)
    except OSError:
        return None
    return st.st_size, st.st_mtime_ns


def set_cache_dir(user_data_dir):
    """Persist results in `<user_data_dir>/vr-stereo-cache.json` and load it."""
    global _cache_path
    _cache_path = os.path.join(user_data_dir, CACHE_FILE_NAME)
    try:
        with open(_cache_path, 'r', encoding='utf-8') as f:
            data = json.load(f)
        entries = data.get('entries') if isinstance(data, dict) else None
        if isinstance(entries, dict):
            with _lock:
                _store.update({p: e for p, e in entries.items()
                               if isinstance(e, dict) and e.get('version') == ANALYSIS_VERSION})
    except FileNotFoundError:
        pass
    except (OSError, ValueError) as e:
        log.warning("could not read %s: %s", _cache_path, e)


def _save():
    if not _cache_path:
        return
    with _lock:
        body = json.dumps({'version': 1, 'entries': _store})
    tmp = _cache_path + '.tmp'
    try:
        with open(tmp, 'w', encoding='utf-8') as f:
            f.write(body)
        os.replace(tmp, _cache_path)
    except OSError as e:
        log.warning("could not write %s: %s", _cache_path, e)


def cached_layout(path):
    """'sbs' / 'mono' / None (undecided or not looked at yet). Never touches
    the disk."""
    return _layouts.get(path) if path else None


def queue_checks(targets):
    """Look at `targets` (`(path, duration)` pairs) in the background. Paths
    with a current cache entry are answered from it straight away; the rest
    replace the previous queue. Returns how many were queued."""
    global _worker_running
    todo = []
    for path, duration in targets:
        sig = _sig(path)
        if sig is None:
            continue
        with _lock:
            e = _store.get(path)
            if e and (e.get('size'), e.get('mtimeNs')) == sig:
                _layouts[path] = e.get('layout')
                continue
        todo.append((path, duration or 0.0))
    with _lock:
        _queue.clear()
        _queued.clear()
        for item in todo:
            _queue.append(item)
            _queued.add(item[0])
        queued = len(_queue)
        if _queue and not _worker_running:
            _worker_running = True
            threading.Thread(target=_worker, name='stereo-detect', daemon=True).start()
    if queued:
        log.info("stereo check queued %d video(s)", queued)
    return queued


def _worker():
    global _worker_running
    done = 0
    while True:
        with _lock:
            if not _queue:
                _worker_running = False
                break
            path, duration = _queue.popleft()
            _queued.discard(path)
        sig = _sig(path)
        if sig is None:
            continue
        layout = analyze(path, duration)
        with _lock:
            _store[path] = {'size': sig[0], 'mtimeNs': sig[1], 'layout': layout, 'version': ANALYSIS_VERSION}
            _layouts[path] = layout
        done += 1
        if layout:
            log.info("stereo check: %s → %s", os.path.basename(path), layout)
        if done % 20 == 0:
            _save()
    _save()


def _reset_for_tests():
    global _cache_path, _worker_running
    with _lock:
        _store.clear()
        _layouts.clear()
        _queue.clear()
        _queued.clear()
        _cache_path = None
        _worker_running = False


def _set_layout_for_tests(path, layout):
    _layouts[path] = layout
