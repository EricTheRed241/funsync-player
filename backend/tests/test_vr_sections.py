"""Headset library sections, filter tags, and hand-made multi-axis detection.

The HereSphere / DeoVR listings split the library into plain-text sections
(VR / Flat, speed, length, hand-made, recently added) and the scene details
carry matching filter tags. Rules live in `services.vr_sections`. Hand-made
means a hand-made companion axis file (decided at registration) or
non-empty axes embedded in the main script (background scan started at
registration).
"""

import json
import os
import sys
import time

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))

from httpx import AsyncClient, ASGITransport
from main import app
from routes import remote_sync
from routes.media import register_videos, register_playlists, register_ratings, register_categories, get_video_registry, _path_to_id
from services import vr_sections
from services.funscript import companion_axes, embedded_axes, has_handmade_companions, may_have_embedded_axes

DAY_MS = 24 * 60 * 60 * 1000
ALL_SECTIONS = [
    "VR", "Flat", "Slow", "Medium", "Fast",
    "Short (<5m)", "Medium (5-15m)", "Long (15m+)",
    "Hand-made", "Recently Added",
]


@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture
async def client():
    transport = ASGITransport(app=app)
    async with AsyncClient(transport=transport, base_url="http://test") as c:
        yield c


@pytest.fixture(autouse=True)
def _no_triage_state():
    """No ratings, playlists, categories or queued edits leak between tests."""
    register_ratings({})
    register_playlists([])
    register_categories([], {})
    remote_sync._reset_state_for_tests()
    yield
    register_ratings({})
    register_playlists([])
    register_categories([], {})
    remote_sync._reset_state_for_tests()


def _actions(n):
    return [{"at": i * 100, "pos": (i * 37) % 100} for i in range(n)]


def _wait_for_embedded_scan(timeout=5.0):
    """The embedded-axes scan runs on a background thread after
    registration; wait until every entry has its result."""
    deadline = time.monotonic() + timeout
    while any("embeddedAxes" not in v for v in get_video_registry().values()):
        assert time.monotonic() < deadline, "embedded-axes scan did not finish"
        time.sleep(0.01)


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
        (1, "Short (<5m)"), (299, "Short (<5m)"), (299.9, "Short (<5m)"),
        (300, "Medium (5-15m)"), (600, "Medium (5-15m)"), (900, "Medium (5-15m)"),
        (900.5, "Long (15m+)"), (901, "Long (15m+)"), (7200, "Long (15m+)"),
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


class TestHandmadeRule:
    def test_companions_or_embedded(self):
        assert vr_sections.is_handmade_multi_axis({"handmadeCompanions": True})
        assert vr_sections.is_handmade_multi_axis({"embeddedAxes": ["roll"]})
        assert vr_sections.is_handmade_multi_axis({"handmadeCompanions": True, "embeddedAxes": ["roll"]})

    def test_neither(self):
        assert not vr_sections.is_handmade_multi_axis({})
        assert not vr_sections.is_handmade_multi_axis({"handmadeCompanions": False, "embeddedAxes": []})
        assert not vr_sections.is_handmade_multi_axis(None)


class TestAxisKind:
    S = {"funscriptPath": "/v/T.funscript", "path": "/v/T.mp4"}

    def test_handmade_beats_generated(self):
        assert vr_sections.axis_kind({**self.S, "handmadeCompanions": True, "generatedCompanions": True}) == "handmade"
        assert vr_sections.axis_kind({**self.S, "embeddedAxes": ["roll"], "generatedCompanions": True}) == "handmade"

    def test_generated_then_single(self):
        assert vr_sections.axis_kind({**self.S, "embeddedAxes": [], "generatedCompanions": True}) == "generated"
        assert vr_sections.axis_kind({**self.S, "embeddedAxes": [], "generatedCompanions": False}) == "single"

    def test_undecided_until_embedded_check_ran(self):
        # Embedded axes would make it hand-made, so generated companions don't decide it yet.
        assert vr_sections.axis_kind({**self.S, "generatedCompanions": True}) is None
        assert vr_sections.axis_kind({"path": "/v/T.mp4", "embeddedAxes": []}) is None

    def test_flags_leave_out_scriptless_and_count_pending(self):
        registry = {
            "a": {**self.S, "handmadeCompanions": True},
            "b": {"funscriptPath": "/v/B.funscript", "path": "/v/B.mp4", "embeddedAxes": [], "generatedCompanions": True},
            "c": {"funscriptPath": "/v/C.funscript", "path": "/v/C.mp4"},
            "d": {"path": "/v/D.mp4", "embeddedAxes": []},
        }
        assert vr_sections.axis_flags(registry) == {"flags": {"/v/T.mp4": "handmade", "/v/B.mp4": "generated"}, "pending": 1}


class TestCompanionAxes:
    def test_split_handmade_and_generated(self):
        assert companion_axes("/v/Title.funscript", [
            "Title.funscript", "Title.roll.funscript", "Title.generated.twist.funscript",
            "Title.generated.pitch.funscript", "._Title.surge.funscript", "Other.generated.roll.funscript",
        ]) == (["roll"], ["pitch", "twist"])


class TestBuildSections:
    def test_order_and_empty_sections_omitted(self):
        now = 1_800_000_000_000
        entries = [
            ("a", {"avgSpeed": 400, "duration": 100}, True),
            ("b", {"avgSpeed": 100, "duration": 2000, "dateAdded": now, "embeddedAxes": ["pitch"]}, False),
            ("c", {}, False),
        ]
        sections = vr_sections.build_sections(entries, now_ms=now)
        assert sections == [
            ("VR", ["a"]),
            ("Flat", ["b", "c"]),
            ("Slow", ["b"]),
            ("Fast", ["a"]),
            ("Short (<5m)", ["a"]),
            ("Long (15m+)", ["b"]),
            ("Hand-made", ["b"]),
            ("Recently Added", ["b"]),
            ("Unrated", ["a", "b", "c"]),
        ]

    def test_section_order_is_the_approved_list(self):
        assert list(vr_sections.SECTION_ORDER) == ALL_SECTIONS

    def test_names_are_plain_text_and_unique(self):
        for name in vr_sections.SECTION_ORDER:
            assert name.isascii(), name
        assert len(set(vr_sections.SECTION_ORDER)) == len(vr_sections.SECTION_ORDER)


# --- Hand-made companion files --------------------------------------------

class TestHandmadeCompanions:
    def test_handmade_companion(self):
        assert has_handmade_companions("/v/Title.funscript", ["Title.funscript", "Title.twist.funscript"])

    def test_generated_companions_only(self):
        assert not has_handmade_companions("/v/Title.funscript", [
            "Title.funscript",
            "Title.generated.twist.funscript",
            "Title.generated.roll.funscript",
        ])

    def test_mixed_generated_and_handmade(self):
        assert has_handmade_companions("/v/Title.funscript", [
            "Title.generated.pitch.funscript", "Title.roll.funscript",
        ])

    def test_case_insensitive_and_aliases(self):
        assert has_handmade_companions("/v/Title.funscript", ["title.SURGE.Funscript"])
        assert has_handmade_companions("/v/Title.funscript", ["Title.suction.funscript"])
        assert not has_handmade_companions("/v/Title.funscript", ["Title.GENERATED.Twist.funscript"])

    def test_companion_of_another_script_ignored(self):
        # A variant's companion is not the main script's.
        assert not has_handmade_companions("/v/Title.funscript", ["Title (hard).twist.funscript"])
        assert not has_handmade_companions("/v/Title (soft).funscript", ["Title.twist.funscript"])

    def test_appledouble_ignored(self):
        assert not has_handmade_companions("/v/Title.funscript", ["._Title.twist.funscript"])

    def test_unknown_suffix_and_non_funscript_ignored(self):
        assert not has_handmade_companions("/v/Title.funscript", [
            "Title.intense.funscript", "Title.twist.json", "Title.twist.funscript.bak",
        ])

    def test_dotted_main_name(self):
        assert has_handmade_companions("/v/Mr.Smith.funscript", ["Mr.Smith.vib.funscript"])


# --- Embedded axes --------------------------------------------------------

class TestEmbeddedAxes:
    def test_format_a_additional_axes(self):
        assert embedded_axes({"actions": _actions(5), "additional_axes": [
            {"axis": "Twist", "actions": _actions(3)},
            {"axis": "rotate", "actions": _actions(3)},
            {"axis": "unknown", "actions": _actions(3)},
        ]}) == ["twist"]

    def test_format_b_raw(self):
        assert embedded_axes({"actions": _actions(5), "raw": {
            "pitch": {"actions": _actions(4)}, "roll": _actions(4),   # bare list isn't format B
        }}) == ["pitch"]

    def test_format_c_sibling_keys(self):
        assert embedded_axes({
            "actions": _actions(5), "twist": _actions(3), "Vibration": {"actions": _actions(3)},
            "metadata": {"title": "x"},
        }) == ["twist", "vib"]

    def test_format_d_inline_tcode(self):
        acts = [{"at": i * 100, "pos": 50, "R0": 10 * i, "R1": 5} for i in range(4)]
        acts.append({"at": 500, "pos": 50, "R2": 40})          # only one R2 point
        assert embedded_axes({"actions": acts}) == ["roll", "twist"]

    def test_format_e_axes_by_tcode(self):
        # Stellar Blade / Neurotomo / Rerirs / ZZL / DUMM shape.
        assert embedded_axes({"actions": _actions(5), "version": "1.1", "axes": [
            {"id": "L0", "actions": _actions(5)},   # main stroke, not an extra axis
            {"id": "r0", "actions": _actions(5)},
            {"id": "R1", "actions": _actions(5)},
            {"id": "R2", "actions": _actions(5)},
            {"id": "L1", "actions": _actions(5)},
            {"id": "L2", "actions": _actions(5)},
        ]}) == ["pitch", "roll", "surge", "sway", "twist"]

    def test_format_f_channels(self):
        # Nintendawg / Interceptor / Rondoudou shape ({metadata, actions}) and
        # Yae Miko / WickedR34 shape ({actions}).
        assert embedded_axes({"actions": _actions(5), "channels": {
            "twist": {"metadata": {}, "actions": _actions(5)},
            "roll": {"actions": _actions(5)},
            "pitch": _actions(5),
            "R2": {"actions": _actions(5)},
            "weird": {"actions": _actions(5)},
        }}) == ["pitch", "roll", "twist"]

    def test_axes_need_two_actions(self):
        assert embedded_axes({"actions": _actions(5), "axes": [
            {"id": "R0", "actions": _actions(1)}, {"id": "R1", "actions": []},
        ], "channels": {"roll": {"actions": _actions(1)}}}) == []

    def test_actions_need_numeric_at(self):
        bad = [{"at": "0", "pos": 1}, {"at": 100, "pos": 2}]
        assert embedded_axes({"axes": [{"id": "R0", "actions": bad}]}) == []

    def test_plain_script_and_junk(self):
        assert embedded_axes({"actions": _actions(10), "inverted": False, "range": 100}) == []
        assert embedded_axes([]) == []
        assert embedded_axes({"axes": "R0", "channels": [1, 2], "raw": 5, "additional_axes": {}}) == []

    def test_precheck(self):
        plain = json.dumps({"actions": _actions(10), "metadata": {"title": "Twist ending"}}).encode()
        assert not may_have_embedded_axes(plain)
        for doc in [
            {"axes": []}, {"channels": {}}, {"raw": {}}, {"additional_axes": []},
            {"Twist": []}, {"actions": [{"at": 0, "pos": 0, "R1": 3}]},
        ]:
            assert may_have_embedded_axes(json.dumps(doc).encode()), doc


# --- Registration ---------------------------------------------------------

def _touch(path, content="{}"):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def test_registration_marks_handmade_companions(tmp_path):
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
    assert reg[_path_to_id(str(hand / "Hand.mp4"))]["handmadeCompanions"] is True
    assert reg[_path_to_id(str(gen / "Gen.mp4"))]["handmadeCompanions"] is False
    assert reg[_path_to_id(str(tmp_path / "None.mp4"))]["handmadeCompanions"] is False
    assert reg[_path_to_id("/missing/X.mp4")]["handmadeCompanions"] is False


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
    assert reg[_path_to_id(str(folder / "A.mp4"))]["handmadeCompanions"] is False
    assert reg[_path_to_id(str(folder / "B.mp4"))]["handmadeCompanions"] is True


def test_registration_scans_embedded_axes(tmp_path):
    files = {
        "Channels.funscript": json.dumps({"actions": _actions(10), "channels": {
            "roll": {"actions": _actions(5)}, "pitch": {"actions": _actions(5)}}}),
        "Axes.funscript": "\ufeff" + json.dumps({"actions": _actions(10), "axes": [
            {"id": "R2", "actions": _actions(5)}]}),            # UTF-8 BOM
        "Plain.funscript": json.dumps({"actions": _actions(10)}),
        "Broken.funscript": '{"axes": [',
    }
    for name, content in files.items():
        _touch(tmp_path / name, content)
    register_videos(
        [{"path": str(tmp_path / f"{n}.mp4"), "name": f"{n}.mp4",
          "funscriptPath": str(tmp_path / f"{n}.funscript"), "avgSpeed": 0}
         for n in ["Channels", "Axes", "Plain", "Broken", "Missing"]]
        + [{"path": str(tmp_path / "NoScript.mp4"), "name": "NoScript.mp4"}]
    )
    _wait_for_embedded_scan()
    reg = get_video_registry()

    def axes(n):
        return reg[_path_to_id(str(tmp_path / f"{n}.mp4"))]["embeddedAxes"]

    assert axes("Channels") == ["pitch", "roll"]
    assert axes("Axes") == ["pitch"]
    assert axes("Plain") == []
    assert axes("Broken") == []
    assert axes("Missing") == []
    assert axes("NoScript") == []


# --- Endpoints ------------------------------------------------------------

def _library(tmp_path):
    """One video per section, real files so hand-made detection runs."""
    now = time.time() * 1000
    multi = tmp_path / "Multi"
    for f in ["Multi.funscript", "Multi.surge.funscript"]:
        _touch(multi / f)
    # Embedded axes next to generated companions: still hand-made.
    emb = tmp_path / "Emb"
    _touch(emb / "Emb.funscript", json.dumps({"actions": _actions(10), "axes": [
        {"id": "R1", "actions": _actions(5)}]}))
    _touch(emb / "Emb.generated.twist.funscript")
    # Only generated companions and a one-point embedded axis: not hand-made.
    gen = tmp_path / "Gen"
    _touch(gen / "Gen.funscript", json.dumps({"actions": _actions(10), "channels": {
        "roll": {"actions": _actions(1)}}}))
    _touch(gen / "Gen.generated.roll.funscript")
    return [
        # VR, Slow, Short, recently added
        {"path": "/Lib/A_180_sbs.mp4", "name": "A_180_sbs.mp4", "sourceName": "Lib",
         "funscriptPath": "/Lib/A.funscript", "hasFunscript": True,
         "avgSpeed": 249, "duration": 299, "dateAdded": now - DAY_MS},
        # Flat, Medium speed (lower edge), Medium length (lower edge), old
        {"path": "/Lib/B.mp4", "name": "B.mp4", "sourceName": "Lib",
         "funscriptPath": "/Lib/B.funscript", "hasFunscript": True,
         "avgSpeed": 250, "duration": 300, "dateAdded": now - 30 * DAY_MS},
        # Flat, Medium speed (upper edge), Medium length (upper edge)
        {"path": "/Lib/C.mp4", "name": "C.mp4",
         "funscriptPath": "/Lib/C.funscript", "hasFunscript": True,
         "avgSpeed": 349, "duration": 900},
        # Flat, Fast, Long, hand-made companion
        {"path": str(multi / "Multi.mp4"), "name": "Multi.mp4",
         "funscriptPath": str(multi / "Multi.funscript"), "hasFunscript": True,
         "avgSpeed": 350, "duration": 901},
        # Flat, no script / no duration: only in Flat
        {"path": "/Lib/D.mp4", "name": "D.mp4"},
        # Flat, hand-made through embedded axes (avgSpeed 0 = no speed bucket)
        {"path": str(emb / "Emb.mp4"), "name": "Emb.mp4",
         "funscriptPath": str(emb / "Emb.funscript"), "hasFunscript": True, "avgSpeed": 0},
        # Flat only
        {"path": str(gen / "Gen.mp4"), "name": "Gen.mp4",
         "funscriptPath": str(gen / "Gen.funscript"), "hasFunscript": True, "avgSpeed": 0},
    ]


def _register_library(tmp_path):
    register_videos(_library(tmp_path))
    _wait_for_embedded_scan()


def _ids(urls):
    return [u.rstrip("/").rsplit("/", 1)[1] for u in urls]


@pytest.mark.anyio
async def test_heresphere_sections(client, tmp_path):
    _register_library(tmp_path)
    data = (await client.get("/heresphere")).json()
    sections = {g["name"]: _ids(g["list"]) for g in data["library"]}
    assert [g["name"] for g in data["library"]] == ALL_SECTIONS + ["Unrated"]

    a, b, c, d = (_path_to_id(p) for p in ["/Lib/A_180_sbs.mp4", "/Lib/B.mp4", "/Lib/C.mp4", "/Lib/D.mp4"])
    m = _path_to_id(str(tmp_path / "Multi" / "Multi.mp4"))
    e = _path_to_id(str(tmp_path / "Emb" / "Emb.mp4"))
    g = _path_to_id(str(tmp_path / "Gen" / "Gen.mp4"))
    assert sections["VR"] == [a]
    assert sections["Flat"] == [b, c, d, e, g, m]   # sorted by title
    assert sections["Slow"] == [a]
    assert sections["Medium"] == [b, c]
    assert sections["Fast"] == [m]
    assert sections["Short (<5m)"] == [a]
    assert sections["Medium (5-15m)"] == [b, c]
    assert sections["Long (15m+)"] == [m]
    assert sections["Hand-made"] == [e, m]
    assert sections["Recently Added"] == [a]


@pytest.mark.anyio
async def test_deovr_sections(client, tmp_path):
    _register_library(tmp_path)
    data = (await client.get("/deovr")).json()
    assert [g["name"] for g in data["scenes"]] == ALL_SECTIONS + ["Unrated"]
    sections = {g["name"]: [i["title"] for i in g["list"]] for g in data["scenes"]}
    assert sections["Hand-made"] == ["Emb", "Multi"]
    assert sections["Medium"] == ["B", "C"]
    assert sections["Flat"] == ["B", "C", "D", "Emb", "Gen", "Multi"]


@pytest.mark.anyio
async def test_empty_sections_omitted(client):
    register_videos([
        {"path": "/Lib/Only.mp4", "name": "Only.mp4", "sourceName": "Lib",
         "funscriptPath": "/Lib/Only.funscript", "hasFunscript": True},
    ])
    _wait_for_embedded_scan()
    hs = [g["name"] for g in (await client.get("/heresphere")).json()["library"]]
    deo = [g["name"] for g in (await client.get("/deovr")).json()["scenes"]]
    assert hs == ["Flat", "Unrated"]
    assert deo == ["Flat", "Unrated"]


@pytest.mark.anyio
async def test_filter_param_still_applies(client, tmp_path):
    _register_library(tmp_path)
    hs = (await client.get("/heresphere?filter=vr")).json()["library"]
    assert [g["name"] for g in hs] == ["VR", "Slow", "Short (<5m)", "Recently Added", "Unrated"]
    deo = (await client.get("/deovr?filter=scripted")).json()["scenes"]
    assert "Flat" in [g["name"] for g in deo]
    assert all(i["title"] != "D" for g in deo for i in g["list"])


@pytest.mark.anyio
async def test_section_names_are_plain_text(client, tmp_path):
    _register_library(tmp_path)
    for name in [g["name"] for g in (await client.get("/heresphere")).json()["library"]] + \
                [g["name"] for g in (await client.get("/deovr")).json()["scenes"]]:
        assert name.isascii(), name


# --- Scene tags -----------------------------------------------------------

@pytest.mark.anyio
async def test_heresphere_scene_tags(client, tmp_path):
    _register_library(tmp_path)
    m = _path_to_id(str(tmp_path / "Multi" / "Multi.mp4"))
    tags = [t["name"] for t in (await client.get(f"/heresphere/{m}")).json()["tags"]]
    assert tags[:6] == [
        "Studio:Multi", "Feature:Is scripted", "Format:Flat",
        "Speed:Fast", "Length:Long (15m+)", "Feature:Hand-made multi-axis",
    ]
    # Chapter tags (every 5 min) still follow.
    assert "5 min" in tags and "15 min" in tags

    a = _path_to_id("/Lib/A_180_sbs.mp4")
    tags = [t["name"] for t in (await client.get(f"/heresphere/{a}")).json()["tags"]]
    assert tags == ["Studio:Lib", "Feature:Is scripted", "Format:VR", "Speed:Slow", "Length:Short (<5m)"]


@pytest.mark.anyio
async def test_heresphere_scene_tags_embedded_handmade(client, tmp_path):
    _register_library(tmp_path)
    e = _path_to_id(str(tmp_path / "Emb" / "Emb.mp4"))
    tags = [t["name"] for t in (await client.get(f"/heresphere/{e}")).json()["tags"]]
    assert tags == ["Studio:Emb", "Feature:Is scripted", "Format:Flat", "Feature:Hand-made multi-axis"]


@pytest.mark.anyio
async def test_heresphere_scene_tags_without_stats(client, tmp_path):
    _register_library(tmp_path)
    d = _path_to_id("/Lib/D.mp4")
    tags = [t["name"] for t in (await client.get(f"/heresphere/{d}")).json()["tags"]]
    assert tags == ["Studio:Lib", "Format:Flat"]


@pytest.mark.anyio
async def test_heresphere_scene_category_tags_follow_the_desktop(client, tmp_path):
    _register_library(tmp_path)
    register_categories(
        [{"id": "c1", "name": "Creampie"}, {"id": "c2", "name": "VR"}, {"id": "c3", "name": " "}],
        {"/Lib/D.mp4": ["c2", "c1", "c3", "gone"]},
    )
    d = _path_to_id("/Lib/D.mp4")
    tags = [t["name"] for t in (await client.get(f"/heresphere/{d}")).json()["tags"]]
    assert tags == ["Studio:Lib", "Format:Flat", "Category:Creampie", "Category:VR"]

    # Editing them in the headset changes nothing.
    await client.post(f"/heresphere/{d}", json={"tags": [{"name": "Studio:Lib"}]})
    assert remote_sync.library_edits.queue.pending() == []


@pytest.mark.anyio
async def test_deovr_scene_categories(client, tmp_path):
    _register_library(tmp_path)
    b = _path_to_id("/Lib/B.mp4")
    cats = (await client.get(f"/deovr/{b}")).json()["categories"]
    assert [c["tag"]["name"] for c in cats] == [
        "Studio:Lib", "Feature:Is scripted", "Format:Flat", "Speed:Medium", "Length:Medium (5-15m)",
    ]
    assert [c["tag"]["id"] for c in cats] == [1, 2, 3, 4, 5]
