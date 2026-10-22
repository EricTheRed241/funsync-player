"""`/api/media/thumb/{id}` must hand every client a real thumbnail.

INCIDENT: HereSphere requested 44 library thumbnails; 18 were not cached
yet. The endpoint queued background generation and answered those 18 with
a 1x1 transparent PNG and HTTP 200. HereSphere (and DeoVR) cache whatever
image they receive and never ask again, so those tiles stayed blank even
after the real JPEGs existed on disk.

The rules these tests defend:
  1. A cache miss waits for the real JPEG and serves it — no placeholder.
  2. If generation fails or overruns, the answer is non-200 + no-store, and
     the failure is logged with the video path.
  3. Registration warms the cache for every video whose thumbnail file is
     missing, without any client asking — and a browsing client never waits
     behind that warm-up backlog.
"""

import asyncio
import logging
import os
import sys
import threading
import time
import types

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from httpx import ASGITransport, AsyncClient  # noqa: E402

from main import app  # noqa: E402
from routes import media  # noqa: E402
import services.ffmpeg as ffmpeg_service  # noqa: E402

JPEG = b"\xff\xd8\xff\xe0" + b"\x00" * 60 + b"\xff\xd9"


@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture
async def client():
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as c:
        yield c


@pytest.fixture
def thumbs(tmp_path, monkeypatch):
    """Isolated thumb cache + a scriptable fake generator.

    `gen.calls` records (filepath, seek_pct); `gen.block[filepath]` is an
    Event the fake waits on before producing; `gen.fail` is a set of paths
    that raise instead."""
    cache = tmp_path / "thumb-cache"
    cache.mkdir()
    src_dir = tmp_path / "gen"
    src_dir.mkdir()

    monkeypatch.setattr(media, "_thumb_cache_dir", str(cache))
    monkeypatch.setattr(media, "_custom_thumbs", {})
    monkeypatch.setattr(media, "_custom_thumbs_dir", None)
    monkeypatch.setattr(media, "_queue_duration_probes", lambda: None)
    monkeypatch.setattr(media, "_queue_speed_probes", lambda: None)

    gen = types.SimpleNamespace(calls=[], block={}, fail=set())
    lock = threading.Lock()

    def fake_generate(filepath, seek_pct=0.1, width=320, exact=False):
        with lock:
            gen.calls.append((filepath, seek_pct if exact else None))
        ev = gen.block.get(filepath)
        if ev is not None:
            ev.wait(10)
        if filepath in gen.fail:
            raise RuntimeError("ffmpeg single-thumbnail failed: moov atom not found")
        out = src_dir / f"{abs(hash((filepath, seek_pct, exact)))}.jpg"
        out.write_bytes(JPEG)
        return {"path": str(out), "width": width, "height": 180}

    monkeypatch.setattr(ffmpeg_service, "generate_single_thumbnail", fake_generate)

    def make_video(name):
        p = tmp_path / name
        p.write_bytes(b"not really a video")
        return str(p)

    gen.cache = cache
    gen.make_video = make_video
    yield gen

    # Release anything still parked so no worker outlives the test blocked.
    for ev in gen.block.values():
        ev.set()
    _wait_idle()


def _wait_idle(timeout=5.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        with media._thumb_jobs_lock:
            if not media._thumb_jobs and not media._warmup_running:
                return
        time.sleep(0.01)
    raise AssertionError("thumbnail jobs still running")


def _register(paths):
    media.register_videos([{"path": p, "name": os.path.basename(p), "duration": 60} for p in paths])
    return [media._path_to_id(p) for p in paths]


@pytest.mark.anyio
async def test_cache_miss_waits_and_serves_the_real_jpeg(client, thumbs):
    video = thumbs.make_video("a.mp4")
    (vid,) = _register([video])
    thumbs.block[video] = threading.Event()
    threading.Timer(0.3, thumbs.block[video].set).start()

    # Two clients asking at once share one generation.
    r1, r2 = await asyncio.gather(
        client.get(f"/api/media/thumb/{vid}"),
        client.get(f"/api/media/thumb/{vid}"),
    )

    for r in (r1, r2):
        assert r.status_code == 200
        assert r.headers["content-type"] == "image/jpeg"
        assert r.content == JPEG, "a placeholder image was served instead of the thumbnail"
    assert thumbs.calls == [(video, None)]
    assert (thumbs.cache / f"{vid}.jpg").read_bytes() == JPEG
    assert not [f for f in os.listdir(thumbs.cache) if f.endswith(".tmp")]


@pytest.mark.anyio
async def test_cached_thumbnail_is_served_without_generating(client, thumbs):
    video = thumbs.make_video("a.mp4")
    (vid,) = _register([video])
    (thumbs.cache / f"{vid}.jpg").write_bytes(JPEG)

    r = await client.get(f"/api/media/thumb/{vid}")

    assert r.status_code == 200
    assert r.content == JPEG
    assert thumbs.calls == []


@pytest.mark.anyio
async def test_generation_failure_is_logged_and_not_cacheable(client, thumbs, caplog):
    video = thumbs.make_video("broken.mp4")
    (vid,) = _register([video])
    thumbs.fail.add(video)

    with caplog.at_level(logging.WARNING, logger="funsync.media"):
        r = await client.get(f"/api/media/thumb/{vid}")

    assert r.status_code == 503
    assert r.headers["cache-control"] == "no-store"
    assert not r.headers["content-type"].startswith("image/")
    failures = [rec.getMessage() for rec in caplog.records if rec.name == "funsync.media"]
    assert any(video in m and "moov atom not found" in m for m in failures), failures

    # Once the file is fixed, the next request gets a real thumbnail.
    thumbs.fail.clear()
    r = await client.get(f"/api/media/thumb/{vid}")
    assert r.status_code == 200
    assert r.content == JPEG


@pytest.mark.anyio
async def test_timeout_is_not_cacheable_and_the_job_still_finishes(client, thumbs, monkeypatch):
    monkeypatch.setattr(media, "THUMB_WAIT_SEC", 0.2)
    video = thumbs.make_video("slow.mp4")
    (vid,) = _register([video])
    thumbs.block[video] = threading.Event()

    r = await client.get(f"/api/media/thumb/{vid}")
    assert r.status_code == 503
    assert r.headers["cache-control"] == "no-store"

    # The timed-out waiter must not have cancelled the shared job.
    thumbs.block[video].set()
    _wait_idle()
    r = await client.get(f"/api/media/thumb/{vid}")
    assert r.status_code == 200
    assert r.content == JPEG
    assert thumbs.calls == [(video, None)]


@pytest.mark.anyio
async def test_registration_warms_only_missing_thumbnails(client, thumbs, tmp_path):
    cached, missing, pinned, poster = (
        thumbs.make_video(n) for n in ("cached.mp4", "missing.mp4", "pinned.mp4", "poster.mp4")
    )
    custom_dir = tmp_path / "custom-thumbs"
    custom_dir.mkdir()
    poster_img = custom_dir / "poster.png"
    poster_img.write_bytes(b"\x89PNG")

    cached_id = media._path_to_id(cached)
    (thumbs.cache / f"{cached_id}.jpg").write_bytes(JPEG)

    r = await client.post("/api/media/register", json={
        "videos": [{"path": p, "name": os.path.basename(p), "duration": 60}
                   for p in (cached, missing, pinned, poster)],
        "thumbCacheDir": str(thumbs.cache),
        "customThumbnails": {
            pinned: {"type": "frame", "seekPct": 0.5},
            poster: {"type": "image", "imagePath": str(poster_img)},
        },
        "customThumbsDir": str(custom_dir),
    })
    assert r.status_code == 200
    _wait_idle()

    assert sorted(thumbs.calls) == sorted([(missing, None), (pinned, 0.5)])
    assert (thumbs.cache / f"{media._path_to_id(missing)}.jpg").read_bytes() == JPEG
    assert (thumbs.cache / f"{media._path_to_id(pinned)}_p0500.jpg").read_bytes() == JPEG
    assert not (thumbs.cache / f"{media._path_to_id(poster)}.jpg").exists()

    # Re-registering the same library queues nothing new.
    assert media._queue_thumb_warmup() == 0


@pytest.mark.anyio
async def test_on_demand_request_does_not_wait_behind_the_warmup(client, thumbs):
    first, second = thumbs.make_video("first.mp4"), thumbs.make_video("second.mp4")
    _first_id, second_id = _register([first, second])
    # The warm-up worker gets stuck on the first video...
    thumbs.block[first] = threading.Event()
    assert media._queue_thumb_warmup() == 2

    # ...but a client asking for the second (still queued) gets it at once.
    started = time.perf_counter()
    r = await client.get(f"/api/media/thumb/{second_id}")
    elapsed = time.perf_counter() - started

    assert r.status_code == 200
    assert r.content == JPEG
    assert elapsed < 2.0
    thumbs.block[first].set()
    _wait_idle()
    # Each video generated exactly once despite being in both lanes.
    assert sorted(thumbs.calls) == sorted([(first, None), (second, None)])
