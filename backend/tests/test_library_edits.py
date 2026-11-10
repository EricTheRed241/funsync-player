"""Ratings, favourites, playlist and play-queue edits from HereSphere and the
web remote.

Covers the edit channel to the desktop (services/library_edits.py +
routes/remote_sync.py), HereSphere's scene POST and playback events
(routes/vr_api.py), the triage and queue sections (services/vr_sections.py)
and the web remote write endpoints (routes/remote.py).
"""

import json
import time

import pytest
from fastapi.testclient import TestClient

from main import app
from routes import media, remote_sync, vr_api
from routes.media import _path_to_id, register_playlists, register_ratings, register_videos
from routes.vr_api import parse_heresphere_update
from services import library_edits, vr_sections
from services.library_edits import (
    KIND_PLAYLIST_ADD, KIND_PLAYLIST_REMOVE, KIND_RATING, LibraryEditQueue, QueueFull, normalize_rating,
)

A = "/Lib/A_180_sbs.mp4"
B = "/Lib/B.mp4"
C = "/Lib/C.mp4"


@pytest.fixture(autouse=True)
def fresh_state():
    register_ratings({})
    register_playlists([])
    media.register_tag_scores({})
    media.register_tag_score_info({})
    media.register_preference_scores({})
    media.register_artists([])
    media.register_favourite_artists([])
    media.register_favourite_artist_score({})
    media.register_queue({})
    media.register_watched({})
    vr_api._reset_queue_state_for_tests()
    remote_sync._reset_state_for_tests()
    yield
    register_ratings({})
    register_playlists([])
    media.register_tag_scores({})
    media.register_tag_score_info({})
    media.register_preference_scores({})
    media.register_artists([])
    media.register_favourite_artists([])
    media.register_favourite_artist_score({})
    media.register_queue({})
    media.register_watched({})
    vr_api._reset_queue_state_for_tests()
    remote_sync._reset_state_for_tests()


@pytest.fixture
def client():
    with TestClient(app) as c:
        yield c


@pytest.fixture
def library():
    register_videos([
        {"path": A, "name": "A_180_sbs.mp4", "sourceName": "Lib", "duration": 100},
        {"path": B, "name": "B.mp4", "sourceName": "Lib", "duration": 400},
        {"path": C, "name": "C.mp4", "sourceName": "Lib", "duration": 1000},
    ])
    register_playlists([
        {"id": "pl-1", "name": "Evening", "videoPaths": [C, A], "createdAt": 1},
        {"id": "pl-2", "name": "Empty", "videoPaths": [], "createdAt": 2},
    ])
    return {"A": _path_to_id(A), "B": _path_to_id(B), "C": _path_to_id(C)}


def _sections(client, url="/heresphere"):
    data = client.get(url).json()
    groups = data["library"] if "library" in data else data["scenes"]
    return {g["name"]: g["list"] for g in groups}, [g["name"] for g in groups]


def _ids(urls):
    return [u.rstrip("/").rsplit("/", 1)[1] for u in urls]


def _ack(ws, msg):
    ws.send_json({"type": "library-edit-ack", "seq": msg["edit"]["seq"]})


def _wait_until(pred, timeout=2.0):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(0.01)
    return False


# --- Rating validation ------------------------------------------------------

class TestNormalizeRating:
    @pytest.mark.parametrize("value, expected", [
        (0, 0.0), (0.5, 0.5), (4.5, 4.5), (5, 5.0), ("3.5", 3.5), (2.0, 2.0),
    ])
    def test_valid(self, value, expected):
        assert normalize_rating(value) == expected

    @pytest.mark.parametrize("value", [-0.5, 5.5, 4.3, 0.25, "x", "", None, True, False, float("nan"), [], {}])
    def test_invalid(self, value):
        assert normalize_rating(value) is None

    def test_rating_edit_rejects_bad_values(self):
        with pytest.raises(ValueError):
            library_edits.make_rating_edit(A, rating=4.3)
        with pytest.raises(ValueError):
            library_edits.make_rating_edit(A)
        with pytest.raises(ValueError):
            library_edits.make_rating_edit(A, favorite="yes")
        assert library_edits.make_rating_edit(A, rating="4.5")["rating"] == 4.5

    def test_playlist_edit_needs_id_or_name(self):
        with pytest.raises(ValueError):
            library_edits.make_playlist_edit(KIND_PLAYLIST_ADD, A)
        with pytest.raises(ValueError):
            library_edits.make_playlist_edit(KIND_PLAYLIST_ADD, A, name="  ")
        edit = library_edits.make_playlist_edit(KIND_PLAYLIST_ADD, A, playlist_id="pending:Triage")
        assert edit["playlistId"] is None and edit["name"] == "Triage"


# --- Queue + effective state -------------------------------------------------

class TestQueue:
    def test_in_order_with_monotonic_seq_and_ack(self):
        q = LibraryEditQueue()
        e1 = q.enqueue({"kind": KIND_RATING, "path": A, "rating": 3.0})
        e2 = q.enqueue({"kind": KIND_RATING, "path": A, "rating": 4.0})
        assert (e1["seq"], e2["seq"]) == (1, 2)
        assert [e["seq"] for e in q.pending()] == [1, 2]
        assert q.ack(1)["rating"] == 3.0
        assert q.ack(1) is None  # duplicate ack after a replay
        assert [e["seq"] for e in q.pending()] == [2]

    def test_persists_across_restart(self, tmp_path):
        q = LibraryEditQueue(persist_dir=str(tmp_path))
        q.enqueue({"kind": KIND_RATING, "path": A, "rating": 3.0})
        q.enqueue({"kind": KIND_PLAYLIST_ADD, "path": A, "name": "Triage"})
        q.ack(1)
        again = LibraryEditQueue(persist_dir=str(tmp_path))
        assert [(e["seq"], e["kind"]) for e in again.pending()] == [(2, KIND_PLAYLIST_ADD)]
        assert again.enqueue({"kind": KIND_RATING, "path": B, "rating": 1.0})["seq"] == 3

    def test_corrupt_file_is_ignored(self, tmp_path):
        (tmp_path / library_edits.QUEUE_FILE).write_text("{nope")
        assert LibraryEditQueue(persist_dir=str(tmp_path)).pending() == []

    def test_full_queue_rejects(self, monkeypatch):
        monkeypatch.setattr(library_edits, "MAX_PENDING", 2)
        q = LibraryEditQueue()
        q.enqueue({"kind": KIND_RATING, "path": A, "rating": 1.0})
        q.enqueue({"kind": KIND_RATING, "path": A, "rating": 2.0})
        with pytest.raises(QueueFull):
            q.enqueue({"kind": KIND_RATING, "path": A, "rating": 3.0})

    def test_effective_ratings_apply_in_order_and_clear(self):
        base = {A: {"rating": 2.0, "favorite": True, "updatedAt": 1}}
        pending = [
            {"kind": KIND_RATING, "path": A, "rating": 4.5, "favorite": None, "ts": 5},
            {"kind": KIND_RATING, "path": B, "rating": None, "favorite": True, "ts": 6},
            {"kind": KIND_RATING, "path": A, "rating": 0.0, "favorite": False, "ts": 7},
        ]
        out = library_edits.effective_ratings(base, pending)
        assert A not in out  # unrated + not favourite = no entry
        assert out[B] == {"rating": 0.0, "favorite": True, "updatedAt": 6}
        assert base[A]["rating"] == 2.0  # base untouched

    def test_effective_playlists_create_add_remove(self):
        base = [{"id": "pl-1", "name": "Evening", "videoPaths": [A]}]
        pending = [
            {"kind": KIND_PLAYLIST_ADD, "path": B, "playlistId": None, "name": "triage", "ts": 1},
            {"kind": KIND_PLAYLIST_ADD, "path": C, "playlistId": None, "name": "Triage", "ts": 2},
            {"kind": KIND_PLAYLIST_ADD, "path": B, "playlistId": "pl-1", "name": "Evening", "ts": 3},
            {"kind": KIND_PLAYLIST_REMOVE, "path": A, "playlistId": None, "name": "EVENING", "ts": 4},
        ]
        out = library_edits.effective_playlists(base, pending)
        assert out[0]["videoPaths"] == [B]
        assert out[1] == {"id": "pending:triage", "name": "triage", "videoPaths": [B, C],
                          "createdAt": 1, "pending": True}
        assert base[0]["videoPaths"] == [A]  # base untouched


# --- Edit channel over the observer socket ------------------------------------

class TestEditChannel:
    def test_queued_while_desktop_offline_then_replayed_in_order(self, client, library):
        r1 = client.post("/api/remote/rating", json={"videoId": library["A"], "rating": 3})
        r2 = client.post("/api/remote/rating", json={"videoId": library["A"], "rating": 4.5})
        r3 = client.post("/api/remote/playlists/add-video", json={"videoId": library["A"], "name": "Triage"})
        assert [r.json()["delivered"] for r in (r1, r2, r3)] == [False, False, False]
        assert len(library_edits.queue.pending()) == 3

        with client.websocket_connect("/api/remote/sync/observe") as ws:
            msgs = [ws.receive_json() for _ in range(3)]
            assert [m["type"] for m in msgs] == ["library-edit"] * 3
            assert [m["edit"]["seq"] for m in msgs] == [1, 2, 3]
            assert [m["edit"].get("rating") for m in msgs[:2]] == [3.0, 4.5]
            assert msgs[2]["edit"]["kind"] == KIND_PLAYLIST_ADD and msgs[2]["edit"]["path"] == A
            for m in msgs:
                _ack(ws, m)
            assert _wait_until(lambda: library_edits.queue.pending() == [])

        # Acked edits stay visible until the desktop's re-registration.
        assert library_edits.rating_entry(media._ratings, A) == (4.5, False)
        assert A in media._playlists[-1]["videoPaths"] and media._playlists[-1]["name"] == "Triage"

    def test_unacked_edits_replay_on_reconnect(self, client, library):
        with client.websocket_connect("/api/remote/sync/observe") as ws:
            r = client.post("/api/remote/rating", json={"videoId": library["B"], "favorite": True})
            assert r.json()["delivered"] is True
            first = ws.receive_json()
            assert first["edit"]["favorite"] is True
            # Desktop dies before acking.
        with client.websocket_connect("/api/remote/sync/observe") as ws:
            again = ws.receive_json()
            assert again["edit"]["seq"] == first["edit"]["seq"]
            _ack(ws, again)
            assert _wait_until(lambda: library_edits.queue.pending() == [])

    def test_rejected_edit_is_dropped(self, client, library):
        client.post("/api/remote/rating", json={"videoId": library["B"], "rating": 2})
        with client.websocket_connect("/api/remote/sync/observe") as ws:
            msg = ws.receive_json()
            ws.send_json({"type": "library-edit-ack", "seq": msg["edit"]["seq"], "error": "video gone"})
            assert _wait_until(lambda: library_edits.queue.pending() == [])
        # Not folded into the registered state.
        assert client.get("/api/remote/videos").json()["videos"][1]["rating"] == 0

    def test_ack_is_not_forwarded_to_phone(self, client, library):
        client.post("/api/remote/rating", json={"videoId": library["B"], "rating": 2})
        with client.websocket_connect("/api/remote/sync") as phone:
            with client.websocket_connect("/api/remote/sync/observe") as ws:
                assert ws.receive_json()["type"] == "phone-connected"
                msg = ws.receive_json()
                _ack(ws, msg)
                ws.send_json({"type": "device-status", "ok": True})
                assert phone.receive_json()["type"] == "device-status"


# --- HereSphere ---------------------------------------------------------------

class TestHereSphereParsing:
    PLAYLISTS = [
        {"id": "pl-1", "name": "Evening", "videoPaths": [A]},
        {"id": "pl-2", "name": "Triage", "videoPaths": []},
    ]

    def test_load_without_edits(self):
        assert parse_heresphere_update({"username": "", "password": ""}, A, {}, self.PLAYLISTS) == []
        assert parse_heresphere_update(None, A, {}, []) == []

    def test_unchanged_values_produce_nothing(self):
        ratings = {A: {"rating": 4.5, "favorite": True}}
        body = {"rating": 4.5, "isFavorite": True,
                "tags": [{"name": "Studio:Lib"}, {"name": "Playlist:Evening"}]}
        assert parse_heresphere_update(body, A, ratings, self.PLAYLISTS) == []

    def test_rating_and_favorite(self):
        edits = parse_heresphere_update({"rating": 4.5, "isFavorite": True}, A, {}, [])
        assert len(edits) == 1
        assert edits[0]["kind"] == KIND_RATING
        assert (edits[0]["rating"], edits[0]["favorite"], edits[0]["source"]) == (4.5, True, "heresphere")

    def test_only_changed_field_is_sent(self):
        ratings = {A: {"rating": 3.0, "favorite": True}}
        [edit] = parse_heresphere_update({"rating": 3.5, "isFavorite": True}, A, ratings, [])
        assert (edit["rating"], edit["favorite"]) == (3.5, None)

    def test_clearing_rating(self):
        [edit] = parse_heresphere_update({"rating": 0}, A, {A: {"rating": 2.0}}, [])
        assert edit["rating"] == 0.0

    def test_invalid_rating_ignored(self):
        assert parse_heresphere_update({"rating": 4.3, "isFavorite": "yes"}, A, {}, []) == []

    def test_playlist_tag_added_existing_and_new(self):
        body = {"tags": [{"name": "Playlist:Evening"}, {"name": "playlist: triage "},
                         {"name": "Playlist:New One"}, {"name": "Genre:Whatever"},
                         {"name": "5 min", "start": 300000, "end": 600000, "track": 0}]}
        edits = parse_heresphere_update(body, A, {}, self.PLAYLISTS)
        assert [(e["kind"], e["playlistId"], e["name"]) for e in edits] == [
            (KIND_PLAYLIST_ADD, "pl-2", "Triage"),
            (KIND_PLAYLIST_ADD, None, "New One"),
        ]

    def test_playlist_tag_removed(self):
        edits = parse_heresphere_update({"tags": [{"name": "Studio:Lib"}]}, A, {}, self.PLAYLISTS)
        assert [(e["kind"], e["playlistId"], e["name"]) for e in edits] == [
            (KIND_PLAYLIST_REMOVE, "pl-1", "Evening"),
        ]

    def test_absent_tags_leave_playlists_alone(self):
        assert parse_heresphere_update({"isFavorite": False}, A, {}, self.PLAYLISTS) == []

    def test_with_served_playlists_only_user_changes_count(self):
        # Served with Evening only. Since then the phone put A in Triage (so
        # it's a member) and the headset's stale save lacks that tag: no remove.
        playlists = [
            {"id": "pl-1", "name": "Evening", "videoPaths": [A]},
            {"id": "pl-2", "name": "Triage", "videoPaths": [A]},
            {"id": "pl-3", "name": "Later", "videoPaths": []},
        ]
        stale = {"tags": [{"name": "Playlist:Evening"}]}
        assert parse_heresphere_update(stale, A, {}, playlists, None, frozenset({"evening"})) == []
        # The user deletes Evening and adds Later in the headset: both count.
        edited = {"tags": [{"name": "Playlist:Later"}]}
        edits = parse_heresphere_update(edited, A, {}, playlists, None, frozenset({"evening"}))
        assert [(e["kind"], e["playlistId"]) for e in edits] == [
            (KIND_PLAYLIST_ADD, "pl-3"), (KIND_PLAYLIST_REMOVE, "pl-1"),
        ]

    def test_with_served_playlists_a_stale_tag_does_not_refile(self):
        # Served with Evening; the phone took A out of it since. A rating
        # saved with the old tags must not put it back.
        playlists = [{"id": "pl-1", "name": "Evening", "videoPaths": []}]
        body = {"rating": 4, "tags": [{"name": "Playlist:Evening"}]}
        edits = parse_heresphere_update(body, A, {}, playlists, None, frozenset({"evening"}))
        assert [e["kind"] for e in edits] == [KIND_RATING]


class TestHereSphereEndpoint:
    def test_scene_advertises_write_support_and_current_state(self, client, library):
        register_ratings({A: {"rating": 3.5, "favorite": True, "updatedAt": 1}})
        scene = client.get(f"/heresphere/{library['A']}").json()
        assert (scene["rating"], scene["isFavorite"]) == (3.5, True)
        assert (scene["writeRating"], scene["writeFavorite"], scene["writeTags"], scene["writeHSP"]) == (
            True, True, True, False)
        assert "Playlist:Evening" in [t["name"] for t in scene["tags"]]
        assert client.get(f"/deovr/{library['A']}").json()["isFavorite"] is True

    def test_post_rating_favorite_and_playlist_tag(self, client, library):
        a = library["A"]
        body = {"rating": 4.5, "isFavorite": True,
                "tags": [{"name": "Playlist:Evening"}, {"name": "Playlist:Triage"}]}
        scene = client.post(f"/heresphere/{a}", json=body).json()
        # Reflected straight away, before the desktop has seen it.
        assert (scene["rating"], scene["isFavorite"]) == (4.5, True)
        assert {"Playlist:Evening", "Playlist:Triage"} <= {t["name"] for t in scene["tags"]}
        kinds = [(e["kind"], e.get("name")) for e in library_edits.queue.pending()]
        assert kinds == [(KIND_RATING, None), (KIND_PLAYLIST_ADD, "Triage")]

        sections, names = _sections(client)
        assert _ids(sections["Favorites"]) == [a]
        assert _ids(sections["Top Rated"]) == [a]
        assert _ids(sections["Playlist: Triage"]) == [a]
        assert a not in _ids(sections["Unrated"])
        assert names[-2:] == ["Playlist: Evening", "Playlist: Triage"]

        # Re-sending the same state (HereSphere does) queues nothing new.
        client.post(f"/heresphere/{a}", json=body)
        assert len(library_edits.queue.pending()) == 2

    def test_post_removing_playlist_tag(self, client, library):
        client.post(f"/heresphere/{library['A']}", json={"tags": []})
        [edit] = library_edits.queue.pending()
        assert (edit["kind"], edit["playlistId"]) == (KIND_PLAYLIST_REMOVE, "pl-1")
        _, names = _sections(client)
        assert "Playlist: Evening" in names  # C is still in it
        sections, _ = _sections(client)
        assert _ids(sections["Playlist: Evening"]) == [library["C"]]

    def test_stale_save_after_phone_unfiles_does_not_refile(self, client, library):
        a = library["A"]
        client.get(f"/heresphere/{a}")  # served with Playlist:Evening
        client.post("/api/remote/playlists/remove-video", json={"videoId": a, "playlistId": "pl-1"})
        # The headset still shows Evening and saves a rating with it.
        client.post(f"/heresphere/{a}", json={"rating": 4, "tags": [{"name": "Playlist:Evening"}]})
        assert [e["kind"] for e in library_edits.queue.pending()] == [KIND_PLAYLIST_REMOVE, KIND_RATING]

    def test_post_with_non_json_body_still_loads(self, client, library):
        r = client.post(f"/heresphere/{library['A']}", content=b"not json")
        assert r.status_code == 200
        assert library_edits.queue.pending() == []


# --- Sections -------------------------------------------------------------------

class TestTriageSections:
    def test_favorites_top_rated_unrated_and_playlists(self):
        entries = [("a", {"path": A}, True), ("b", {"path": B}, False), ("c", {"path": C}, False)]
        ratings = {
            A: {"rating": 4.0, "favorite": False},
            B: {"rating": 3.5, "favorite": True},
        }
        playlists = [
            {"name": "Zed", "videoPaths": [C, A, "/gone.mp4"]},
            {"name": "Empty", "videoPaths": []},
            {"name": "Alpha", "videoPaths": [B]},
        ]
        sections = vr_sections.build_sections(entries, now_ms=0, ratings=ratings, playlists=playlists)
        names = [name for name, _ in sections]
        assert names[:2] == ["VR", "Flat"]
        assert sections[-6:] == [
            ("Favorites", ["b"]),
            ("Top Rated", ["a"]),
            ("Unrated", ["c"]),
            ("Watched", ["a", "b"]),           # rated; no updatedAt, so by path
            ("Playlist: Zed", ["c", "a"]),     # playlist order, not title order
            ("Playlist: Alpha", ["b"]),        # desktop's playlist order; empty one dropped
        ]

    def test_names_plain_text(self):
        for name in vr_sections.TRIAGE_SECTION_ORDER:
            assert name.isascii()

    def test_playlist_tags_round_trip(self):
        playlists = [{"name": "Evening", "videoPaths": [A]}, {"name": "Other", "videoPaths": [B]}]
        tags = vr_sections.playlist_tags(A, playlists)
        assert tags == ["Playlist:Evening"]
        assert vr_sections.playlist_names_from_tags([{"name": t} for t in tags]) == ["Evening"]
        assert vr_sections.playlist_names_from_tags(
            [{"name": "Playlist:x"}, {"name": "PLAYLIST:X"}, {"name": "Playlist:"}, "junk"]) == ["x"]

    def test_deovr_gets_the_same_sections(self, client, library):
        register_ratings({B: {"rating": 5.0, "favorite": True}})
        sections, names = _sections(client, "/deovr")
        assert [i["title"] for i in sections["Favorites"]] == ["B"]
        assert [i["title"] for i in sections["Top Rated"]] == ["B"]
        assert [i["title"] for i in sections["Playlist: Evening"]] == ["C", "A_180_sbs"]
        assert "Playlist: Empty" not in names

    def test_headset_rating_makes_a_video_watched_on_the_next_listing(self, client, library):
        # Evening (pl-1: C, A) is a playlist whose videos sink, like Discovery.
        register_ratings({B: {"rating": 3.0, "favorite": False, "updatedAt": 100}})
        media.register_watched({"sink": True, "playlistIds": ["pl-1"]})
        sections, _ = _sections(client)
        assert _ids(sections["Flat"]) == [library["C"], library["B"]]
        assert _ids(sections["Playlist: Evening"]) == [library["C"], library["A"]]
        assert _ids(sections["Watched"]) == [library["B"]]

        # Rated in the headset (still pending for the desktop); A only
        # becomes a favourite, which doesn't make it watched.
        scene = client.post(f"/heresphere/{library['C']}", json={"rating": 4}).json()
        assert "Status:Watched" in [t["name"] for t in scene["tags"]]
        client.post(f"/heresphere/{library['A']}", json={"isFavorite": True})
        assert len(library_edits.queue.pending()) == 2

        sections, _ = _sections(client)
        assert _ids(sections["Playlist: Evening"]) == [library["A"], library["C"]]
        assert _ids(sections["Flat"]) == [library["B"], library["C"]]
        assert _ids(sections["Watched"]) == [library["C"], library["B"]]   # newest rating first
        assert _ids(sections["Favorites"]) == [library["A"]]
        tags = [t["name"] for t in client.get(f"/heresphere/{library['A']}").json()["tags"]]
        assert "Status:Watched" not in tags


# --- Web remote endpoints ---------------------------------------------------------

class TestRemoteEndpoints:
    def test_videos_carry_rating_and_favorite(self, client, library):
        register_ratings({A: {"rating": 2.5, "favorite": True}})
        videos = {v["id"]: v for v in client.get("/api/remote/videos").json()["videos"]}
        assert (videos[library["A"]]["rating"], videos[library["A"]]["favorite"]) == (2.5, True)
        assert (videos[library["B"]]["rating"], videos[library["B"]]["favorite"]) == (0, False)

    def test_rating_endpoint(self, client, library):
        r = client.post("/api/remote/rating", json={"videoId": library["B"], "rating": 4.5})
        assert r.status_code == 200
        body = r.json()
        assert (body["ok"], body["delivered"], body["rating"], body["favorite"]) == (True, False, 4.5, False)
        r = client.post("/api/remote/rating", json={"videoId": library["B"], "favorite": True})
        assert (r.json()["rating"], r.json()["favorite"]) == (4.5, True)
        [edit1, edit2] = library_edits.queue.pending()
        assert edit1["source"] == "remote" and edit1["path"] == B

    @pytest.mark.parametrize("payload, status", [
        ({"rating": 4.3}, 400),
        ({"rating": 6}, 400),
        ({}, 400),
        ({"favorite": "yes"}, 400),
    ])
    def test_rating_endpoint_rejects(self, client, library, payload, status):
        r = client.post("/api/remote/rating", json={"videoId": library["B"], **payload})
        assert r.status_code == status
        assert library_edits.queue.pending() == []

    def test_unknown_video_404(self, client, library):
        assert client.post("/api/remote/rating", json={"videoId": "nope", "rating": 1}).status_code == 404
        assert client.post("/api/remote/playlists/add-video",
                           json={"videoId": "nope", "name": "x"}).status_code == 404

    def test_add_to_existing_and_new_playlist(self, client, library):
        r = client.post("/api/remote/playlists/add-video", json={"videoId": library["B"], "playlistId": "pl-1"})
        assert r.json()["ok"] is True
        client.post("/api/remote/playlists/add-video", json={"videoId": library["B"], "name": "Triage"})
        playlists = {p["name"]: p for p in client.get("/api/remote/playlists").json()["playlists"]}
        assert library["B"] in playlists["Evening"]["videoIds"]
        assert playlists["Triage"]["videoIds"] == [library["B"]]
        assert playlists["Triage"]["pending"] is True
        assert playlists["Evening"]["pending"] is False
        # Adding to the pending one by its placeholder id resolves by name.
        client.post("/api/remote/playlists/add-video",
                    json={"videoId": library["C"], "playlistId": playlists["Triage"]["id"]})
        playlists = {p["name"]: p for p in client.get("/api/remote/playlists").json()["playlists"]}
        assert playlists["Triage"]["videoIds"] == [library["B"], library["C"]]

    def test_remove_from_playlist(self, client, library):
        r = client.post("/api/remote/playlists/remove-video", json={"videoId": library["A"], "playlistId": "pl-1"})
        assert r.status_code == 200
        playlists = {p["name"]: p for p in client.get("/api/remote/playlists").json()["playlists"]}
        assert playlists["Evening"]["videoIds"] == [library["C"]]

    def test_playlist_endpoint_needs_target(self, client, library):
        assert client.post("/api/remote/playlists/add-video",
                           json={"videoId": library["A"]}).status_code == 400

    def test_full_queue_is_503(self, client, library, monkeypatch):
        monkeypatch.setattr(library_edits, "MAX_PENDING", 0)
        r = client.post("/api/remote/rating", json={"videoId": library["A"], "rating": 1})
        assert r.status_code == 503

    def test_registration_replaces_base_state(self, client, library):
        client.post("/api/media/register", json={
            "ratings": {C: {"rating": 1.5, "favorite": False, "updatedAt": 9}},
        })
        videos = {v["id"]: v for v in client.get("/api/remote/videos").json()["videos"]}
        assert videos[library["C"]]["rating"] == 1.5


# --- Tag scores and Recalculate scores ----------------------------------------

class TestTagScores:
    @pytest.mark.parametrize("raw, expected", [
        (1.5, 1.5), (1.2, 1.0), (1.25, 1.5), (-1.25, -1.0), (-1.3, -1.5), (0.24, 0.0),
        ("2.5", 2.5), (11, 10.0), (-99, -10.0), (-10, -10.0),
        (None, None), (True, None), ("", None), ("x", None), (float("nan"), None), (float("inf"), None),
        ([], None), (10 ** 400, None),
    ])
    def test_normalize_steps_and_clamps(self, raw, expected):
        """Same rule as renderer/js/preference-score.js::normalizeTagScore."""
        assert library_edits.normalize_tag_score(raw) == expected

    def test_no_negative_zero(self):
        assert str(library_edits.normalize_tag_score(-0.2)) == "0.0"

    def test_effective_scores_overlay_pending_edits(self):
        base = {"riding": 2.0, "pmv": 1.0}
        pending = [
            library_edits.make_tag_score_edit(" Riding ", 3),
            library_edits.make_tag_score_edit("pmv", 0),
            library_edits.make_tag_score_edit("ahegao", -1.5),
            library_edits.make_recalculate_edit(),
        ]
        assert library_edits.effective_tag_scores(base, pending) == {"riding": 3.0, "ahegao": -1.5}
        assert base == {"riding": 2.0, "pmv": 1.0}

    def test_registration_cleans_keys_and_values(self, client):
        client.post("/api/media/register", json={
            "tagScores": {" Riding ": 2, "pmv": 0, "bad": "x", "ahegao": -1.26, "": 3},
            "tagScoreInfo": {
                "groups": {" Riding ": "Riding", "cowgirl": "Riding", "odd": 3, "": "X"},
                "rules": {"vr": 3, "hentaiOnly": -3, "bad": "x", "flag": True},
            },
        })
        assert client.get("/api/remote/tag-scores").json() == {
            "scores": {"riding": 2.0, "ahegao": -1.5}, "pending": [], "recalculating": False,
            "groups": {"riding": "Riding", "cowgirl": "Riding"}, "rules": {"vr": 3, "hentaiOnly": -3},
        }
        # A registration without tagScoreInfo leaves it alone.
        client.post("/api/media/register", json={"tagScores": {}})
        assert client.get("/api/remote/tag-scores").json()["groups"] == {"riding": "Riding", "cowgirl": "Riding"}

    @pytest.mark.parametrize("payload", [{"score": 1}, {"tag": "  ", "score": 1}, {"tag": "x"}, {"tag": "x", "score": "lots"}])
    def test_endpoint_rejects(self, client, payload):
        assert client.post("/api/remote/tag-score", json=payload).status_code == 400
        assert library_edits.queue.pending() == []

    def test_set_while_desktop_offline_then_applied(self, client):
        client.post("/api/media/register", json={"tagScores": {"pmv": 1}})
        r = client.post("/api/remote/tag-score", json={"tag": "Riding", "score": 2.3})
        assert r.json() == {"ok": True, "seq": 1, "delivered": False, "tag": "riding", "score": 2.5}
        client.post("/api/remote/tag-score", json={"tag": "pmv", "score": 0})
        state = client.get("/api/remote/tag-scores").json()
        assert state == {"scores": {"riding": 2.5}, "pending": ["pmv", "riding"], "recalculating": False, "groups": {}, "rules": {}}

        with client.websocket_connect("/api/remote/sync/observe") as ws:
            msgs = [ws.receive_json() for _ in range(2)]
            assert [(m["edit"]["kind"], m["edit"]["tag"], m["edit"]["score"]) for m in msgs] == [
                ("tag-score", "riding", 2.5), ("tag-score", "pmv", 0.0),
            ]
            for m in msgs:
                _ack(ws, m)
            assert _wait_until(lambda: library_edits.queue.pending() == [])
        # Folded into the registered state until the desktop registers again.
        assert client.get("/api/remote/tag-scores").json() == {
            "scores": {"riding": 2.5}, "pending": [], "recalculating": False, "groups": {}, "rules": {},
        }

    def test_recalculate_waits_for_the_desktop(self, client):
        r = client.post("/api/remote/recalculate-scores").json()
        assert (r["ok"], r["delivered"]) == (True, False)
        assert client.get("/api/remote/tag-scores").json()["recalculating"] is True
        # Asking again while one waits returns that one.
        again = client.post("/api/remote/recalculate-scores").json()
        assert (again["seq"], again["alreadyPending"]) == (r["seq"], True)
        assert len(library_edits.queue.pending()) == 1

        with client.websocket_connect("/api/remote/sync/observe") as ws:
            msg = ws.receive_json()
            assert msg["edit"]["kind"] == "recalculate-scores" and "path" not in msg["edit"]
            # The desktop scores the library, registers the scores, then acks.
            client.post("/api/media/register", json={"preferenceScores": {A: 7.5}})
            _ack(ws, msg)
            assert _wait_until(lambda: library_edits.queue.pending() == [])
        assert client.get("/api/remote/tag-scores").json()["recalculating"] is False
        assert media.get_preference_score(A) == 7.5

    def test_recalculate_delivered_when_desktop_connected(self, client):
        with client.websocket_connect("/api/remote/sync/observe") as ws:
            r = client.post("/api/remote/recalculate-scores").json()
            assert r["delivered"] is True
            assert ws.receive_json()["edit"]["seq"] == r["seq"]

    def test_tag_edits_survive_a_backend_restart(self, tmp_path):
        q = LibraryEditQueue(persist_dir=str(tmp_path))
        q.enqueue(library_edits.make_tag_score_edit("riding", 1))
        q.enqueue(library_edits.make_recalculate_edit())
        again = LibraryEditQueue(persist_dir=str(tmp_path))
        assert [e["kind"] for e in again.pending()] == ["tag-score", "recalculate-scores"]


# --- Favourite artists --------------------------------------------------------

class TestFavouriteArtists:
    def test_artist_key_folds_case_unicode_and_space(self):
        assert library_edits.artist_key("  Ünder ") == library_edits.artist_key("u\u0308nder")
        assert library_edits.artist_key("STRASSE") == library_edits.artist_key("straße")
        assert library_edits.artist_key(None) == "" and library_edits.artist_key(3) == ""

    @pytest.mark.parametrize("artist, favourite", [
        (None, True), ("", True), ("   ", False), (3, True), ("Mira", None), ("Mira", 1), ("Mira", "true"),
    ])
    def test_artist_edit_rejects(self, artist, favourite):
        with pytest.raises(ValueError):
            library_edits.make_favourite_artist_edit(artist, favourite)

    def test_artist_edit_keeps_spelling(self):
        assert library_edits.make_favourite_artist_edit("  MiraBot ", False, source="remote") == {
            "kind": "favourite-artist", "artist": "MiraBot", "favourite": False, "source": "remote",
        }

    @pytest.mark.parametrize("raw, expected", [(5, 5.0), (2.3, 2.5), (-99, -10.0), ("1.5", 1.5), (0, 0.0)])
    def test_score_edit_steps_and_clamps(self, raw, expected):
        assert library_edits.make_favourite_score_edit(raw)["score"] == expected

    @pytest.mark.parametrize("raw", [None, True, "lots", [], float("nan")])
    def test_score_edit_rejects(self, raw):
        with pytest.raises(ValueError):
            library_edits.make_favourite_score_edit(raw)

    def test_effective_favourites_overlay_pending_edits_in_order(self):
        base = ["Mira", "Zeta", "Kai"]
        pending = [
            library_edits.make_favourite_artist_edit("MIRA", False),
            library_edits.make_favourite_artist_edit("newcomer", True),
            library_edits.make_favourite_artist_edit("zeta", True),
            library_edits.make_favourite_artist_edit("Newcomer", False),
            library_edits.make_favourite_artist_edit("NewComer", True),
            library_edits.make_tag_score_edit("riding", 1),
        ]
        # Case-insensitive; a later edit wins and its spelling replaces the old one.
        assert library_edits.effective_favourite_artists(base, pending) == ["Kai", "zeta", "NewComer"]
        assert base == ["Mira", "Zeta", "Kai"]

    def test_effective_score_takes_the_last_pending_edit(self):
        assert library_edits.effective_favourite_score(5.0, []) == 5.0
        assert library_edits.effective_favourite_score(None, []) is None
        pending = [library_edits.make_favourite_score_edit(3), library_edits.make_favourite_score_edit(-2)]
        assert library_edits.effective_favourite_score(5.0, pending) == -2.0

    def test_registration_drops_junk(self, client):
        client.post("/api/media/register", json={
            "artists": [
                {"name": " Mira ", "videoPaths": [A, 3, None, ""]},
                {"name": "MIRA", "videoPaths": [A, B]},
                {"name": "", "videoPaths": [C]},
                {"videoPaths": [C]},
                "Kai",
                {"name": "Kai", "videoPaths": "nope"},
            ],
            "favouriteArtists": [" Zeta ", "zeta", "", 7, None, "Kai"],
            "favouriteArtistScore": {"score": 2.3, "default": 5},
        })
        assert media.get_artists() == [{"name": "Mira", "videoPaths": [A, B]}, {"name": "Kai", "videoPaths": []}]
        assert media.get_favourite_artists() == ["Zeta", "Kai"]
        assert media.get_favourite_artist_score() == {"score": 2.5, "default": 5.0}
        # Each key is independent; a missing score falls back to the default.
        client.post("/api/media/register", json={"favouriteArtistScore": {"default": 5}})
        assert media.get_favourite_artist_score() == {"score": 5.0, "default": 5.0}
        assert media.get_artists()[0]["name"] == "Mira"
        client.post("/api/media/register", json={"favouriteArtistScore": "junk", "favouriteArtists": "junk"})
        assert media.get_favourite_artist_score() == {"score": None, "default": None}
        assert media.get_favourite_artists() == []

    def test_list_merges_artists_and_favourites(self, client, library):
        client.post("/api/media/register", json={
            "artists": [
                {"name": "beta", "videoPaths": [B, "/Lib/gone.mp4"]},
                {"name": "Mira", "videoPaths": [C, A]},
                {"name": "alpha", "videoPaths": []},
            ],
            "favouriteArtists": ["MIRA", "Zoë"],
            "favouriteArtistScore": {"score": 5, "default": 5},
        })
        data = client.get("/api/remote/artists").json()
        assert data == {
            "artists": [
                {"id": "mira", "name": "Mira", "videoCount": 2, "totalDuration": 1100.0,
                 "videoIds": [library["C"], library["A"]], "favourite": True, "pending": False},
                {"id": "zoë", "name": "Zoë", "videoCount": 0, "totalDuration": 0.0,
                 "videoIds": [], "favourite": True, "pending": False},
                {"id": "alpha", "name": "alpha", "videoCount": 0, "totalDuration": 0.0,
                 "videoIds": [], "favourite": False, "pending": False},
                {"id": "beta", "name": "beta", "videoCount": 1, "totalDuration": 400.0,
                 "videoIds": [library["B"]], "favourite": False, "pending": False},
            ],
            "score": 5.0, "defaultScore": 5.0, "scorePending": False,
        }

    def test_pending_edits_overlay_the_list(self, client, library):
        client.post("/api/media/register", json={
            "artists": [{"name": "Mira", "videoPaths": [A]}, {"name": "beta", "videoPaths": [B]}],
            "favouriteArtists": ["Mira", "Zeta"],
            "favouriteArtistScore": {"score": 5, "default": 5},
        })
        client.post("/api/remote/favourite-artist", json={"artist": "MIRA", "favourite": False})
        client.post("/api/remote/favourite-artist", json={"artist": "Beta", "favourite": True})
        client.post("/api/remote/favourite-score", json={"score": 7})
        data = client.get("/api/remote/artists").json()
        rows = [(a["id"], a["name"], a["favourite"], a["pending"]) for a in data["artists"]]
        assert rows == [("beta", "beta", True, True), ("zeta", "Zeta", True, False), ("mira", "Mira", False, True)]
        assert (data["score"], data["defaultScore"], data["scorePending"]) == (7.0, 5.0, True)

    @pytest.mark.parametrize("payload", [
        {"favourite": True}, {"artist": "  ", "favourite": True}, {"artist": "Mira"}, {"artist": "Mira", "favourite": "yes"},
        {"artist": 5, "favourite": True},
    ])
    def test_artist_endpoint_rejects(self, client, payload):
        assert client.post("/api/remote/favourite-artist", json=payload).status_code == 400
        assert library_edits.queue.pending() == []

    @pytest.mark.parametrize("payload", [{}, {"score": None}, {"score": "lots"}, {"score": True}])
    def test_score_endpoint_rejects(self, client, payload):
        assert client.post("/api/remote/favourite-score", json=payload).status_code == 400
        assert library_edits.queue.pending() == []

    def test_set_while_desktop_offline_then_applied(self, client):
        client.post("/api/media/register", json={
            "favouriteArtists": ["Mira"], "favouriteArtistScore": {"score": 5, "default": 5},
        })
        r = client.post("/api/remote/favourite-artist", json={"artist": " Kai ", "favourite": True})
        assert r.json() == {"ok": True, "seq": 1, "delivered": False, "artist": "Kai", "favourite": True}
        r = client.post("/api/remote/favourite-score", json={"score": 12})
        assert r.json() == {"ok": True, "seq": 2, "delivered": False, "score": 10.0}

        with client.websocket_connect("/api/remote/sync/observe") as ws:
            msgs = [ws.receive_json() for _ in range(2)]
            assert [m["edit"]["kind"] for m in msgs] == ["favourite-artist", "favourite-score"]
            assert "path" not in msgs[0]["edit"] and "path" not in msgs[1]["edit"]
            for m in msgs:
                _ack(ws, m)
            assert _wait_until(lambda: library_edits.queue.pending() == [])
        # Folded into the registered state until the desktop registers again.
        data = client.get("/api/remote/artists").json()
        assert [(a["name"], a["favourite"], a["pending"]) for a in data["artists"]] == [
            ("Kai", True, False), ("Mira", True, False),
        ]
        assert (data["score"], data["scorePending"]) == (10.0, False)

    def test_edits_survive_a_backend_restart(self, tmp_path):
        q = LibraryEditQueue(persist_dir=str(tmp_path))
        q.enqueue(library_edits.make_favourite_artist_edit("Mira", True))
        q.enqueue(library_edits.make_favourite_score_edit(3))
        again = LibraryEditQueue(persist_dir=str(tmp_path))
        assert [e["kind"] for e in again.pending()] == ["favourite-artist", "favourite-score"]


# --- Play queue ---------------------------------------------------------------------

def _queue(manual=(), auto=(), auto_fill=True):
    return {"manual": list(manual), "auto": list(auto), "autoFill": auto_fill}


def _add(path, position="end"):
    return library_edits.make_queue_edit(library_edits.KIND_QUEUE_ADD, path=path, position=position)


def _edit(kind, **fields):
    return library_edits.make_queue_edit(kind, **fields)


class TestQueueOverlay:
    def test_edit_validation(self):
        with pytest.raises(ValueError):
            library_edits.make_queue_edit("queue-shuffle", path=A)
        with pytest.raises(ValueError):
            library_edits.make_queue_edit(library_edits.KIND_QUEUE_ADD)
        with pytest.raises(ValueError):
            library_edits.make_queue_edit(library_edits.KIND_QUEUE_REMOVE, path="")
        with pytest.raises(ValueError):
            library_edits.make_queue_edit(library_edits.KIND_QUEUE_ADD, path=A, position="top")
        for enabled in (None, "yes", 1):
            with pytest.raises(ValueError):
                library_edits.make_queue_edit(library_edits.KIND_QUEUE_AUTO_FILL, enabled=enabled)
        assert _add(A, position=None) == {"kind": "queue-add", "path": A, "position": "end", "source": ""}
        assert _edit(library_edits.KIND_QUEUE_CLEAR, source="remote") == {"kind": "queue-clear", "source": "remote"}

    def test_add_end_next_and_move_to_front(self):
        base = _queue(manual=[A], auto=[B, C])
        out = library_edits.effective_queue(base, [_add(C)])
        assert out == _queue(manual=[A, C], auto=[B])
        out = library_edits.effective_queue(base, [_add(C), _add(B, "next")])
        assert out == _queue(manual=[B, A, C], auto=[])
        # `next` on a queued video moves it first; `end` leaves it where it is.
        out = library_edits.effective_queue(base, [_add(C), _add(C, "next"), _add(A)])
        assert out["manual"] == [C, A]
        assert base == _queue(manual=[A], auto=[B, C])  # base untouched

    def test_remove_and_played_leave_both_lists(self):
        base = _queue(manual=[A, B], auto=[C])
        out = library_edits.effective_queue(base, [
            _edit(library_edits.KIND_QUEUE_REMOVE, path=A),
            _edit(library_edits.KIND_QUEUE_PLAYED, path=C),
            _edit(library_edits.KIND_QUEUE_REMOVE, path="/gone.mp4"),
        ])
        assert out == _queue(manual=[B], auto=[])

    def test_clear_empties_manual_only(self):
        out = library_edits.effective_queue(_queue(manual=[A, B], auto=[C]), [_edit(library_edits.KIND_QUEUE_CLEAR)])
        assert out == _queue(manual=[], auto=[C])

    def test_auto_fill_off_empties_auto(self):
        base = _queue(manual=[A], auto=[B, C])
        off = library_edits.effective_queue(base, [_edit(library_edits.KIND_QUEUE_AUTO_FILL, enabled=False)])
        assert off == _queue(manual=[A], auto=[], auto_fill=False)
        on = library_edits.effective_queue(_queue(auto_fill=False), [_edit(library_edits.KIND_QUEUE_AUTO_FILL, enabled=True)])
        assert on["autoFill"] is True

    def test_other_kinds_are_ignored(self):
        base = _queue(manual=[A])
        assert library_edits.effective_queue(base, [{"kind": KIND_RATING, "path": B, "rating": 3.0}]) == base


class TestQueueRegistration:
    def test_registration_is_cleaned(self, client):
        client.post("/api/media/register", json={"queue": {
            "manual": [A, A, 3, "", None, B], "auto": [B, C, C, {"x": 1}], "autoFill": "no",
        }})
        assert media.get_queue() == _queue(manual=[A, B], auto=[C], auto_fill=True)
        client.post("/api/media/register", json={"queue": {"manual": "nope", "autoFill": False}})
        assert media.get_queue() == _queue(auto_fill=False)
        client.post("/api/media/register", json={"queue": None})
        assert media.get_queue() == _queue()

    def test_omitted_key_leaves_queue_alone(self, client):
        media.register_queue(_queue(manual=[A]))
        client.post("/api/media/register", json={"ratings": {}})
        assert media.get_queue()["manual"] == [A]

    def test_ack_keeps_the_desktops_registration(self, client, library):
        # The desktop registers its answer to an edit before acking it; a
        # skipped auto-filled video moves to the end of the auto-fill there,
        # and the ack must not drop it again.
        media.register_queue(_queue(manual=[C], auto=[A, B]))
        client.post("/api/remote/queue/remove", json={"videoId": library["A"]})
        with client.websocket_connect("/api/remote/sync/observe") as ws:
            msg = ws.receive_json()
            assert msg["edit"]["kind"] == "queue-remove" and msg["edit"]["path"] == A
            client.post("/api/media/register", json={"queue": {"manual": [C], "auto": [B, A], "autoFill": True}})
            _ack(ws, msg)
            assert _wait_until(lambda: library_edits.queue.pending() == [])
        assert media.get_queue() == _queue(manual=[C], auto=[B, A])


class TestRemoteQueue:
    def test_get_queue_lists_manual_then_auto(self, client, library):
        media.register_queue(_queue(manual=[C, "/gone.mp4"], auto=[A, B], auto_fill=True))
        assert client.get("/api/remote/queue").json() == {
            "items": [
                {"videoId": library["C"], "auto": False},
                {"videoId": library["A"], "auto": True},
                {"videoId": library["B"], "auto": True},
            ],
            "autoFill": True,
        }

    def test_add_is_visible_immediately(self, client, library):
        media.register_queue(_queue(manual=[C], auto=[A, B]))
        r = client.post("/api/remote/queue/add", json={"videoId": library["B"], "position": "next"})
        assert r.status_code == 200
        body = r.json()
        assert (body["ok"], body["seq"], body["delivered"]) == (True, 1, False)
        expected = [(library["B"], False), (library["C"], False), (library["A"], True)]
        assert [(i["videoId"], i["auto"]) for i in body["queue"]["items"]] == expected
        got = client.get("/api/remote/queue").json()
        assert [(i["videoId"], i["auto"]) for i in got["items"]] == expected
        [edit] = library_edits.queue.pending()
        assert (edit["kind"], edit["path"], edit["position"], edit["source"]) == ("queue-add", B, "next", "remote")

        r = client.post("/api/remote/queue/add", json={"videoId": library["A"]})
        assert [i["videoId"] for i in r.json()["queue"]["items"]] == [library["B"], library["C"], library["A"]]
        assert library_edits.queue.pending()[-1]["position"] == "end"

    def test_remove_played_clear_and_auto_fill(self, client, library):
        media.register_queue(_queue(manual=[A, B], auto=[C]))
        r = client.post("/api/remote/queue/remove", json={"videoId": library["A"]})
        assert [i["videoId"] for i in r.json()["queue"]["items"]] == [library["B"], library["C"]]
        r = client.post("/api/remote/queue/played", json={"videoId": library["C"]})
        assert [i["videoId"] for i in r.json()["queue"]["items"]] == [library["B"]]
        r = client.post("/api/remote/queue/clear", json={})
        assert r.json()["queue"] == {"items": [], "autoFill": True}
        r = client.post("/api/remote/queue/auto-fill", json={"enabled": False})
        assert r.json()["queue"]["autoFill"] is False
        assert [e["kind"] for e in library_edits.queue.pending()] == [
            "queue-remove", "queue-played", "queue-clear", "queue-auto-fill",
        ]
        assert library_edits.queue.pending()[-1]["enabled"] is False

    @pytest.mark.parametrize("url", ["/api/remote/queue/add", "/api/remote/queue/remove", "/api/remote/queue/played"])
    def test_unknown_video_is_404(self, client, library, url):
        assert client.post(url, json={"videoId": "nope"}).status_code == 404
        assert client.post(url, json={}).status_code == 404
        assert library_edits.queue.pending() == []

    @pytest.mark.parametrize("url, payload", [
        ("/api/remote/queue/add", {"position": "top"}),
        ("/api/remote/queue/add", {"position": 1}),
        ("/api/remote/queue/auto-fill", {"enabled": "yes"}),
        ("/api/remote/queue/auto-fill", {}),
    ])
    def test_bad_body_is_400(self, client, library, url, payload):
        assert client.post(url, json={"videoId": library["A"], **payload}).status_code == 400
        assert library_edits.queue.pending() == []


class TestHereSphereQueue:
    TAGGED = {"tags": [{"name": "Studio:Lib"}, {"name": " queue:up NEXT "}]}
    UNTAGGED = {"tags": [{"name": "Studio:Lib"}]}

    @pytest.mark.parametrize("body, served, expected", [
        (TAGGED, False, [("queue-add", "end")]),
        (UNTAGGED, True, [("queue-remove", None)]),
        (TAGGED, True, []),
        (UNTAGGED, False, []),
        (TAGGED, None, []),      # never served: no record, nothing
        (UNTAGGED, None, []),
        ({"rating": 0}, True, []),  # no tag list
    ])
    def test_tag_counts_only_relative_to_served_state(self, body, served, expected):
        edits = parse_heresphere_update(body, A, {}, [], served)
        assert [(e["kind"], e.get("position")) for e in edits] == expected
        assert all(e["source"] == "heresphere" and e["path"] == A for e in edits)

    def test_scene_carries_queue_tag_and_event_server(self, client, library):
        media.register_queue(_queue(manual=[A], auto=[C]))
        for vid in (library["A"], library["C"]):
            scene = client.get(f"/heresphere/{vid}").json()
            assert "Queue:Up next" in [t["name"] for t in scene["tags"]]
            assert scene["eventServer"] == f"http://testserver/heresphere/events/{vid}"
        assert "Queue:Up next" not in [t["name"] for t in client.get(f"/heresphere/{library['B']}").json()["tags"]]
        cats = [c["tag"]["name"] for c in client.get(f"/deovr/{library['C']}").json()["categories"]]
        assert "Queue:Up next" in cats

    def test_adding_and_removing_the_tag(self, client, library):
        b = library["B"]
        client.get(f"/heresphere/{b}")  # served without the tag
        scene = client.post(f"/heresphere/{b}", json=self.TAGGED).json()
        assert "Queue:Up next" in [t["name"] for t in scene["tags"]]
        [edit] = library_edits.queue.pending()
        assert (edit["kind"], edit["path"], edit["position"]) == ("queue-add", B, "end")

        scene = client.post(f"/heresphere/{b}", json=self.UNTAGGED).json()
        assert "Queue:Up next" not in [t["name"] for t in scene["tags"]]
        assert [e["kind"] for e in library_edits.queue.pending()] == ["queue-add", "queue-remove"]

    def test_no_served_record_changes_nothing(self, client, library):
        client.post(f"/heresphere/{library['B']}", json=self.TAGGED)
        assert library_edits.queue.pending() == []

    def test_rating_after_consumed_while_open_does_not_requeue(self, client, library):
        b = library["B"]  # in no playlist, so the tag list below changes nothing else
        media.register_queue(_queue(manual=[B]))
        client.get(f"/heresphere/{b}")  # served with the tag
        client.post(f"/heresphere/events/{b}", json={"event": 1, "id": b})
        # HereSphere still holds the old tag list when the user rates.
        client.post(f"/heresphere/{b}", json={"rating": 4.0, **self.TAGGED})
        assert [e["kind"] for e in library_edits.queue.pending()] == ["queue-played", "rating"]

    def test_play_event_consumes_once(self, client, library):
        a, b = library["A"], library["B"]
        media.register_queue(_queue(manual=[A, B]))
        r = client.post(f"/heresphere/events/{a}", json={"username": "", "id": a, "event": 1, "time": 0})
        assert r.status_code == 200
        client.post(f"/heresphere/events/{a}", json={"event": 2, "time": 5000})  # pause
        client.post(f"/heresphere/events/{a}", json={"event": 1, "time": 5000})  # play again
        [edit] = library_edits.queue.pending()
        assert (edit["kind"], edit["path"], edit["source"]) == ("queue-played", A, "heresphere")
        assert [i["videoId"] for i in client.get("/api/remote/queue").json()["items"]] == [b]

        client.post(f"/heresphere/events/{b}", json={"event": 1})
        client.post(f"/heresphere/events/{a}", json={"event": 1})  # back to A: a new play
        assert [e["path"] for e in library_edits.queue.pending()] == [A, B, A]

    @pytest.mark.parametrize("event", [0, 2, 3, True, "1", None])
    def test_other_events_are_ignored(self, client, library, event):
        r = client.post(f"/heresphere/events/{library['A']}", json={"event": event})
        assert r.status_code == 200
        assert library_edits.queue.pending() == []

    def test_bad_body_and_unknown_video_answer_200(self, client, library):
        assert client.post(f"/heresphere/events/{library['A']}", content=b"not json").status_code == 200
        assert client.post(f"/heresphere/events/{library['A']}", json=[1]).status_code == 200
        assert client.post("/heresphere/events/nope", json={"event": 1}).status_code == 200
        assert library_edits.queue.pending() == []

    def test_queue_section_first_in_queue_order(self, client, library):
        media.register_queue(_queue(manual=[C], auto=[A, "/gone.mp4"]))
        sections, names = _sections(client)
        assert names[0] == "Queue"
        assert _ids(sections["Queue"]) == [library["C"], library["A"]]
        deo, deo_names = _sections(client, "/deovr")
        assert deo_names[0] == "Queue"
        assert [i["title"] for i in deo["Queue"]] == ["C", "A_180_sbs"]
        vr, _ = _sections(client, "/heresphere?filter=vr")
        assert _ids(vr["Queue"]) == [library["A"]]

    def test_queue_section_reflects_pending_edits_and_is_omitted_when_empty(self, client, library):
        assert "Queue" not in _sections(client)[1]
        client.post("/api/remote/queue/add", json={"videoId": library["B"]})
        sections, names = _sections(client)
        assert names[0] == "Queue" and _ids(sections["Queue"]) == [library["B"]]
