"""Access PIN: the gate for other devices, login page, tokens, rate limit,
and the HereSphere / DeoVR logins (services/access_pin.py, routes/access.py)."""

import json
import os
import sys
import time
from urllib.parse import parse_qs, urlsplit

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from fastapi.testclient import TestClient
from starlette.websockets import WebSocketDisconnect

from main import app
from routes.access import safe_next
from routes.media import register_videos, _path_to_id
from services import access_pin

PIN = "4711"
SECRET_HEX = "ab" * 32
SECRET = bytes.fromhex(SECRET_HEX)
LAN = ("192.168.2.50", 1234)


def write_pin(user_data, pin=PIN, secret=SECRET_HEX):
    path = user_data / access_pin.PIN_FILE_NAME
    path.write_text(json.dumps({"version": 1, "pin": pin, "secret": secret, "changedAt": "2026-11-16T10:00:00Z"}))
    # A rewrite within the same mtime tick must still be picked up.
    st = path.stat()
    os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns + 1_000_000_000))
    return path


@pytest.fixture(autouse=True)
def user_data(tmp_path, monkeypatch):
    access_pin.set_user_data_dir(str(tmp_path))
    monkeypatch.setattr(access_pin, "limiter", access_pin.RateLimiter())
    yield tmp_path
    access_pin.set_user_data_dir(None)


@pytest.fixture
def pin(user_data):
    return write_pin(user_data)


@pytest.fixture
def lan():
    return TestClient(app, client=LAN, follow_redirects=False)


@pytest.fixture
def local():
    return TestClient(app, client=("127.0.0.1", 50000), follow_redirects=False)


@pytest.fixture
def video(tmp_path):
    path = tmp_path / "Scene_180_sbs.mp4"
    path.write_bytes(b"\0" * 2048)
    script = tmp_path / "Scene_180_sbs.funscript"
    script.write_text('{"actions": [{"at": 0, "pos": 0}, {"at": 1000, "pos": 100}]}')
    register_videos([{"path": str(path), "name": path.name, "funscriptPath": str(script)}])
    return _path_to_id(str(path))


def token(expiry=None):
    return access_pin.make_token(SECRET, expiry=expiry)


def cookie(value):
    return {"Cookie": f"{access_pin.COOKIE_NAME}={value}"}


# --- Tokens -----------------------------------------------------------------

def test_token_matches_the_shared_test_vector():
    # Same vector as the Rust server, so a login on one port unlocks the other.
    assert access_pin.make_token(bytes(range(32)), expiry=1800000000) == (
        "v1.1800000000.866fbcd4be953182d7f832e844bf390a5f79d112cccd131af311acbb7431e77a")


def test_token_lasts_30_days():
    now = 1_700_000_000
    t = access_pin.make_token(SECRET, now=now)
    assert t.split(".")[1] == str(now + 30 * 24 * 3600)
    assert access_pin.verify_token(SECRET, t, now=now + 30 * 24 * 3600 - 1)
    assert not access_pin.verify_token(SECRET, t, now=now + 30 * 24 * 3600)


@pytest.mark.parametrize("bad", [
    "", "v1", "v2.9999999999.00", "v1.abc." + "0" * 64, "v1.9999999999." + "0" * 64,
    "v1.9999999999." + "A" * 64, " " + "x",
])
def test_garbage_and_wrong_macs_are_rejected(bad):
    assert not access_pin.verify_token(SECRET, bad)


def test_tampered_expiry_or_other_secret_is_rejected():
    t = token(expiry=int(time.time()) + 3600)
    _, exp, mac = t.split(".")
    assert not access_pin.verify_token(SECRET, f"v1.{int(exp) + 1}.{mac}")
    assert not access_pin.verify_token(b"\1" * 32, t)


def test_media_token_is_stable_for_a_day_and_valid_29_to_30_days():
    day = 86400
    a = access_pin.make_media_token(SECRET, now=100 * day + 10)
    b = access_pin.make_media_token(SECRET, now=100 * day + 80000)
    assert a == b
    expiry = int(a.split(".")[1])
    assert 100 * day + 29 * day <= expiry <= 100 * day + 30 * day + 10


def test_with_key_appends_to_either_kind_of_url():
    assert access_pin.with_key("http://h/a", "T") == "http://h/a?k=T"
    assert access_pin.with_key("http://h/a?v=1", "T") == "http://h/a?v=1&k=T"
    assert access_pin.with_key("http://h/a", None) == "http://h/a"


# --- PIN file ---------------------------------------------------------------

def test_no_file_means_no_pin(user_data):
    assert access_pin.load() is None


def test_file_is_reloaded_when_it_changes(user_data):
    write_pin(user_data, pin="1234")
    assert access_pin.load().pin == "1234"
    write_pin(user_data, pin="98765", secret="cd" * 32)
    assert access_pin.load() == access_pin.AccessPin("98765", bytes.fromhex("cd" * 32))
    (user_data / access_pin.PIN_FILE_NAME).unlink()
    assert access_pin.load() is None


@pytest.mark.parametrize("content", [
    "not json",
    json.dumps({"version": 2, "pin": "1234", "secret": SECRET_HEX}),
    json.dumps({"version": 1, "pin": "12", "secret": SECRET_HEX}),
    json.dumps({"version": 1, "pin": "12a4", "secret": SECRET_HEX}),
    json.dumps({"version": 1, "pin": "1234", "secret": "AB" * 32}),
    json.dumps({"version": 1, "pin": "1234", "secret": "ab"}),
])
def test_invalid_file_fails_closed(user_data, lan, local, content):
    (user_data / access_pin.PIN_FILE_NAME).write_text(content)
    with pytest.raises(access_pin.PinFileError):
        access_pin.load()
    res = lan.get("/api/remote/sources")
    assert res.status_code == 503
    assert res.json()["error"] == "access pin file unreadable"
    page = lan.get("/remote/")
    assert page.status_code == 503 and "unreadable" in page.text
    assert lan.get("/login").status_code == 503
    assert lan.post("/heresphere").json()["access"] == -1
    assert local.get("/api/remote/sources").status_code == 200


# --- Gate -------------------------------------------------------------------

def test_without_a_pin_everything_is_open(lan):
    assert lan.get("/api/remote/sources").status_code == 200
    assert lan.get("/remote/").status_code == 200
    assert lan.post("/heresphere").json()["access"] == 1
    # No PIN: the login page has nothing to ask and sends the device on.
    res = lan.get("/login?next=/remote/")
    assert res.status_code == 303 and res.headers["location"] == "/remote/"


def test_loopback_is_never_asked(pin):
    for host in ("127.0.0.1", "::1", "::ffff:127.0.0.1"):
        c = TestClient(app, client=(host, 1), follow_redirects=False)
        assert c.get("/api/remote/sources").status_code == 200
        assert c.get("/remote/").status_code == 200
        body = c.post("/heresphere").json()
        assert body["access"] == 1


@pytest.mark.parametrize("path", ["/remote/", "/remote", "/remote/app.js", "/remote/manifest.json", "/"])
def test_pages_redirect_to_login(pin, lan, path):
    res = lan.get(path)
    assert res.status_code == 303
    loc = urlsplit(res.headers["location"])
    assert loc.path == "/login"
    assert parse_qs(loc.query)["next"] == [path]


def test_html_requests_redirect_keeping_the_query(pin, lan):
    res = lan.get("/api/remote/videos?x=1", headers={"Accept": "text/html,*/*"})
    assert res.status_code == 303
    assert parse_qs(urlsplit(res.headers["location"]).query)["next"] == ["/api/remote/videos?x=1"]


@pytest.mark.parametrize("method,path", [
    ("GET", "/api/remote/sources"), ("POST", "/api/remote/rating"), ("GET", "/api/status"),
    ("GET", "/locales/en.json"), ("GET", "/health"), ("POST", "/heresphere/events/x"),
])
def test_api_needs_the_pin(pin, lan, method, path):
    res = lan.request(method, path)
    assert res.status_code == 401
    assert res.json() == {"error": "pin required"}


def test_cookie_or_k_token_opens_the_api(pin, lan):
    t = token()
    assert lan.get("/api/remote/sources", headers=cookie(t)).status_code == 200
    assert lan.get(f"/api/remote/sources?k={t}").status_code == 200
    assert lan.get("/remote/", headers=cookie(t)).status_code == 200


def test_expired_tampered_or_old_secret_tokens_are_refused(pin, user_data, lan):
    expired = token(expiry=int(time.time()) - 1)
    good = token()
    tampered = good[:-1] + ("0" if good[-1] != "0" else "1")
    for t in (expired, tampered, "garbage"):
        assert lan.get("/api/remote/sources", headers=cookie(t)).status_code == 401
        assert lan.get(f"/api/remote/sources?k={t}").status_code == 401
    # Changing the PIN regenerates the secret: every device is logged out.
    write_pin(user_data, pin="2468", secret="ef" * 32)
    assert lan.get("/api/remote/sources", headers=cookie(good)).status_code == 401


def test_media_accepts_cookie_or_k(pin, lan, video):
    t = token()
    assert lan.get(f"/api/media/stream/{video}").status_code == 401
    assert lan.get(f"/api/media/stream/{video}?k={t}").status_code in (200, 206)
    assert lan.get(f"/api/media/stream/{video}", headers=cookie(t)).status_code in (200, 206)
    assert lan.get(f"/api/media/script/{video}?k={t}").status_code == 200


def test_websockets_are_closed_without_a_login(pin, lan):
    for path in ("/api/remote/sync?videoId=x", "/api/remote/tcode-relay?role=sink", "/api/remote/sync/observe"):
        with lan.websocket_connect(path) as ws:
            with pytest.raises(WebSocketDisconnect) as exc:
                ws.receive_text()
            assert exc.value.code == 4401


def test_websocket_with_cookie_gets_through(pin, lan):
    with lan.websocket_connect("/api/remote/sync?videoId=x", headers=cookie(token())) as ws:
        ws.send_json({"type": "hello", "videoId": "x", "duration": 0})


# --- Login page -------------------------------------------------------------

def test_login_page_is_a_numeric_form(pin, lan):
    res = lan.get("/login?next=/remote/")
    assert res.status_code == 200
    assert "<title>FunSync</title>" in res.text
    assert 'inputmode="numeric"' in res.text
    assert 'name="next" value="/remote/"' in res.text
    assert "http" not in res.text.split("<body>", 1)[1]  # no external assets


def test_right_pin_sets_the_cookie_and_goes_on(pin, lan):
    res = lan.post("/login", data={"pin": PIN, "next": "/remote/"})
    assert res.status_code == 303
    assert res.headers["location"] == "/remote/"
    set_cookie = res.headers["set-cookie"]
    value = set_cookie.split(";", 1)[0].split("=", 1)[1]
    assert set_cookie == f"funsync_access={value}; Path=/; Max-Age=2592000; HttpOnly; SameSite=Lax"
    assert access_pin.verify_token(SECRET, value)
    assert lan.get("/api/remote/sources", headers=cookie(value)).status_code == 200


def test_wrong_pin_shows_the_form_again(pin, lan):
    res = lan.post("/login", data={"pin": "0000", "next": "/remote/"})
    assert res.status_code == 401
    assert "Wrong PIN" in res.text
    assert "set-cookie" not in res.headers


@pytest.mark.parametrize("raw,expected", [
    ("/remote/", "/remote/"),
    ("/remote/?a=b", "/remote/?a=b"),
    ("https://evil.example/", "/"),
    ("//evil.example/", "/"),
    ("/\\evil.example", "/"),
    ("remote/", "/"),
    ("/re mote", "/"),
    ("/r\r\nSet-Cookie:x", "/"),
    ("/login", "/"),
    ("/login?next=/x", "/"),
    ("", "/"),
    (None, "/"),
])
def test_next_is_sanitised(raw, expected):
    assert safe_next(raw) == expected


def test_login_redirect_is_sanitised(pin, lan):
    res = lan.post("/login", data={"pin": PIN, "next": "//evil.example/"})
    assert res.headers["location"] == "/"


def test_five_wrong_pins_lock_the_ip_out_for_ten_minutes(pin, lan, monkeypatch):
    clock = [1000.0]
    monkeypatch.setattr(access_pin, "limiter", access_pin.RateLimiter(clock=lambda: clock[0]))
    for _ in range(5):
        assert lan.post("/login", data={"pin": "0000"}).status_code == 401
    res = lan.post("/login", data={"pin": PIN})
    assert res.status_code == 429
    assert int(res.headers["retry-after"]) > 0
    # Another device isn't affected.
    other = TestClient(app, client=("192.168.2.51", 1), follow_redirects=False)
    assert other.post("/login", data={"pin": PIN}).status_code == 303
    clock[0] += 599
    assert lan.post("/login", data={"pin": PIN}).status_code == 429
    clock[0] += 1
    assert lan.post("/login", data={"pin": PIN}).status_code == 303


def test_logout_clears_the_cookie(pin, lan):
    res = lan.post("/logout")
    assert res.status_code == 303
    assert res.headers["set-cookie"] == "funsync_access=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"


# --- HereSphere -------------------------------------------------------------

def _hs_login(body, name, access):
    assert body == {"access": access, "library": [{"name": name, "list": []}]}


def test_heresphere_asks_for_a_login(pin, lan, video):
    for path in ("/heresphere", f"/heresphere/{video}", "/heresphere/unknown"):
        res = lan.post(path, json={})
        assert res.status_code == 200
        assert res.headers["heresphere-json-version"] == "1"
        _hs_login(res.json(), "Login Required", 0)
    _hs_login(lan.get("/heresphere").json(), "Login Required", 0)


def test_heresphere_wrong_password_fails(pin, lan, video):
    res = lan.post("/heresphere", json={"username": "x", "password": "0000"})
    _hs_login(res.json(), "Login Failed", -1)


def test_heresphere_password_opens_the_library_with_keyed_urls(pin, lan, video):
    creds = {"username": "anyone", "password": PIN}
    lib = lan.post("/heresphere", json=creds).json()
    assert lib["access"] == 1
    urls = [u for section in lib["library"] for u in section["list"]]
    assert urls and all("k=" not in u for u in urls)  # scene URLs stay bare

    scene = lan.post(f"/heresphere/{video}", json=creds).json()
    assert scene["access"] == 1
    keyed = [scene["media"][0]["sources"][0]["url"], scene["scripts"][0]["url"], scene["eventServer"]]
    for url in keyed:
        k = parse_qs(urlsplit(url).query)["k"][0]
        assert access_pin.verify_token(SECRET, k)
        parts = urlsplit(url)
        res = lan.request("POST" if url == scene["eventServer"] else "GET", f"{parts.path}?{parts.query}")
        assert res.status_code != 401
        if url == keyed[0]:
            assert res.status_code in (200, 206)
    # The thumbnail carries the thumbnail-only key, and it gets past the gate (the fake
    # video has no real thumbnail to make: 503).
    thumb = urlsplit(scene["thumbnailImage"])
    assert parse_qs(thumb.query)["k"] == [access_pin.make_thumb_token(SECRET)]
    assert lan.get(f"{thumb.path}?{thumb.query}").status_code != 401


def test_headset_thumbnail_urls_stay_the_same_until_the_pin_changes(pin, user_data, lan, video, monkeypatch):
    creds = {"username": "anyone", "password": PIN}
    scene_today = lan.post(f"/heresphere/{video}", json=creds).json()
    today = scene_today["thumbnailImage"]
    deovr_creds = {"login": "anyone", "password": PIN}
    deovr_today = lan.post(f"/deovr/{video}", data=deovr_creds).json()["thumbnailUrl"]
    real_time = time.time
    monkeypatch.setattr(time, "time", lambda: real_time() + 5 * 86400)
    later = lan.post(f"/heresphere/{video}", json=creds).json()
    assert later["thumbnailImage"] == today, "the headset would download every thumbnail again"
    assert lan.post(f"/deovr/{video}", data=deovr_creds).json()["thumbnailUrl"] == deovr_today
    # Videos keep the expiring key.
    assert later["media"][0]["sources"][0]["url"] != scene_today["media"][0]["sources"][0]["url"]
    monkeypatch.setattr(time, "time", real_time)
    write_pin(user_data, pin="2468", secret="ef" * 32)
    thumb = urlsplit(today)
    assert lan.get(f"{thumb.path}?{thumb.query}").status_code == 401


def test_thumbnail_key_opens_thumbnails_only(pin, lan, video):
    k = access_pin.make_thumb_token(SECRET)
    assert lan.get(f"/api/media/thumb/{video}?v=1&k={k}").status_code != 401
    for method, path in [("GET", f"/api/media/stream/{video}"), ("GET", f"/api/media/script/{video}"),
                         ("GET", "/api/remote/sources"), ("POST", f"/api/media/thumb/{video}")]:
        assert lan.request(method, f"{path}?k={k}").status_code == 401, f"{method} {path}"
    assert lan.get("/remote/", headers=cookie(k)).status_code == 303
    # It doesn't log a headset in either.
    assert lan.post(f"/heresphere/{video}?k={k}", json={}).json()["access"] == 0


def test_heresphere_cookie_or_k_is_enough(pin, lan, video):
    t = token()
    assert lan.post("/heresphere", json={}, headers=cookie(t)).json()["access"] == 1
    assert lan.post(f"/heresphere/{video}?k={t}", json={}).json()["access"] == 1


def test_heresphere_wrong_passwords_are_rate_limited(pin, lan):
    for _ in range(5):
        assert lan.post("/heresphere", json={"username": "u", "password": "0000"}).json()["access"] == -1
    _hs_login(lan.post("/heresphere", json={"username": "u", "password": PIN}).json(), "Login Failed", -1)
    assert lan.post("/login", data={"pin": PIN}).status_code == 429


def test_heresphere_without_pin_has_bare_urls(lan, video):
    scene = lan.post(f"/heresphere/{video}", json={}).json()
    assert scene["access"] == 1
    assert "k=" not in scene["media"][0]["sources"][0]["url"]


# --- DeoVR ------------------------------------------------------------------

def test_deovr_asks_for_a_login(pin, lan, video):
    for path in ("/deovr", f"/deovr/{video}"):
        res = lan.post(path)
        assert res.status_code == 200
        assert res.json() == {"scenes": [{"name": "Login Required", "list": []}], "authorized": "0"}
        assert res.headers["content-length"] == str(len(res.content))


def test_deovr_wrong_password_fails(pin, lan):
    res = lan.post("/deovr", data={"login": "x", "password": "0000"})
    assert res.json() == {"scenes": [{"name": "Login Failed", "list": []}], "authorized": "-1"}


def test_deovr_password_opens_the_library_with_keyed_urls(pin, lan, video):
    creds = {"login": "anyone", "password": PIN}
    lib = lan.post("/deovr", data=creds).json()
    assert lib["authorized"] == "1"
    items = [item for section in lib["scenes"] for item in section["list"]]
    assert items
    thumb_key = [access_pin.make_thumb_token(SECRET)]
    for item in items:
        assert parse_qs(urlsplit(item["thumbnailUrl"]).query)["k"] == thumb_key
        assert "k=" not in item["video_url"]

    scene = lan.post(f"/deovr/{video}", data=creds).json()
    stream = scene["encodings"][0]["videoSources"][0]["url"]
    script = scene["fleshlight"][0]["url"]
    for url in (stream, script):
        assert access_pin.verify_token(SECRET, parse_qs(urlsplit(url).query)["k"][0])
    assert parse_qs(urlsplit(scene["thumbnailUrl"]).query)["k"] == thumb_key
    parts = urlsplit(stream)
    assert lan.get(f"{parts.path}?{parts.query}").status_code in (200, 206)


def test_deovr_without_pin_is_unchanged(lan, video):
    lib = lan.post("/deovr").json()
    assert lib["authorized"] == "0"
    assert lib["scenes"] and lib["scenes"][0]["name"] != "Login Required"
