"""Headset library sections, filter tags, and hand-made multi-axis detection.

The HereSphere / DeoVR listings split the library into plain-text sections
(VR / Flat, speed, length, hand-made multi-axis, recently added) and the
scene details carry matching filter tags. Rules live in
`services.vr_sections`; hand-made axes are decided at registration.
"""

import os
import sys
import time

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))

from httpx import AsyncClient, ASGITransport
from main import app
from routes.media import register_videos, get_video_registry, _path_to_id
from services import vr_sections
from services.funscript import has_handmade_axes

DAY_MS = 24 * 60 * 60 * 1000
ALL_SECTIONS = [
    "VR Videos", "Flat Videos", "Slow", "Medium Speed", "Fast", "Short",
    "Medium Length", "Long", "Hand-made Multi-Axis", "Recently Added",
]


@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture
async def client():
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as c:
        yield c


# --- Bucket rules ---------------------------------------------------------

class TestSpeedBucket:
    @pytest.mark.parametrize("avg, expected", [
        (1, "Slow"), (249, "Slow"), (249.9, "Slow"),
        (250, "Medium"), (349, "Medium"), (349.9, "Medium"),
        (350, "Fast"), (900, "Fast"),
    ])
    def test_boundaries(self, avg, expected):
        assert vr_sections.speed_bucket({"avgSpeed": avg}) == expected

    @pytest.mark.parametrize("video", [{}, {"avgSpeed": None}, {"avgSpeed": 0}, None])
    def test_no_stats_is_no_bucket(self, video):
        assert vr_sections.speed_bucket(video) is None


class TestLengthBucket:
    @pytest.mark.parametrize("duration, expected", [
        (1, "Short"), (299, "Short"), (299.9, "Short"),
        (300, "Medium"), (600, "Medium"), (900, "Medium"),
        (900.5, "Long"), (901, "Long"), (7200, "Long"),
    ])
    def test_boundaries(self, duration, expected):
        assert vr_sections.length_bucket({"duration": duration}) == expected

    @pytest.mark.parametrize("video", [{}, {"duration": None}, {"duration": 0}, None])
    def test_unknown_duration_is_no_bucket(self, video):
        assert vr_sections.length_bucket(video) is None


class TestRecent:
    def test_within_window(self):
        now = 1_800_000_000_000
        assert vr_sections.is_recent({"dateAdded": now - 13 * DAY_MS}, now)
        assert vr_sections.is_recent({"dateAdded": now - 14 * DAY_MS}, now)

    def test_outside_window_or_missing(self):
        now = 1_800_000_000_000
        assert not vr_sections.is_recent({"dateAdded": now - 15 * DAY_MS}, now)
        assert not vr_sections.is_recent({}, now)


class TestBuildSections:
    def test_order_and_empty_sections_omitted(self):
        now = 1_800_000_000_000
        entries = [
            ("a", {"avgSpeed": 400, "duration": 100}, True),
            ("b", {"avgSpeed": 100, "duration": 2000, "dateAdded": now}, False),
            ("c", {}, False),
        ]
        sections = vr_sections.build_sections(entries, now_ms=now)
        assert sections == [
            ("VR Videos", ["a"]),
            ("Flat Videos", ["b", "c"]),
            ("Slow", ["b"]),
            ("Fast", ["a"]),
            ("Short", ["a"]),
            ("Long", ["b"]),
            ("Recently Added", ["b"]),
        ]

    def test_section_order_is_the_approved_list(self):
        assert list(vr_sections.SECTION_ORDER) == ALL_SECTIONS

    def test_names_are_plain_text(self):
        for name in vr_sections.SECTION_ORDER:
            assert name.isascii(), name


# --- Hand-made axis detection ---------------------------------------------

class TestHandmadeAxes:
    def test_handmade_companion(self):
        assert has_handmade_axes("/v/Title.funscript", ["Title.funscript", "Title.twist.funscript"])

    def test_generated_companions_only(self):
        assert not has_handmade_axes("/v/Title.funscript", [
            "Title.funscript",
            "Title.generated.twist.funscript",
            "Title.generated.roll.funscript",
        ])

    def test_mixed_generated_and_handmade(self):
        assert has_handmade_axes("/v/Title.funscript", [
            "Title.generated.pitch.funscript", "Title.roll.funscript",
        ])

    def test_case_insensitive_and_aliases(self):
        assert has_handmade_axes("/v/Title.funscript", ["title.SURGE.Funscript"])
        assert has_handmade_axes("/v/Title.funscript", ["Title.suction.funscript"])
        assert not has_handmade_axes("/v/Title.funscript", ["Title.GENERATED.Twist.funscript"])

    def test_companion_of_another_script_ignored(self):
        # A variant's companion is not the main script's.
        assert not has_handmade_axes("/v/Title.funscript", ["Title (hard).twist.funscript"])
        assert not has_handmade_axes("/v/Title (soft).funscript", ["Title.twist.funscript"])

    def test_appledouble_ignored(self):
        assert not has_handmade_axes("/v/Title.funscript", ["._Title.twist.funscript"])

    def test_unknown_suffix_and_non_funscript_ignored(self):
        assert not has_handmade_axes("/v/Title.funscript", [
            "Title.intense.funscript", "Title.twist.json", "Title.twist.funscript.bak",
        ])

    def test_dotted_main_name(self):
        assert has_handmade_axes("/v/Mr.Smith.funscript", ["Mr.Smith.vib.funscript"])


def _touch(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("{}")


def test_registration_marks_handmade_axes(tmp_path):
    hand = tmp_path / "Hand"
    gen = tmp_path / "Gen"
    for f in ["Hand.mp4", "Hand.funscript", "Hand.twist.funscript", "._Hand.roll.funscript"]:
        _touch(hand / f)
    for f in ["Gen.mp4", "Gen.funscript", "Gen.generated.twist.funscript", "._Gen.roll.funscript"]:
        _touch(gen / f)
    register_videos([
        {"path": str(hand / "Hand.mp4"), "name": "Hand.mp4", "funscriptPath": str(hand / "Hand.funscript")},
        {"path": str(gen / "Gen.mp4"), "name": "Gen.mp4", "funscriptPath": str(gen / "Gen.funscript")},
        {"path": str(tmp_path / "None.mp4"), "name": "None.mp4"},
        {"path": "/missing/X.mp4", "name": "X.mp4", "funscriptPath": "/missing/X.funscript"},
    ])
    reg = get_video_registry()
    assert reg[_path_to_id(str(hand / "Hand.mp4"))]["handmadeAxes"] is True
    assert reg[_path_to_id(str(gen / "Gen.mp4"))]["handmadeAxes"] is False
    assert reg[_path_to_id(str(tmp_path / "None.mp4"))]["handmadeAxes"] is False
    assert reg[_path_to_id("/missing/X.mp4")]["handmadeAxes"] is False


def test_registration_lists_each_folder_once(tmp_path, monkeypatch):
    folder = tmp_path / "Shared"
    for f in ["A.funscript", "B.funscript", "B.pitch.funscript"]:
        _touch(folder / f)
    calls = []
    real_listdir = os.listdir

    def counting_listdir(path):
        calls.append(path)
        return real_listdir(path)

    monkeypatch.setattr(os, "listdir", counting_listdir)
    register_videos([
        {"path": str(folder / "A.mp4"), "name": "A.mp4", "funscriptPath": str(folder / "A.funscript")},
        {"path": str(folder / "B.mp4"), "name": "B.mp4", "funscriptPath": str(folder / "B.funscript")},
    ])
    assert calls == [str(folder)]
    reg = get_video_registry()
    assert reg[_path_to_id(str(folder / "A.mp4"))]["handmadeAxes"] is False
    assert reg[_path_to_id(str(folder / "B.mp4"))]["handmadeAxes"] is True


# --- Endpoints ------------------------------------------------------------

def _library(tmp_path):
    """One video per section, real files so hand-made detection runs."""
    now = time.time() * 1000
    multi = tmp_path / "Multi"
    for f in ["Multi.funscript", "Multi.surge.funscript"]:
        _touch(multi / f)
    return [
        # VR, Slow, Short, recently added
        {"path": "/Lib/A_180_sbs.mp4", "name": "A_180_sbs.mp4", "sourceName": "Lib",
         "funscriptPath": "/Lib/A.funscript", "hasFunscript": True,
         "avgSpeed": 249, "duration": 299, "dateAdded": now - DAY_MS},
        # Flat, Medium Speed (lower edge), Medium Length (lower edge), old
        {"path": "/Lib/B.mp4", "name": "B.mp4", "sourceName": "Lib",
         "funscriptPath": "/Lib/B.funscript", "hasFunscript": True,
         "avgSpeed": 250, "duration": 300, "dateAdded": now - 30 * DAY_MS},
        # Flat, Medium Speed (upper edge), Medium Length (upper edge)
        {"path": "/Lib/C.mp4", "name": "C.mp4",
         "funscriptPath": "/Lib/C.funscript", "hasFunscript": True,
         "avgSpeed": 349, "duration": 900},
        # Flat, Fast, Long, hand-made multi-axis
        {"path": str(multi / "Multi.mp4"), "name": "Multi.mp4",
         "funscriptPath": str(multi / "Multi.funscript"), "hasFunscript": True,
         "avgSpeed": 350, "duration": 901},
        # Flat, no script / no duration: only in Flat Videos
        {"path": "/Lib/D.mp4", "name": "D.mp4"},
    ]


def _ids(urls):
    return [u.rstrip("/").rsplit("/", 1)[1] for u in urls]


@pytest.mark.anyio
async def test_heresphere_sections(client, tmp_path):
    register_videos(_library(tmp_path))
    data = (await client.get("/heresphere")).json()
    sections = {g["name"]: _ids(g["list"]) for g in data["library"]}
    assert [g["name"] for g in data["library"]] == ALL_SECTIONS

    a, b, c, d = (_path_to_id(p) for p in ["/Lib/A_180_sbs.mp4", "/Lib/B.mp4", "/Lib/C.mp4", "/Lib/D.mp4"])
    m = _path_to_id(str(tmp_path / "Multi" / "Multi.mp4"))
    assert sections["VR Videos"] == [a]
    assert sections["Flat Videos"] == [b, c, d, m]   # sorted by title
    assert sections["Slow"] == [a]
    assert sections["Medium Speed"] == [b, c]
    assert sections["Fast"] == [m]
    assert sections["Short"] == [a]
    assert sections["Medium Length"] == [b, c]
    assert sections["Long"] == [m]
    assert sections["Hand-made Multi-Axis"] == [m]
    assert sections["Recently Added"] == [a]


@pytest.mark.anyio
async def test_deovr_sections(client, tmp_path):
    register_videos(_library(tmp_path))
    data = (await client.get("/deovr")).json()
    assert [g["name"] for g in data["scenes"]] == ALL_SECTIONS
    sections = {g["name"]: [i["title"] for i in g["list"]] for g in data["scenes"]}
    assert sections["Hand-made Multi-Axis"] == ["Multi"]
    assert sections["Medium Speed"] == ["B", "C"]
    assert sections["Flat Videos"] == ["B", "C", "D", "Multi"]


@pytest.mark.anyio
async def test_empty_sections_omitted_and_no_old_groups(client):
    register_videos([
        {"path": "/Lib/Only.mp4", "name": "Only.mp4", "sourceName": "Lib",
         "funscriptPath": "/Lib/Only.funscript", "hasFunscript": True},
    ])
    hs = [g["name"] for g in (await client.get("/heresphere")).json()["library"]]
    deo = [g["name"] for g in (await client.get("/deovr")).json()["scenes"]]
    assert hs == ["Flat Videos"]
    assert deo == ["Flat Videos"]


@pytest.mark.anyio
async def test_filter_param_still_applies(client, tmp_path):
    register_videos(_library(tmp_path))
    hs = (await client.get("/heresphere?filter=vr")).json()["library"]
    assert [g["name"] for g in hs] == ["VR Videos", "Slow", "Short", "Recently Added"]
    deo = (await client.get("/deovr?filter=scripted")).json()["scenes"]
    assert "Flat Videos" in [g["name"] for g in deo]
    assert all(i["title"] != "D" for g in deo for i in g["list"])


@pytest.mark.anyio
async def test_section_names_are_plain_text(client, tmp_path):
    register_videos(_library(tmp_path))
    for name in [g["name"] for g in (await client.get("/heresphere")).json()["library"]] + \
                [g["name"] for g in (await client.get("/deovr")).json()["scenes"]]:
        assert name.isascii(), name


# --- Scene tags -----------------------------------------------------------

@pytest.mark.anyio
async def test_heresphere_scene_tags(client, tmp_path):
    register_videos(_library(tmp_path))
    m = _path_to_id(str(tmp_path / "Multi" / "Multi.mp4"))
    tags = [t["name"] for t in (await client.get(f"/heresphere/{m}")).json()["tags"]]
    assert tags[:6] == [
        "Studio:Multi", "Feature:Is scripted", "Format:Flat",
        "Speed:Fast", "Length:Long", "Feature:Hand-made multi-axis",
    ]
    # Chapter tags (every 5 min) still follow.
    assert "5 min" in tags and "15 min" in tags

    a = _path_to_id("/Lib/A_180_sbs.mp4")
    tags = [t["name"] for t in (await client.get(f"/heresphere/{a}")).json()["tags"]]
    assert tags == ["Studio:Lib", "Feature:Is scripted", "Format:VR", "Speed:Slow", "Length:Short"]


@pytest.mark.anyio
async def test_heresphere_scene_tags_without_stats(client, tmp_path):
    register_videos(_library(tmp_path))
    d = _path_to_id("/Lib/D.mp4")
    tags = [t["name"] for t in (await client.get(f"/heresphere/{d}")).json()["tags"]]
    assert tags == ["Studio:Lib", "Format:Flat"]


@pytest.mark.anyio
async def test_deovr_scene_categories(client, tmp_path):
    register_videos(_library(tmp_path))
    b = _path_to_id("/Lib/B.mp4")
    cats = (await client.get(f"/deovr/{b}")).json()["categories"]
    assert [c["tag"]["name"] for c in cats] == [
        "Studio:Lib", "Feature:Is scripted", "Format:Flat", "Speed:Medium", "Length:Medium",
    ]
    assert [c["tag"]["id"] for c in cats] == [1, 2, 3, 4, 5]
