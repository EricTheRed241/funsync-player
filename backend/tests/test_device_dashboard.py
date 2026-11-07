"""Local device dashboard: /device and /api/device/telemetry (routes/device.py,
services/device_telemetry.py)."""

import time

import pytest
from fastapi.testclient import TestClient

from main import app
from routes import remote_sync
from services.device_telemetry import LEASE_MS, LEASE_RENEW_S, Telemetry


@pytest.fixture(autouse=True)
def reset_state():
    remote_sync._reset_state_for_tests()
    yield
    remote_sync._reset_state_for_tests()


@pytest.fixture
def local():
    with TestClient(app, client=("127.0.0.1", 50000)) as c:
        yield c


@pytest.fixture
def lan():
    with TestClient(app, client=("192.168.2.238", 50000)) as c:
        yield c


def _wait(client, pred, params=None, timeout=2.0):
    end = time.time() + timeout
    while time.time() < end:
        body = client.get("/api/device/telemetry", params=params).json()
        if pred(body):
            return body
        time.sleep(0.02)
    raise AssertionError(f"telemetry never matched: {body}")


def _status(link, sync_active=False, vr=None):
    return {
        "devices": {"tcode": {"connected": link.get("state") == "connected", "syncActive": sync_active, "link": link}},
        "vr": vr or {"connected": False, "linkState": "disconnected"},
    }


CONNECTED = {"state": "connected", "url": "ws://mirabot.local:8888", "ip": "192.168.2.216"}
DROPPED = {
    "state": "reconnecting", "url": "ws://mirabot.local:8888", "attempts": 1,
    "lastDisconnect": {"at": "2026-11-07T21:26:00.369Z", "close": "close=1006 (abnormal, no close frame) clean=false"},
}


def _texts(t):
    return [e["text"] for e in t.events]


@pytest.mark.parametrize("path", ["/device", "/device/", "/device/dashboard.js", "/api/device/telemetry"])
def test_lan_clients_are_refused(lan, path):
    assert lan.get(path, follow_redirects=False).status_code == 403


def test_this_computer_gets_the_page(local):
    res = local.get("/device")
    assert res.status_code == 200
    assert "Device dashboard" in res.text
    assert local.get("/device/dashboard.js").status_code == 200
    assert local.get("/device/../main.py").status_code == 404


def test_a_poll_asks_the_desktop_to_stream_and_a_new_desktop_is_asked_again(local):
    with local.websocket_connect("/api/remote/sync/observe") as obs:
        body = local.get("/api/device/telemetry").json()
        assert obs.receive_json() == {"type": "telemetry-lease", "ms": LEASE_MS}
        assert body["streaming"] is False
        assert body["status"]["desktop"]["connected"] is True
    # A restarted renderer has no lease, however recent the last one was.
    with local.websocket_connect("/api/remote/sync/observe") as obs:
        local.get("/api/device/telemetry")
        assert obs.receive_json()["type"] == "telemetry-lease"


def test_lease_is_renewed_only_after_the_renew_interval():
    t = Telemetry()
    assert t.lease_due(100.0)
    t.lease_sent(100.0)
    assert not t.lease_due(100.0 + LEASE_RENEW_S - 1)
    assert t.lease_due(100.0 + LEASE_RENEW_S)


def test_samples_reach_the_dashboard_with_their_commands_once(local):
    sample = {
        "source": "heresphere",
        "tcode": {"link": CONNECTED, "output": {"commands": 2, "failed": 0},
                  "commands": [{"t": 1000, "cmd": "L09999I120"}, {"t": 1016, "cmd": "L00000I200 R05000I200"}]},
        "sync": {"active": True, "mediaTimeMs": 5000, "paused": False, "axes": [{"axis": "L0"}, {"axis": "R0"}]},
        "script": {"name": "scene.funscript", "actions": 700},
        "vr": {"connected": True, "linkState": "receiving", "host": "192.168.2.251", "lastPacketAgeMs": 90},
    }
    repeat = {**sample, "tcode": {**sample["tcode"], "commands": [{"t": 1016, "cmd": "L00000I200 R05000I200"},
                                                                   {"t": 1040, "cmd": "L05000I100"}]}}
    with local.websocket_connect("/api/remote/sync/observe") as obs:
        obs.send_json({"type": "device-telemetry", "sample": sample})
        obs.send_json({"type": "device-telemetry", "sample": repeat})
        body = _wait(local, lambda b: b["seq"] == 2, params={"cmdSince": 1})
        later = local.get("/api/device/telemetry", params={
            "since": body["seq"], "cmdSince": 1040, "eventSince": body["events"][-1]["seq"]}).json()
    assert body["streaming"] is True
    assert body["latest"]["source"] == "heresphere"
    assert [c["t"] for c in body["commands"]] == [1000, 1016, 1040]
    assert [p["mediaMs"] for p in body["points"]] == [5000, 5000]
    events = [e["text"] for e in body["events"]]
    assert "Sync follows: heresphere" in events
    assert "Script loaded: scene.funscript (700 actions; axes L0, R0)" in events
    assert later["points"] == [] and later["commands"] == [] and later["events"] == []


def test_timeline_tells_a_drop_and_folds_repeated_reconnect_failures():
    t = Telemetry()
    t.record_status(_status(CONNECTED, sync_active=True), 1.0)
    t.record_status(_status(DROPPED), 2.0)
    for attempt in range(1, 6):
        failing = {**DROPPED, "attempts": attempt,
                   "lastError": {"text": "mirabot.local does not resolve (ENOTFOUND)", "at": f"x{attempt}"}}
        t.record_status(_status(failing), 2.0 + attempt)
    t.record_status(_status({**CONNECTED, "lastDisconnect": DROPPED["lastDisconnect"]}, sync_active=True), 9.0)
    assert _texts(t) == [
        "TCode link connected: ws://mirabot.local:8888 (192.168.2.216)",
        "Sync engine started",
        "TCode link lost, reconnecting: ws://mirabot.local:8888 (close=1006 (abnormal, no close frame) clean=false)",
        "Sync engine stopped",
        "Reconnect failing: mirabot.local does not resolve (ENOTFOUND) (attempt 1)",
        "TCode link connected: ws://mirabot.local:8888 (192.168.2.216)",
        "Sync engine started",
    ]


def test_no_device_in_snapshots_and_stopped_in_samples_do_not_flap():
    t = Telemetry()
    stopped = {"connected": False, "transport": None, "state": "stopped"}
    for i in range(3):
        t.record_status({"devices": {"tcode": None}}, i * 2.0)
        t.record_sample({"tcode": {"link": stopped}, "sync": {"active": False, "axes": []}}, i * 2.0 + 1)
    assert [e for e in _texts(t) if "TCode" in e] == ["TCode link stopped"]


def test_headset_bridge_going_quiet_and_dropping_is_on_the_timeline():
    t = Telemetry()
    vr = {"connected": True, "linkState": "receiving", "playerType": "heresphere", "host": "192.168.2.251", "video": "a.mp4"}
    t.record_status(_status(CONNECTED, vr=vr), 1.0)
    t.record_status(_status(CONNECTED, vr={**vr, "linkState": "waiting"}), 2.0)
    t.record_status(_status(CONNECTED, vr={"connected": False, "linkState": "disconnected"}), 3.0)
    assert _texts(t)[1:] == [
        "VR bridge connected: heresphere at 192.168.2.251",
        "VR bridge: no timestamp packets for 5 s",
        "VR bridge disconnected",
    ]
