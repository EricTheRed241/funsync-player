"""GET /api/status: the desktop's latest snapshot plus the backend's own state."""

import time

import pytest
from fastapi.testclient import TestClient

from main import app
from routes import media, remote_sync


@pytest.fixture(autouse=True)
def reset_state():
    remote_sync._reset_state_for_tests()
    media.register_videos([])
    media._vr_activity.update({"clientIp": None, "videoId": None, "timestamp": 0})
    yield
    remote_sync._reset_state_for_tests()
    media.register_videos([])
    media._vr_activity.update({"clientIp": None, "videoId": None, "timestamp": 0})


@pytest.fixture
def client():
    with TestClient(app) as c:
        yield c


def _wait_status(client, pred, timeout=2.0):
    end = time.time() + timeout
    while time.time() < end:
        body = client.get("/api/status").json()
        if pred(body):
            return body
        time.sleep(0.02)
    raise AssertionError(f"status never matched: {body}")


WAITING_TCODE = {
    "at": "2026-11-03T09:00:00.000Z",
    "devices": {
        "buttplug": {"connected": False, "devices": []},
        "tcode": {
            "connected": False, "waiting": True, "syncActive": False,
            "link": {
                "state": "waiting", "url": "ws://mirabot.local:8888", "attempts": 12,
                "downSince": "2026-11-03T08:59:00.000Z",
                "lastError": {"text": "mirabot.local does not resolve (ENOTFOUND)", "at": "2026-11-03T08:59:55.000Z"},
            },
        },
    },
    "vr": {"connected": False, "linkState": "disconnected"},
    "session": None,
    "player": {"video": None, "playing": False, "currentTime": None},
}


def test_without_the_desktop_says_so(client):
    body = client.get("/api/status").json()
    assert body["desktop"]["connected"] is False
    assert body["devices"] is None
    assert any("Desktop app is not connected" in i for i in body["issues"])


def test_serves_the_desktop_snapshot_and_explains_a_waiting_device(client):
    with client.websocket_connect("/api/remote/sync/observe") as obs:
        obs.send_json({"type": "desktop-status", "status": WAITING_TCODE})
        body = _wait_status(client, lambda b: b["devices"] is not None)
    assert body["devices"]["tcode"]["link"]["state"] == "waiting"
    assert body["desktop"]["statusAgeS"] is not None
    tcode_issue = next(i for i in body["issues"] if i.startswith("TCode"))
    assert "ws://mirabot.local:8888" in tcode_issue
    assert "12 attempts" in tcode_issue
    assert "ENOTFOUND" in tcode_issue
    assert "No device is connected." in body["issues"]


def test_desktop_status_is_not_forwarded_to_the_phone(client):
    with client.websocket_connect("/api/remote/sync?videoId=x") as phone:
        with client.websocket_connect("/api/remote/sync/observe") as obs:
            obs.receive_json()  # phone-connected catch-up
            obs.send_json({"type": "desktop-status", "status": WAITING_TCODE})
            obs.send_json({"type": "device-status", "devices": []})
            assert phone.receive_json()["type"] == "device-status"


def test_flags_a_headset_session_the_devices_dont_follow(client):
    media.register_videos([{"path": "/lib/scene.mp4", "name": "Scene"}])
    video_id = next(iter(media.get_video_registry()))
    media.record_vr_activity("192.168.2.50", video_id)
    with client.websocket_connect("/api/remote/sync/observe") as obs:
        obs.send_json({"type": "desktop-status", "status": WAITING_TCODE})
        body = _wait_status(client, lambda b: b["devices"] is not None)
    assert body["headset"]["video"] == "Scene"
    assert body["headset"]["clientIp"] == "192.168.2.50"
    assert any("VR companion bridge isn't connected" in i for i in body["issues"])
