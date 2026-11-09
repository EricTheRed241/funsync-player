"""Thumbnails pick a real picture, follow the file, and reach every client.

Rules these tests defend (ThumbScout findings, 2026-11-09):
  1. A dark, blown-out, flat or title-card auto frame is retried deeper into
     the video (25 / 40 / 60%); when every position is bad the best-scoring
     frame is kept. A pinned (exact) frame is never second-guessed.
  2. One persistent cache keyed by path + size + mtime: a changed file gets
     a new thumbnail, and the registration's prune drops files no
     registered video uses.
  3. VR tiles show one eye (left half / top half), wide-angle projections
     also the middle of it.
  4. Headset and phone URLs carry a `?v=` version that changes with the
     served image.

Synthetic videos are made with ffmpeg in a temp dir.
"""

import os
import shutil
import subprocess
import sys
import time

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from httpx import ASGITransport, AsyncClient  # noqa: E402

from main import app  # noqa: E402
from routes import media, vr_api  # noqa: E402
from services import ffmpeg as ffmpeg_service  # noqa: E402
from services import stereo_detect  # noqa: E402

HAVE_FFMPEG = shutil.which(ffmpeg_service.FFMPEG) is not None or os.path.isfile(ffmpeg_service.FFMPEG)
needs_ffmpeg = pytest.mark.skipif(not HAVE_FFMPEG, reason="ffmpeg not available")

# A plain grey card with a small white caption: one grey level fills ~97%.
CARD = "color=gray:s=160x90:r=5:d={d},drawbox=x=60:y=40:w=40:h=8:color=white:t=fill"
CONTENT = "testsrc2=s={s}:r=5:d={d}"


@pytest.fixture
def cache(tmp_path, monkeypatch):
    root = tmp_path / "thumb-cache"
    monkeypatch.setattr(ffmpeg_service, "_thumb_cache_dir", str(root))
    return root


def _concat(out, *sources):
    """Encode lavfi `sources` back to back (keyframe every second)."""
    args = [ffmpeg_service.FFMPEG, "-v", "error", "-y"]
    for src in sources:
        args += ["-f", "lavfi", "-i", src]
    n = len(sources)
    graph = "".join(f"[{i}:v]" for i in range(n)) + f"concat=n={n}:v=1[v]"
    args += ["-filter_complex", graph, "-map", "[v]", "-g", "5", "-pix_fmt", "yuv420p", str(out)]
    subprocess.run(args, check=True, capture_output=True)
    return str(out)


def _stats(jpeg_path):
    gray = subprocess.run(
        [ffmpeg_service.FFMPEG, "-v", "error", "-i", jpeg_path,
         "-vf", "scale=32:32:flags=area,format=gray", "-f", "rawvideo", "pipe:1"],
        capture_output=True, check=True).stdout
    return ffmpeg_service.frame_stats(gray)


def _dims(jpeg_path):
    out = subprocess.run(
        [ffmpeg_service.FFPROBE, "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height", "-of", "csv=p=0", jpeg_path],
        capture_output=True, text=True, check=True).stdout.strip()
    w, h = out.split(",")
    return int(w), int(h)


# --- 1. Frame rejection ---------------------------------------------------

class TestFrameProblem:
    def test_black_white_flat_and_title_card_are_rejected(self):
        assert ffmpeg_service.frame_problem(ffmpeg_service.frame_stats(bytes(1024))) == "dark"
        assert ffmpeg_service.frame_problem(ffmpeg_service.frame_stats(bytes([250]) * 1024)) == "bright"
        flat = bytes([100 + (i % 5) for i in range(1024)])
        assert ffmpeg_service.frame_problem(ffmpeg_service.frame_stats(flat)) == "flat"
        card = bytes([128] * 960 + [255] * 64)   # grey card, white caption
        assert ffmpeg_service.frame_problem(ffmpeg_service.frame_stats(card)) == "title-card"

    def test_a_picture_passes(self):
        picture = bytes((i * 37) % 256 for i in range(1024))
        assert ffmpeg_service.frame_problem(ffmpeg_service.frame_stats(picture)) is None


@needs_ffmpeg
class TestRetries:
    def test_black_intro_is_skipped(self, tmp_path, cache):
        # Black for 30 s: the 10 s and 25 % (25 s) frames are dark, 40 % isn't.
        video = _concat(tmp_path / "intro.mp4", "color=black:s=160x90:r=5:d=30", CONTENT.format(s="160x90", d=70))
        out = ffmpeg_service.generate_single_thumbnail(video)
        assert ffmpeg_service.frame_problem(_stats(out["path"])) is None
        assert out["path"].startswith(str(cache))

    def test_title_card_is_skipped(self, tmp_path, cache):
        video = _concat(tmp_path / "card.mp4", CARD.format(d=50), CONTENT.format(s="160x90", d=50))
        out = ffmpeg_service.generate_single_thumbnail(video)
        assert ffmpeg_service.frame_problem(_stats(out["path"])) is None

    def test_all_bad_keeps_the_best_frame(self, tmp_path, cache):
        video = _concat(tmp_path / "black.mp4", "color=black:s=160x90:r=5:d=80")
        out = ffmpeg_service.generate_single_thumbnail(video)
        assert os.path.getsize(out["path"]) > 0
        assert ffmpeg_service.frame_problem(_stats(out["path"])) == "dark"

    def test_pinned_frame_is_not_second_guessed(self, tmp_path, cache):
        video = _concat(tmp_path / "intro.mp4", "color=black:s=160x90:r=5:d=30", CONTENT.format(s="160x90", d=70))
        out = ffmpeg_service.generate_single_thumbnail(video, seek_pct=0.05, exact=True)
        assert ffmpeg_service.frame_problem(_stats(out["path"])) == "dark"

    def test_cache_directory_appears_only_with_a_thumbnail(self, tmp_path, cache):
        broken = tmp_path / "broken.mp4"
        broken.write_bytes(b"not a video at all")
        with pytest.raises(RuntimeError):
            ffmpeg_service.generate_single_thumbnail(str(broken))
        assert not cache.exists()


# --- 2. Cache key and prune -----------------------------------------------

class TestCacheKey:
    def test_a_changed_file_gets_a_new_thumbnail_name(self, tmp_path, cache):
        video = tmp_path / "v.mp4"
        video.write_bytes(b"x" * 10)
        os.utime(video, ns=(1_000_000_000, 1_000_000_000))
        before = ffmpeg_service.single_thumbnail_path(str(video))
        os.utime(video, ns=(2_000_000_000, 2_000_000_000))
        assert ffmpeg_service.single_thumbnail_path(str(video)) != before
        video.write_bytes(b"x" * 11)
        os.utime(video, ns=(1_000_000_000, 1_000_000_000))
        assert ffmpeg_service.single_thumbnail_path(str(video)) != before

    def test_desktop_and_headset_share_the_file(self, tmp_path, cache):
        video = tmp_path / "v.mp4"
        video.write_bytes(b"x" * 10)
        media.register_videos([{"path": str(video), "name": "v.mp4"}])
        vid = media._path_to_id(str(video))
        _, served, _, _ = media._thumb_target(vid, str(video), media.get_video_registry()[vid])
        assert served == ffmpeg_service.single_thumbnail_path(str(video), 0.1, 320)
        assert os.path.basename(served).startswith(vid + "_")


class TestPrune:
    def _touch(self, root, name):
        root.mkdir(exist_ok=True)
        (root / name).write_bytes(b"jpg")
        return name

    def test_keeps_only_the_files_registered_videos_use(self, cache):
        m = ffmpeg_service.THUMB_MARKER
        keep = self._touch(cache, f"aaaaaaaaaaaa_11111111_single_{m}e_0500_320_sbsc.jpg")
        old_pin = self._touch(cache, f"aaaaaaaaaaaa_11111111_single_{m}e_0300_320_sbsc.jpg")
        flat = self._touch(cache, f"aaaaaaaaaaaa_11111111_single_{m}_0100_320.jpg")
        offline = self._touch(cache, f"cccccccccccc_33333333_single_{m}_0100_320.jpg")
        poster = self._touch(cache, f"dddddddddddd_44444444_single_{m}_0100_320.jpg")
        gone = self._touch(cache, f"bbbbbbbbbbbb_22222222_single_{m}_0100_320.jpg")
        stale = self._touch(cache, f"aaaaaaaaaaaa_99999999_single_{m}e_0500_320_sbsc.jpg")
        old_marker = self._touch(cache, "aaaaaaaaaaaa_11111111_single_v4e_0500_320.jpg")
        legacy = self._touch(cache, "aaaaaaaaaaaa_p0500.jpg")
        other = self._touch(cache, "metadata-cache-v1.json")
        in_flight = self._touch(cache, f"{keep}.123.tmp")

        removed = ffmpeg_service.prune_thumb_cache({
            "aaaaaaaaaaaa": {keep},      # pinned VR frame
            "cccccccccccc": None,        # can't be statted right now
            "dddddddddddd": set(),       # an uploaded poster replaces it
        })

        assert set(os.listdir(cache)) == {keep, offline, other, in_flight}
        assert removed == len({old_pin, flat, poster, gone, stale, old_marker, legacy})

    def test_registration_prunes_only_when_asked(self, tmp_path, cache, monkeypatch):
        monkeypatch.setattr(media, "_queue_duration_probes", lambda: None)
        monkeypatch.setattr(media, "_queue_speed_probes", lambda: None)
        monkeypatch.setattr(media, "_custom_thumbs", {})
        monkeypatch.setattr(ffmpeg_service, "generate_single_thumbnail", lambda *a, **k: (_ for _ in ()).throw(RuntimeError("no")))
        video = tmp_path / "kept.mp4"
        video.write_bytes(b"x" * 10)
        auto = os.path.basename(ffmpeg_service.single_thumbnail_path(str(video)))
        pinned = os.path.basename(ffmpeg_service.single_thumbnail_path(str(video), 0.5, exact=True))
        orphan = f"0123456789ab_11111111_single_{ffmpeg_service.THUMB_MARKER}_0100_320.jpg"
        for name in (auto, pinned, orphan):
            self._touch(cache, name)

        payload = {"videos": [{"path": str(video), "name": "kept.mp4"}]}
        media._apply_registration(payload)
        assert sorted(os.listdir(cache)) == sorted([auto, pinned, orphan])

        media._apply_registration({**payload, "thumbPrune": True})
        assert sorted(os.listdir(cache)) == [auto]

        # Pinning a frame makes the auto frame unused.
        self._touch(cache, pinned)
        media._apply_registration({**payload, "thumbPrune": True,
                                   "customThumbnails": {str(video): {"seekPct": 0.5}}})
        assert sorted(os.listdir(cache)) == [pinned]
        _wait_jobs()


def _wait_jobs(timeout=5.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        with media._thumb_jobs_lock:
            if not media._thumb_jobs and not media._warmup_running:
                return
        time.sleep(0.01)


# --- 3. VR crop -----------------------------------------------------------

class TestVrCropChoice:
    @pytest.fixture(autouse=True)
    def _clean(self, monkeypatch):
        stereo_detect._reset_for_tests()
        monkeypatch.setattr(media, "_vr_format", {})
        yield
        stereo_detect._reset_for_tests()

    def test_crop_follows_the_known_layout(self, monkeypatch):
        def crop(name, projection=None):
            path = f"/lib/{name}"
            if projection:
                monkeypatch.setattr(media, "_vr_format", {path: {"projection": projection}})
            return vr_api.thumb_vr_crop(media._path_to_id(path), {"path": path, "name": name})

        assert crop("Scene_180_LR.mp4") == "sbsc"         # 180° SBS: left eye, centre
        assert crop("Plain.mp4", "sbs-full") == "sbs"     # flat 3D: left eye only
        assert crop("Plain.mp4", "tb-half") == "tbc"      # top-bottom dome: top eye
        assert crop("Holiday.mp4") is None                # flat
        assert crop("Scene_180_LR.mp4", "flat") is None   # the desktop said flat


@needs_ffmpeg
class TestVrCropFrames:
    def _sbs(self, tmp_path):
        # Left eye: picture; right eye: black. 320×160 = two 160×160 eyes.
        out = tmp_path / "sbs.mp4"
        subprocess.run(
            [ffmpeg_service.FFMPEG, "-v", "error", "-y",
             "-f", "lavfi", "-i", CONTENT.format(s="160x160", d=20),
             "-f", "lavfi", "-i", "color=black:s=160x160:r=5:d=20",
             "-filter_complex", "[0:v][1:v]hstack=inputs=2[v]", "-map", "[v]",
             "-g", "5", "-pix_fmt", "yuv420p", str(out)],
            check=True, capture_output=True)
        return str(out)

    def test_side_by_side_shows_the_left_eye(self, tmp_path, cache):
        video = self._sbs(tmp_path)
        out = ffmpeg_service.generate_single_thumbnail(video, vr_crop="sbs")
        assert _dims(out["path"]) == (320, 320)
        assert (out["width"], out["height"]) == (320, 320)
        assert ffmpeg_service.frame_problem(_stats(out["path"])) is None   # no black eye in it

    def test_wide_angle_also_takes_the_middle(self, tmp_path, cache):
        video = self._sbs(tmp_path)
        out = ffmpeg_service.generate_single_thumbnail(video, vr_crop="sbsc")
        w, h = _dims(out["path"])
        assert w == 320 and abs(w / h - 1.4) < 0.05   # 70% × 50% of a square eye
        assert out["path"].endswith("_sbsc.jpg")

    def test_top_bottom_shows_the_top_eye(self, tmp_path, cache):
        out_path = tmp_path / "tb.mp4"
        subprocess.run(
            [ffmpeg_service.FFMPEG, "-v", "error", "-y",
             "-f", "lavfi", "-i", CONTENT.format(s="160x160", d=20),
             "-f", "lavfi", "-i", "color=black:s=160x160:r=5:d=20",
             "-filter_complex", "[0:v][1:v]vstack=inputs=2[v]", "-map", "[v]",
             "-g", "5", "-pix_fmt", "yuv420p", str(out_path)],
            check=True, capture_output=True)
        out = ffmpeg_service.generate_single_thumbnail(str(out_path), vr_crop="tb")
        assert _dims(out["path"]) == (320, 320)
        assert ffmpeg_service.frame_problem(_stats(out["path"])) is None

    def test_pinned_frames_are_cropped_too(self, tmp_path, cache):
        video = self._sbs(tmp_path)
        out = ffmpeg_service.generate_single_thumbnail(video, seek_pct=0.5, exact=True, vr_crop="sbs")
        assert _dims(out["path"]) == (320, 320)


# --- 4. Versioned URLs ----------------------------------------------------

@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture
async def client():
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as c:
        yield c


class TestVersionedUrls:
    @pytest.fixture(autouse=True)
    def _isolate(self, tmp_path, cache, monkeypatch):
        monkeypatch.setattr(media, "_queue_duration_probes", lambda: None)
        monkeypatch.setattr(media, "_queue_speed_probes", lambda: None)
        monkeypatch.setattr(media, "_queue_thumb_warmup", lambda prune=False: 0)
        monkeypatch.setattr(media, "_custom_thumbs", {})
        monkeypatch.setattr(media, "_custom_thumbs_dir", str(tmp_path / "custom-thumbs"))
        (tmp_path / "custom-thumbs").mkdir()
        self.dir = tmp_path

    def _video(self, **extra):
        return {"path": "/lib/Alpha.mp4", "name": "Alpha.mp4", "size": 100, "dateAdded": 1000, **extra}

    def test_version_changes_with_what_is_served(self, monkeypatch):
        v = self._video()
        vid = media._path_to_id(v["path"])
        base = media.thumb_url(vid, v)
        assert base.startswith(f"/api/media/thumb/{vid}?v=")
        assert media.thumb_url(vid, v) == base                                   # stable

        assert media.thumb_url(vid, self._video(dateAdded=2000)) != base         # replaced file
        monkeypatch.setattr(media, "_custom_thumbs", {v["path"]: {"seekPct": 0.4}})
        pinned = media.thumb_url(vid, v)
        assert pinned != base                                                    # pinned frame
        monkeypatch.setattr(media, "_custom_thumbs", {v["path"]: {"seekPct": 0.6}})
        assert media.thumb_url(vid, v) != pinned                                 # re-pinned

        poster = self.dir / "custom-thumbs" / "p.jpg"
        poster.write_bytes(b"jpg")
        os.utime(poster, ns=(1_000_000_000, 1_000_000_000))
        monkeypatch.setattr(media, "_custom_thumbs", {v["path"]: {"type": "image", "imagePath": str(poster)}})
        first_poster = media.thumb_url(vid, v)
        os.utime(poster, ns=(2_000_000_000, 2_000_000_000))
        assert media.thumb_url(vid, v) != first_poster                           # new poster

    @pytest.mark.anyio
    async def test_listings_carry_the_version(self, client):
        media._apply_registration({"videos": [self._video()]})
        vid = media._path_to_id("/lib/Alpha.mp4")
        expected = media.thumb_url(vid, media.get_video_registry()[vid])

        phone = (await client.get("/api/remote/videos")).json()["videos"]
        assert [v["thumbUrl"] for v in phone] == [expected]

        scene = (await client.post(f"/heresphere/{vid}", json={})).json()
        assert scene["thumbnailImage"].endswith(expected)
        deovr = (await client.get(f"/deovr/{vid}")).json()
        assert deovr["thumbnailUrl"].endswith(expected)
