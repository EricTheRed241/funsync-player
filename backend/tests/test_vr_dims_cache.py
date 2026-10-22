"""Untagged VR masters must be in the headset's "VR Videos" section from the
first library load, and stay there across backend restarts.

INCIDENT: Cuddle Mocap files (`CM 045 - Date Night Sex With Joni_6k.mp4`,
5888x2944) carry no VR token, so only their dimensions mark them as VR. The
dimensions were known only after a scene had been opened once in the
headset, and only until the backend restarted: 24 entries in the VR section
instead of 40.

The rules these tests defend:
  1. Registration probes the dimensions of every registered video in the
     background, untagged ones first.
  2. Results persist to the user-data dir, keyed by path + size + mtime; a
     restarted backend classifies from the file without probing, and a
     replaced file is probed again.
  3. The library listings never probe.
  4. Filename tags and manual overrides keep precedence.
"""

import json
import os
import sys
import threading
import time

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from httpx import ASGITransport, AsyncClient  # noqa: E402

from main import app  # noqa: E402
from routes import media  # noqa: E402
from services import video_info  # noqa: E402

VR_DIMS = (5888, 2944, "h265")
FLAT_DIMS = (1920, 1080, "h264")


@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture
async def client():
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as c:
        yield c


class FakeProbe:
    def __init__(self):
        self.dims = {}        # path → (w, h, codec)
        self.calls = []
        self.lock = threading.Lock()

    def __call__(self, path):
        with self.lock:
            self.calls.append(path)
        return self.dims.get(path)


@pytest.fixture
def probe(monkeypatch, tmp_path):
    video_info._reset_for_tests()
    fake = FakeProbe()
    monkeypatch.setattr(video_info, "probe_video_info", fake)
    monkeypatch.setattr(media, "_thumb_cache_dir", None)
    monkeypatch.setattr(media, "_queue_duration_probes", lambda: None)
    monkeypatch.setattr(media, "_queue_speed_probes", lambda: None)
    fake.user_data = tmp_path / "userData"
    fake.user_data.mkdir()
    video_info.set_cache_dir(str(fake.user_data))
    yield fake
    _wait_idle()
    video_info._reset_for_tests()


def _wait_idle(timeout=5.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        with video_info._lock:
            if not video_info._worker_running:
                return
        time.sleep(0.01)
    raise AssertionError("resolution warm-up still running")


def _restart(user_data):
    """Simulate a backend restart: all in-memory state gone, file kept."""
    _wait_idle()
    video_info._reset_for_tests()
    video_info.set_cache_dir(str(user_data))


def _make(tmp_path, name, dims, probe):
    p = tmp_path / name
    p.write_bytes(b"not really a video")
    if dims:
        probe.dims[str(p)] = dims
    return str(p)


async def _register(client, videos):
    r = await client.post("/api/media/register", json={"videos": videos})
    assert r.status_code == 200


def _video(path, **extra):
    return {"path": path, "name": os.path.basename(path), "duration": 600, **extra}


async def _heresphere_vr(client):
    lib = (await client.get("/heresphere")).json()["library"]
    vr = next((g["list"] for g in lib if "VR Videos" in g["name"]), [])
    return {url.rsplit("/", 1)[1] for url in vr}


async def _deovr_vr(client):
    scenes = (await client.get("/deovr")).json()["scenes"]
    vr = next((g["list"] for g in scenes if g["name"] == "VR Videos"), [])
    return {item["video_url"].rsplit("/", 1)[1] for item in vr}


@pytest.mark.anyio
async def test_registration_probes_dims_and_fills_the_vr_section(client, probe, tmp_path):
    vr1 = _make(tmp_path, "CM 045 - Date Night Sex With Joni_6k.mp4", VR_DIMS, probe)
    vr2 = _make(tmp_path, "CM 046 - Morning_6k.mp4", VR_DIMS, probe)
    flat = _make(tmp_path, "Holiday.mp4", FLAT_DIMS, probe)
    forced_flat = _make(tmp_path, "CM 047 - Forced Flat_6k.mp4", VR_DIMS, probe)
    tagged = _make(tmp_path, "Scene_180_sbs.mp4", VR_DIMS, probe)

    await _register(client, [
        _video(vr1), _video(vr2), _video(flat),
        _video(forced_flat, manualVRType="flat"), _video(tagged),
    ])
    _wait_idle()

    ids = {p: media._path_to_id(p) for p in (vr1, vr2, flat, forced_flat, tagged)}
    expected = {ids[vr1], ids[vr2], ids[tagged]}
    assert await _heresphere_vr(client) == expected
    assert await _deovr_vr(client) == expected

    # Every video was probed once; the ones whose VR status depends on their
    # dimensions (untagged, no override) went first.
    assert sorted(probe.calls) == sorted(ids)
    assert set(probe.calls[:3]) == {vr1, vr2, flat}

    # Re-registering the same library probes nothing new.
    await _register(client, [_video(vr1), _video(vr2), _video(flat),
                             _video(forced_flat, manualVRType="flat"), _video(tagged)])
    _wait_idle()
    assert len(probe.calls) == 5


@pytest.mark.anyio
async def test_persisted_cache_survives_a_restart_without_probing(client, probe, tmp_path):
    vr = _make(tmp_path, "CM 045 - Date Night Sex With Joni_6k.mp4", VR_DIMS, probe)
    flat = _make(tmp_path, "Holiday.mp4", FLAT_DIMS, probe)
    await _register(client, [_video(vr), _video(flat)])
    _wait_idle()

    saved = json.loads((probe.user_data / video_info.CACHE_FILE_NAME).read_text())
    assert saved["entries"][vr]["width"] == 5888
    assert saved["entries"][vr]["height"] == 2944
    assert saved["entries"][vr]["size"] == os.path.getsize(vr)
    assert saved["entries"][vr]["mtimeNs"] == os.stat(vr).st_mtime_ns

    _restart(probe.user_data)
    probe.calls.clear()

    await _register(client, [_video(vr), _video(flat)])
    # No waiting on the worker: a valid persisted entry is applied during
    # registration itself, so the very first listing already has it.
    assert await _heresphere_vr(client) == {media._path_to_id(vr)}
    assert await _deovr_vr(client) == {media._path_to_id(vr)}
    _wait_idle()
    assert probe.calls == []


@pytest.mark.anyio
async def test_replaced_file_is_probed_again(client, probe, tmp_path):
    path = _make(tmp_path, "CM 050 - Replaced.mp4", VR_DIMS, probe)
    await _register(client, [_video(path)])
    _wait_idle()
    vid = media._path_to_id(path)
    assert await _heresphere_vr(client) == {vid}

    _restart(probe.user_data)
    # Same path, different file: a flat re-encode with a new size + mtime.
    with open(path, "ab") as f:
        f.write(b"more bytes")
    st = os.stat(path)
    os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns + 5_000_000_000))
    probe.dims[path] = FLAT_DIMS
    probe.calls.clear()

    await _register(client, [_video(path)])
    _wait_idle()
    assert probe.calls == [path]
    assert await _heresphere_vr(client) == set()
    saved = json.loads((probe.user_data / video_info.CACHE_FILE_NAME).read_text())
    assert saved["entries"][path]["width"] == 1920
    assert saved["entries"][path]["size"] == os.path.getsize(path)


@pytest.mark.anyio
async def test_stale_entry_in_the_same_session_is_dropped_and_reprobed(client, probe, tmp_path):
    path = _make(tmp_path, "CM 051 - Swapped.mp4", VR_DIMS, probe)
    await _register(client, [_video(path)])
    _wait_idle()

    with open(path, "ab") as f:
        f.write(b"x")
    probe.dims[path] = FLAT_DIMS
    await _register(client, [_video(path)])
    _wait_idle()
    assert probe.calls == [path, path]
    assert await _deovr_vr(client) == set()


@pytest.mark.anyio
async def test_listing_never_probes(client, probe, tmp_path, monkeypatch):
    vr = _make(tmp_path, "CM 045 - Date Night Sex With Joni_6k.mp4", VR_DIMS, probe)
    # Registered without the registration hook, so nothing is known yet.
    media.register_videos([_video(vr)])

    def no_stat(*_a, **_k):
        raise AssertionError("library listing touched the file system")

    monkeypatch.setattr(video_info, "_file_sig", no_stat)
    assert await _heresphere_vr(client) == set()
    assert await _deovr_vr(client) == set()
    assert (await client.get("/heresphere?filter=vr")).status_code == 200
    assert (await client.get("/deovr?filter=flat")).status_code == 200
    assert probe.calls == []


@pytest.mark.anyio
async def test_manual_vr_override_and_tags_need_no_dims(client, probe, tmp_path):
    forced_vr = _make(tmp_path, "Holiday Forced.mp4", None, probe)  # probe fails
    tagged = _make(tmp_path, "Scene_180_sbs.mp4", None, probe)
    await _register(client, [_video(forced_vr, manualVRType="vr"), _video(tagged)])
    _wait_idle()
    assert await _heresphere_vr(client) == {media._path_to_id(forced_vr), media._path_to_id(tagged)}

    # A file ffprobe can't read isn't retried on every registration.
    probe.calls.clear()
    await _register(client, [_video(forced_vr, manualVRType="vr"), _video(tagged)])
    _wait_idle()
    assert probe.calls == []


@pytest.mark.anyio
async def test_scene_detail_probe_is_persisted(client, probe, tmp_path):
    vr = _make(tmp_path, "CM 052 - Opened.mp4", VR_DIMS, probe)
    media.register_videos([_video(vr)])
    vid = media._path_to_id(vr)

    data = (await client.get(f"/heresphere/{vid}")).json()
    assert data["projection"] == "fisheye"
    assert probe.calls == [vr]

    _restart(probe.user_data)
    await _register(client, [_video(vr)])
    _wait_idle()
    assert probe.calls == [vr]
    assert await _heresphere_vr(client) == {vid}


def test_unreadable_cache_file_is_ignored(probe):
    (probe.user_data / video_info.CACHE_FILE_NAME).write_text("{ not json")
    _restart(probe.user_data)
    assert video_info._store == {}
