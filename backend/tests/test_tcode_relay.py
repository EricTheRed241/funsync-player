"""TCode browser serial relay: desktop app → backend → web remote tab.

Same TestClient websocket pattern as test_remote_sync.py. The client address
is switched per connection (TestClient has one per client) so the desktop
can connect from loopback and the sink from the LAN on the same event loop.
"""

import json
import time

import pytest
from fastapi.testclient import TestClient
from starlette.websockets import WebSocketDisconnect

from main import app
from routes import remote_sync, status, tcode_relay

URL = "/api/remote/tcode-relay"
PAGE_ORIGIN = "http://testserver"  # TestClient sends `Host: testserver`
LAN_IP = "192.168.2.40"
TCODE = "L05000I16 R05000I16\n"
USB = {"usbVendorId": 0x1A86, "usbProductId": 0x7523}


@pytest.fixture(autouse=True)
def reset_relay_state():
    tcode_relay._reset_state_for_tests()
    remote_sync._reset_state_for_tests()
    yield
    tcode_relay._reset_state_for_tests()
    remote_sync._reset_state_for_tests()


@pytest.fixture
def client():
    with TestClient(app, client=("127.0.0.1", 50000)) as c:
        yield c


def _connect(client, role, ip="127.0.0.1", origin=None):
    client._transport.client = (ip, 50000)
    headers = {"origin": origin} if origin else {}
    return client.websocket_connect(f"{URL}?role={role}", headers=headers)


def _desktop(client):
    return _connect(client, "desktop")


def _sink(client, ip=LAN_IP, origin=PAGE_ORIGIN):
    return _connect(client, "sink", ip=ip, origin=origin)


def _refused(client, role, **kw):
    with pytest.raises(WebSocketDisconnect):
        with _connect(client, role, **kw):
            pass


def _info(client):
    client._transport.client = ("127.0.0.1", 50000)
    return client.get(URL).json()


def _wait_info(client, pred, timeout=2.0):
    end = time.time() + timeout
    while time.time() < end:
        body = _info(client)
        if pred(body):
            return body
        time.sleep(0.02)
    raise AssertionError(f"relay info never matched: {body}")


def _open_port(sink, desktop, open_=True):
    sink.send_json({"type": "port", "open": open_, "info": USB, "baudRate": 115200})
    msg = desktop.receive_json()
    assert msg["type"] == "sink" and msg["portOpen"] is open_
    return msg


# --- Role checks -----------------------------------------------------------

def test_desktop_must_be_loopback_without_origin(client):
    _refused(client, "desktop", ip=LAN_IP)
    _refused(client, "desktop", origin="http://127.0.0.1:5123")
    with _desktop(client) as desktop:
        msg = desktop.receive_json()
    assert msg == {"type": "sink", "attached": False, "ip": None, "portOpen": False,
                   "port": None, "baudRate": None, "since": None}


def test_sink_must_be_local_and_from_a_page_this_backend_served(client):
    _refused(client, "sink", ip="8.8.8.8", origin=PAGE_ORIGIN)
    _refused(client, "sink", ip="100.64.1.2", origin=PAGE_ORIGIN)
    _refused(client, "sink", ip=LAN_IP, origin="http://evil.example")
    _refused(client, "sink", ip=LAN_IP, origin="http://testserver:8080")
    _refused(client, "sink", ip=LAN_IP)
    _refused(client, "nobody")
    for ip in (LAN_IP, "10.1.2.3", "172.20.0.5", "169.254.3.4", "127.0.0.1", "fe80::1", "fd00::5"):
        with _sink(client, ip=ip):
            assert _info(client)["sink"]["ip"] == ip


# --- Sink status and forwarding ----------------------------------------------

def test_desktop_follows_attach_port_and_detach(client):
    with _desktop(client) as desktop:
        assert desktop.receive_json()["attached"] is False
        with _sink(client) as sink:
            msg = desktop.receive_json()
            assert (msg["attached"], msg["ip"], msg["portOpen"]) == (True, LAN_IP, False)
            assert msg["since"]

            msg = _open_port(sink, desktop)
            assert (msg["port"], msg["baudRate"]) == (USB, 115200)

            desktop.send_text(TCODE)
            assert sink.receive_text() == TCODE  # verbatim

            _open_port(sink, desktop, open_=False)
        msg = desktop.receive_json()
        assert (msg["attached"], msg["ip"], msg["portOpen"]) == (False, None, False)


def test_tcode_without_an_open_port_is_dropped_not_queued(client):
    with _desktop(client) as desktop:
        desktop.receive_json()
        desktop.send_text("L00000\n")  # no sink yet
        _wait_info(client, lambda b: b["dropped"] == 1)
        with _sink(client) as sink:
            desktop.receive_json()  # attached
            desktop.send_text("L01111\n")  # port still closed
            _wait_info(client, lambda b: b["dropped"] == 2)
            _open_port(sink, desktop)
            desktop.send_text("L02222\n")
            assert sink.receive_text() == "L02222\n"  # the old lines never arrive
    body = _wait_info(client, lambda b: b["forwarded"] == 1)
    assert body["dropped"] == 2


def test_config_is_cached_for_a_later_sink_and_passed_on(client):
    with _desktop(client) as desktop:
        desktop.receive_json()
        desktop.send_text(json.dumps({"type": "config", "baudRate": 250000}))
        desktop.send_text(json.dumps({"type": "config", "baudRate": "fast"}))  # ignored
        _wait_info(client, lambda b: tcode_relay.relay_state()["config"] is not None)
        with _sink(client) as sink:
            assert sink.receive_json() == {"type": "config", "baudRate": 250000}
            desktop.receive_json()  # attached
            desktop.send_text(json.dumps({"type": "config", "baudRate": 115200}))
            assert sink.receive_json() == {"type": "config", "baudRate": 115200}
    assert _info(client)["dropped"] == 0  # control frames aren't TCode


def test_device_replies_are_truncated_and_rate_capped(client):
    with _desktop(client) as desktop:
        desktop.receive_json()
        with _sink(client) as sink:
            desktop.receive_json()
            sink.send_json({"type": "reply", "text": "x" * 300})
            for i in range(29):
                sink.send_json({"type": "reply", "text": f"line {i}"})
            sink.send_json({"type": "port", "open": True, "info": None, "baudRate": None})
            replies = []
            while (msg := desktop.receive_json())["type"] == "reply":
                replies.append(msg["text"])
    assert len(replies) == tcode_relay.REPLY_MAX_PER_S
    assert replies[0] == "x" * 256
    assert replies[1] == "line 0"


# --- Last wins ----------------------------------------------------------------

def test_a_newer_desktop_replaces_the_older(client):
    with _desktop(client) as first:
        first.receive_json()
        with _desktop(client) as second:
            with pytest.raises(WebSocketDisconnect) as closed:
                first.receive_json()
            assert closed.value.code == 4001
            assert second.receive_json()["type"] == "sink"
            with _sink(client):
                assert second.receive_json()["attached"] is True
            assert _info(client)["desktop"] is True


def test_a_newer_sink_kicks_the_older(client):
    with _desktop(client) as desktop:
        desktop.receive_json()
        with _sink(client, ip="192.168.2.41") as old:
            desktop.receive_json()
            _open_port(old, desktop)
            with _sink(client, ip="192.168.2.42") as new:
                assert old.receive_json() == {"type": "kicked"}
                with pytest.raises(WebSocketDisconnect) as closed:
                    old.receive_text()
                assert closed.value.code == 4000
                msg = desktop.receive_json()
                assert (msg["ip"], msg["portOpen"]) == ("192.168.2.42", False)
                _open_port(new, desktop)
                desktop.send_text(TCODE)
                assert new.receive_text() == TCODE
            assert desktop.receive_json()["attached"] is False


# --- Queue bound --------------------------------------------------------------

def test_sink_queue_keeps_the_newest_frames():
    sink = tcode_relay._Sink(ws=None, ip=LAN_IP)
    tcode_relay._enqueue(sink, json.dumps({"type": "config", "baudRate": 115200}), False)
    for i in range(100):
        tcode_relay._enqueue(sink, f"L0{i:04d}\n", True)
    assert len(sink.queue) == tcode_relay.QUEUE_MAX == 64
    assert sink.queue[0] == ("L00036\n", True)
    assert sink.queue[-1] == ("L00099\n", True)
    assert tcode_relay.relay_state()["dropped"] == 36  # the config frame isn't counted


# --- /api/status -----------------------------------------------------------------

def test_status_reports_the_relay(client):
    with _desktop(client) as desktop:
        desktop.receive_json()
        with _sink(client):
            desktop.receive_json()
            relay = client.get("/api/status").json()["tcodeRelay"]
    assert relay["desktop"] is True
    assert relay["sink"]["attached"] is True
    assert relay["sink"]["ip"] == LAN_IP


def _relay_link(waiting_for, **extra):
    return {
        "devices": {"tcode": {"connected": False, "link": {
            "transport": "remote-serial", "state": "waiting", "url": "ws://127.0.0.1:5123/api/remote/tcode-relay?role=desktop",
            "waitingFor": waiting_for, "attempts": 3, "downSince": "2026-11-12T20:00:00Z", **extra,
        }}},
    }


@pytest.mark.parametrize("waiting_for, expected", [
    ("backend", "can't reach the backend's relay"),
    ("browser", "no browser is attached"),
    ("port", f"the browser at {LAN_IP} is attached but hasn't opened the device's serial port"),
])
def test_status_explains_what_the_relay_link_waits_for(waiting_for, expected):
    desktop = {"connected": True, "status_at": time.time()}
    link = _relay_link(waiting_for, sink={"attached": True, "ip": LAN_IP, "portOpen": False})
    issues = status._issues(desktop, link, None, time.time())
    tcode_issue = next(i for i in issues if i.startswith("TCode"))
    assert expected in tcode_issue
    assert "ws://" not in tcode_issue
    assert "since 2026-11-12T20:00:00Z (3 attempts)" in tcode_issue
