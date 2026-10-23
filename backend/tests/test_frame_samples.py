"""Frame samples for EroScripts preview matching (sample_gray_frames and
the /thumbnails/samples route)."""

import base64
import os
import shutil
import subprocess
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import services.ffmpeg as ffmpeg_service  # noqa: E402
from services.ffmpeg import _thin, sample_gray_frames, sample_positions  # noqa: E402

HAVE_FFMPEG = os.path.exists(ffmpeg_service.FFMPEG) or shutil.which(ffmpeg_service.FFMPEG)


@pytest.fixture(autouse=True)
def _isolated_metadata_cache():
    ffmpeg_service._reset_metadata_cache_for_tests()
    yield
    ffmpeg_service._reset_metadata_cache_for_tests()


def test_positions_skip_the_first_and_last_five_percent():
    pos = sample_positions(12)
    assert len(pos) == 12
    assert min(pos) > 0.05 and max(pos) < 0.95
    assert pos == sorted(pos)


def test_thin_spreads_evenly():
    assert _thin(list(range(10)), 20) == list(range(10))
    assert _thin(list(range(100)), 4) == [0, 25, 50, 75]


def _make_video(path, gop):
    subprocess.run(
        [ffmpeg_service.FFMPEG, "-v", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=10",
         "-t", "20", "-g", str(gop), "-pix_fmt", "yuv420p", "-y", str(path)],
        check=True, capture_output=True,
    )


@pytest.mark.skipif(not HAVE_FFMPEG, reason="ffmpeg not available")
def test_keyframe_pass_returns_gray_frames_inside_the_middle(tmp_path):
    video = tmp_path / "v.mp4"
    _make_video(video, gop=10)  # a keyframe a second: ~18 in the middle 90%
    out = sample_gray_frames(str(video), min_count=12, width=64)
    assert out["width"] == 64 and out["height"] == 36
    assert len(out["frames"]) >= 12
    for f in out["frames"]:
        assert len(f["gray"]) == 64 * 36
        assert 0.04 <= f["pct"] <= 0.96
    assert [f["pct"] for f in out["frames"]] == sorted(f["pct"] for f in out["frames"])


@pytest.mark.skipif(not HAVE_FFMPEG, reason="ffmpeg not available")
def test_too_few_keyframes_fall_back_to_seeks(tmp_path):
    video = tmp_path / "v.mp4"
    _make_video(video, gop=60)  # a keyframe every 6 s: 3 in the middle 90%
    out = sample_gray_frames(str(video), min_count=12, width=64)
    # A keyframe-only seek yields the next keyframe, so positions after the
    # last one come back empty and are left out.
    expected = {round(p, 4) for p in sample_positions(12)}
    got = [round(f["pct"], 4) for f in out["frames"]]
    assert len(got) > 3 and set(got) <= expected


def test_big_files_skip_the_full_pass(tmp_path, monkeypatch):
    video = tmp_path / "big.mp4"
    video.write_bytes(b"x" * 100)
    monkeypatch.setattr(ffmpeg_service, "SAMPLE_PASS_MAX_BYTES", 10)
    monkeypatch.setattr(ffmpeg_service, "get_metadata", lambda p: {"duration": 100, "width": 3840, "height": 1920})
    monkeypatch.setattr(ffmpeg_service, "_keyframe_pass", lambda *a: pytest.fail("full pass on a big file"))
    seen = {}

    def fake_seeks(path, duration, width, height, count):
        seen["count"] = count
        return [{"pct": 0.5, "gray": b"\0" * (width * height)}]

    monkeypatch.setattr(ffmpeg_service, "_seek_frames", fake_seeks)
    out = sample_gray_frames(str(video), min_count=12, width=128)
    assert seen["count"] == ffmpeg_service.SAMPLE_BIG_FILE_SEEKS
    assert out["height"] == 64


def test_missing_video_raises(tmp_path):
    with pytest.raises(FileNotFoundError):
        sample_gray_frames(str(tmp_path / "nope.mp4"))


@pytest.mark.skipif(not HAVE_FFMPEG, reason="ffmpeg not available")
def test_samples_route_base64_encodes_frames(tmp_path):
    from fastapi.testclient import TestClient
    from main import app

    video = tmp_path / "v.mp4"
    _make_video(video, gop=10)
    resp = TestClient(app).post("/thumbnails/samples", params={"video_path": str(video), "width": 32})
    assert resp.status_code == 200
    body = resp.json()
    assert body["width"] == 32
    assert len(base64.b64decode(body["frames"][0]["gray"])) == 32 * body["height"]
    missing = TestClient(app).post("/thumbnails/samples", params={"video_path": str(tmp_path / "x.mp4")})
    assert missing.status_code == 404
