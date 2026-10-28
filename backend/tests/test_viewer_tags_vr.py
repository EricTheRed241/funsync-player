"""Chip tags, preference scores and the VR decision in every viewer.

The desktop registers `vrFormat` (the VR Format panel's projection per
video), `scriptTags` (EroScripts / FapTap post tags per video) and
`preferenceScores` (the desktop's preference score per video). The
backend decides VR from the projection first, then `manualVRType`, the
filename and the frame shape, reports it at `GET /api/media/vr-flags`, and
hands the same decision plus the chip tags and the score to the web remote
(`/api/remote/videos`) and the headsets (`Tag:` / `Category:` /
`Playlist:` / `Format:` / `Score:` scene tags).
"""

import os
import threading
import time

import pytest
from httpx import ASGITransport, AsyncClient

from main import app
from routes import media
from routes.media import (
    register_categories,
    register_playlists,
    register_preference_scores,
    register_script_tags,
    register_vr_format,
)
from services import video_info
from services.video_tags import chip_tags
from services.vr_sections import score_tags

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
        self.dims = {}
        self.lock = threading.Lock()

    def __call__(self, path):
        with self.lock:
            return self.dims.get(path)


def _wait_idle(timeout=5.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        with video_info._lock:
            if not video_info._worker_running:
                return
        time.sleep(0.01)
    raise AssertionError("resolution warm-up still running")


@pytest.fixture(autouse=True)
def probe(monkeypatch, tmp_path):
    video_info._reset_for_tests()
    fake = FakeProbe()
    monkeypatch.setattr(video_info, "probe_video_info", fake)
    monkeypatch.setattr(media, "_thumb_cache_dir", None)
    monkeypatch.setattr(media, "_queue_duration_probes", lambda: None)
    monkeypatch.setattr(media, "_queue_speed_probes", lambda: None)
    user_data = tmp_path / "userData"
    user_data.mkdir()
    video_info.set_cache_dir(str(user_data))
    _reset_groupings()
    yield fake
    _wait_idle()
    video_info._reset_for_tests()
    _reset_groupings()


def _reset_groupings():
    register_vr_format({})
    register_script_tags({})
    register_preference_scores({})
    register_categories([], {})
    register_playlists([])


def _make(tmp_path, probe, name, dims):
    p = tmp_path / name
    p.write_bytes(b"not really a video")
    if dims:
        probe.dims[str(p)] = dims
    return str(p)


def _video(path, **extra):
    return {"path": path, "name": os.path.basename(path), "duration": 600, **extra}


async def _register(client, payload):
    r = await client.post("/api/media/register", json=payload)
    assert r.status_code == 200
    _wait_idle()


async def _remote_videos(client):
    return {v["path"]: v for v in (await client.get("/api/remote/videos")).json()["videos"]}


# --- chip tags --------------------------------------------------------------

class TestChipTags:
    def test_noise_is_dropped(self):
        raw = [
            "creampie", "len-5-10-min", "LEN-30", "the-handy", "the-handy-2-pro", "osr2", "sr6",
            "funsr-pro", "syncbot", "multi-axis", "twist", "non-vr", "vr", "VR", "no-free-links",
            "straight", "riding",
        ]
        assert chip_tags(raw) == ["creampie", "riding"]

    def test_case_insensitive_dedupe_sorted(self):
        assert chip_tags([" Riding", "riding", "Anal", "", "  ", None, 7]) == ["anal", "riding"]

    def test_empty(self):
        assert chip_tags(None) == []
        assert chip_tags([]) == []


# --- registration -------------------------------------------------------------

@pytest.mark.anyio
async def test_register_stores_vr_format_and_script_tags(client, probe, tmp_path):
    a = _make(tmp_path, probe, "A.mp4", FLAT_DIMS)
    await _register(client, {
        "videos": [_video(a)],
        "vrFormat": {a: {"projection": "sbs-half", "eye": "left"}, "/gone.mp4": {"eye": "left"}},
        "scriptTags": {a: ["riding", "the-handy", "len-10"], "/noise.mp4": ["vr", "osr2"], "/bad.mp4": "riding"},
    })
    assert media.get_vr_projection(a) == "sbs-half"
    assert media.get_vr_projection("/gone.mp4") is None  # no projection, dropped
    assert media.get_chip_tags(a) == ["riding"]
    assert media._chip_tags == {a: ["riding"]}  # noise-only and non-list entries dropped

    # Partial payloads (groupings, triage state) leave both untouched.
    await _register(client, {"ratings": {}, "playlists": []})
    assert media.get_vr_projection(a) == "sbs-half"
    assert media.get_chip_tags(a) == ["riding"]

    # Registering again with the same data is idempotent; a new map replaces.
    await _register(client, {"vrFormat": {a: {"projection": "sbs-half"}}, "scriptTags": {a: ["riding"]}})
    assert media.get_chip_tags(a) == ["riding"]
    await _register(client, {"vrFormat": {}, "scriptTags": {}})
    assert media.get_vr_projection(a) is None
    assert media.get_chip_tags(a) == []


@pytest.mark.anyio
async def test_old_payload_without_new_fields_still_works(client, probe, tmp_path):
    a = _make(tmp_path, probe, "Scene_180_LR.mp4", None)
    r = await client.post("/api/media/register", json={"videos": [_video(a)]})
    assert r.status_code == 200 and r.json()["registered"] == 1
    _wait_idle()
    v = (await _remote_videos(client))[a]
    assert (v["isVR"], v["tags"], v["categories"]) == (True, [], [])


# --- VR decision ----------------------------------------------------------------

@pytest.mark.anyio
async def test_vr_flags_filename_dims_and_vr_format(client, probe, tmp_path):
    named = _make(tmp_path, probe, "x_180_LR.mp4", None)                      # filename
    shaped = _make(tmp_path, probe, "CM 045 - Joni_6k.mp4", VR_DIMS)          # frame shape
    flat = _make(tmp_path, probe, "Holiday.mp4", FLAT_DIMS)                   # flat by shape
    forced_flat = _make(tmp_path, probe, "Scene_180_sbs.mp4", None)           # vrFormat flat wins
    forced_vr = _make(tmp_path, probe, "Plain.mp4", FLAT_DIMS)                # vrFormat VR wins
    unknown = _make(tmp_path, probe, "Unprobeable.mp4", None)                 # dims never known
    await _register(client, {
        "videos": [_video(p) for p in (named, shaped, flat, forced_flat, forced_vr, unknown)],
        "vrFormat": {forced_flat: {"projection": "flat"}, forced_vr: {"projection": "equirect-180"}},
    })

    body = (await client.get("/api/media/vr-flags")).json()
    assert body["flags"] == {
        named: True, shaped: True, flat: False, forced_flat: False, forced_vr: True,
    }
    assert body["pending"] == 1  # `unknown` left out until its dimensions are known

    # The listings use the same decision.
    lib = (await client.get("/heresphere?filter=vr")).json()["library"]
    vr_ids = {url.rsplit("/", 1)[1] for g in lib for url in g["list"]}
    assert vr_ids == {media._path_to_id(p) for p in (named, shaped, forced_vr)}
    videos = await _remote_videos(client)
    assert {p for p, v in videos.items() if v["isVR"]} == {named, shaped, forced_vr}


@pytest.mark.anyio
async def test_vr_format_projection_sets_the_headset_layout(client, probe, tmp_path):
    tb = _make(tmp_path, probe, "Plain.mp4", FLAT_DIMS)
    flat = _make(tmp_path, probe, "Scene_180_sbs.mp4", None)
    await _register(client, {
        "videos": [_video(tb), _video(flat)],
        "vrFormat": {tb: {"projection": "tb-half"}, flat: {"projection": "flat"}},
    })
    scene = (await client.get(f"/heresphere/{media._path_to_id(tb)}")).json()
    assert (scene["projection"], scene["stereo"]) == ("equirectangular", "tb")
    assert {"name": "Format:VR"} in scene["tags"]

    scene = (await client.get(f"/heresphere/{media._path_to_id(flat)}")).json()
    assert (scene["projection"], scene["stereo"]) == ("perspective", "mono")
    assert {"name": "Format:Flat"} in scene["tags"]
    assert {"name": "Format:VR"} not in scene["tags"]


@pytest.mark.anyio
async def test_register_stores_preference_scores(client, probe, tmp_path):
    a = _make(tmp_path, probe, "A.mp4", FLAT_DIMS)
    b = _make(tmp_path, probe, "B.mp4", FLAT_DIMS)
    await _register(client, {
        "videos": [_video(a), _video(b)],
        "preferenceScores": {a: 7.5, b: -3, "/bad.mp4": "9", "/flag.mp4": True, "/nan.mp4": None},
    })
    assert (media.get_preference_score(a), media.get_preference_score(b)) == (7.5, -3.0)
    assert media.get_preference_score("/bad.mp4") is None
    assert media.get_preference_score("/flag.mp4") is None
    assert media.get_preference_score("/nan.mp4") is None

    # A registration without the key leaves the scores alone; a new map replaces them.
    await _register(client, {"scriptTags": {a: ["riding"]}})
    assert media.get_preference_score(a) == 7.5
    await _register(client, {"preferenceScores": {b: 2}})
    assert (media.get_preference_score(a), media.get_preference_score(b)) == (None, 2.0)


# --- web remote -------------------------------------------------------------------

@pytest.mark.anyio
async def test_remote_videos_carry_is_vr_tags_and_categories(client, probe, tmp_path):
    a = _make(tmp_path, probe, "Alpha_180_LR.mp4", None)
    b = _make(tmp_path, probe, "Beta.mp4", FLAT_DIMS)
    await _register(client, {
        "videos": [_video(a), _video(b)],
        "scriptTags": {a: ["Riding", "creampie", "vr", "len-5-10", "the-handy"], b: ["non-vr"]},
        "categories": [{"id": "c1", "name": "Creampie"}, {"id": "c2", "name": "VR"}, {"id": "c3", "name": " "}],
        "videoCategories": {a: ["c2", "c1", "c3"], b: []},
    })
    videos = await _remote_videos(client)
    assert videos[a]["isVR"] is True
    assert videos[a]["tags"] == ["creampie", "riding"]
    assert videos[a]["categories"] == ["Creampie", "VR"]  # category order, blank names skipped
    assert (videos[b]["isVR"], videos[b]["tags"], videos[b]["categories"]) == (False, [], [])


@pytest.mark.anyio
async def test_remote_videos_carry_the_preference_score(client, probe, tmp_path):
    a = _make(tmp_path, probe, "Alpha.mp4", FLAT_DIMS)
    b = _make(tmp_path, probe, "Beta.mp4", FLAT_DIMS)
    await _register(client, {"videos": [_video(a), _video(b)], "preferenceScores": {a: 9.25}})
    videos = await _remote_videos(client)
    assert videos[a]["score"] == 9.25
    assert videos[b]["score"] is None  # not scored yet


# --- headsets ---------------------------------------------------------------------

@pytest.mark.anyio
async def test_heresphere_and_deovr_scene_tags(client, probe, tmp_path):
    a = _make(tmp_path, probe, "Alpha_180_LR.mp4", None)
    await _register(client, {
        "videos": [_video(a, hasFunscript=True, sourceName="Multi-Axis")],
        "scriptTags": {a: ["riding", "creampie", "osr2"]},
        "categories": [{"id": "c1", "name": "Creampie"}],
        "videoCategories": {a: ["c1"]},
        "playlists": [{"name": "Keep Forever", "videoPaths": [a]}],
    })
    vid = media._path_to_id(a)
    names = [t["name"] for t in (await client.get(f"/heresphere/{vid}")).json()["tags"]]
    expected = [
        "Studio:Multi-Axis", "Feature:Is scripted", "Format:VR", "Length:Medium (5-15m)",
        "Playlist:Keep Forever", "Category:Creampie", "Tag:creampie", "Tag:riding",
    ]
    assert names[:len(expected)] == expected

    deovr = (await client.get(f"/deovr/{vid}")).json()
    assert [c["tag"]["name"] for c in deovr["categories"]] == expected
    assert [c["tag"]["id"] for c in deovr["categories"]] == list(range(1, len(expected) + 1))


class TestScoreTag:
    @pytest.mark.parametrize("score, tag", [
        (7, "Score:7.0"), (7.5, "Score:7.5"), (7.2, "Score:7.0"), (7.25, "Score:7.5"),
        (7.74, "Score:7.5"), (7.75, "Score:8.0"), (0, "Score:0.0"), (-0.1, "Score:0.0"), (-3, "Score:-3.0"),
        (-3.25, "Score:-3.0"), (13.15, "Score:13.0"),
    ])
    def test_nearest_half_one_decimal(self, score, tag):
        assert score_tags(score) == [tag]

    def test_no_score_no_tag(self):
        assert score_tags(None) == []


@pytest.mark.anyio
async def test_heresphere_and_deovr_score_tag(client, probe, tmp_path):
    a = _make(tmp_path, probe, "Alpha_180_LR.mp4", None)
    b = _make(tmp_path, probe, "Beta_180_LR.mp4", None)
    await _register(client, {
        "videos": [_video(a, hasFunscript=True, sourceName="EroScripts"), _video(b)],
        "scriptTags": {a: ["riding"]},
        "preferenceScores": {a: 10.25},
    })
    vid = media._path_to_id(a)
    names = [t["name"] for t in (await client.get(f"/heresphere/{vid}")).json()["tags"]]
    # After the chip tags; chapters follow.
    assert names[names.index("Tag:riding") + 1] == "Score:10.5"
    deovr = [c["tag"]["name"] for c in (await client.get(f"/deovr/{vid}")).json()["categories"]]
    assert deovr[-2:] == ["Tag:riding", "Score:10.5"]

    unscored = [t["name"] for t in (await client.get(f"/heresphere/{media._path_to_id(b)}")).json()["tags"]]
    assert not [n for n in unscored if n.startswith("Score:")]

    # Read-only: a save without it changes nothing, and it comes back.
    r = await client.post(f"/heresphere/{vid}", json={"tags": [{"name": "Format:VR"}]})
    assert {"name": "Score:10.5"} in r.json()["tags"]


@pytest.mark.anyio
async def test_heresphere_tag_edits_only_touch_playlists(client, probe, tmp_path):
    """`Tag:` and `Category:` are read-only: a HereSphere save that drops
    them changes nothing, and they come back on the next load."""
    a = _make(tmp_path, probe, "Alpha_180_LR.mp4", None)
    await _register(client, {"videos": [_video(a)], "scriptTags": {a: ["riding"]}})
    vid = media._path_to_id(a)
    r = await client.post(f"/heresphere/{vid}", json={"tags": [{"name": "Format:VR"}]})
    assert r.status_code == 200
    assert {"name": "Tag:riding"} in r.json()["tags"]
