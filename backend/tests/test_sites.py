"""The local sites on the backend's port (routes/sites.py): relay, rewriting, nav bar."""

import httpx
import pytest
from fastapi.testclient import TestClient

from main import app
from routes import sites

SWIPE = next(s for s in sites.SITES if s["prefix"] == "/swipe")
STATUS = next(s for s in sites.SITES if s["prefix"] == "/status")


@pytest.fixture
def upstream(monkeypatch):
    """Fake sites: records each request and answers by path."""
    seen = []

    def handle(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        path = request.url.path
        if path == "/swipe":
            return httpx.Response(200, headers={"content-type": "text/html; charset=utf-8"},
                                  content=b"<!doctype html><html><head><title>t</title></head><body></body></html>")
        if path == "/old":
            return httpx.Response(303, headers={"location": "/swipe"})
        if path == "/api/swipe":
            return httpx.Response(200, json={"echo": request.content.decode()})
        if path == "/down":
            raise httpx.ConnectError("refused")
        return httpx.Response(404, text="Not found\n")

    monkeypatch.setattr(sites, "_client", httpx.AsyncClient(transport=httpx.MockTransport(handle)))
    return seen


@pytest.fixture
def client():
    return TestClient(app, follow_redirects=False)


def test_prefix_home_is_the_sites_home_page_with_base_and_nav(client, upstream):
    r = client.get("/swipe/", headers={"accept": "text/html"})
    assert r.status_code == 200
    assert upstream[0].url.path == "/swipe"
    assert upstream[0].url.port == SWIPE["port"]
    assert '<head><base href="/swipe/"><script src="/site-nav.js" defer></script><title>' in r.text


def test_bare_prefix_redirects_to_the_slash(client, upstream):
    r = client.get("/swipe?x=1")
    assert r.status_code == 307
    assert r.headers["location"] == "/swipe/?x=1"
    assert upstream == []


def test_paths_bodies_and_queries_pass_through(client, upstream):
    r = client.post("/swipe/api/swipe?k=v", content=b'{"id":1}', headers={"content-type": "application/json"})
    assert r.status_code == 200
    assert r.json() == {"echo": '{"id":1}'}
    assert upstream[0].url.path == "/api/swipe"
    assert upstream[0].url.query == b"k=v"
    assert client.get("/swipe/nope").status_code == 404


def test_locations_come_back_under_the_prefix():
    assert sites.outside_location(SWIPE, "/swipe") == "/swipe/"
    assert sites.outside_location(SWIPE, "/login?next=%2F") == "/swipe/login?next=%2F"
    assert sites.outside_location(STATUS, "/") == "/status/"
    assert sites.outside_location(SWIPE, "https://x.test/a") == "https://x.test/a"
    assert sites.outside_location(SWIPE, "//x.test/a") == "//x.test/a"


def test_redirect_from_the_site_is_rewritten(client, upstream):
    r = client.get("/swipe/old")
    assert r.status_code == 303
    assert r.headers["location"] == "/swipe/"


def test_site_down_says_so(client, upstream):
    r = client.get("/swipe/down", headers={"accept": "text/html"})
    assert r.status_code == 502
    assert "isn't answering (not running)" in r.text
    assert '<script src="/site-nav.js"' in r.text
    r = client.get("/swipe/down")
    assert r.status_code == 502
    assert r.headers["content-type"].startswith("text/plain")


def test_inject_without_head_leaves_the_page():
    assert sites.inject(SWIPE, b"plain") == b"plain"
    assert sites.inject(SWIPE, b'<HEAD lang="x">').startswith(b'<HEAD lang="x"><base href="/swipe/">')


def test_nav_script_lists_the_remote_then_every_site(client):
    r = client.get("/site-nav.js")
    assert r.status_code == 200
    assert r.headers["content-type"].startswith("text/javascript")
    first = r.text.splitlines()[0]
    assert first.startswith('window.FUNSYNC_NAV = [{"name": "Remote", "href": "/remote/"}')
    for s in sites.SITES:
        assert f'"href": "{s["prefix"]}/"' in first
