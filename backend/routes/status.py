"""One-call status for diagnosing a session: GET /api/status.

Combines what the backend knows itself (uptime, library size, the last
headset scene request, the web-remote phone) with the desktop app's latest
`desktop-status` snapshot (devices, TCode link, VR companion bridge, the
current session; built in renderer/js/desktop-status.js and sent over the
observer socket on every change and every 15 s). `issues` lists what looks
wrong, in plain words, so a session going wrong mid-video can be read off
one response instead of main.log.
"""

import os
import time
from datetime import datetime, timezone

from fastapi import APIRouter

from routes import remote_sync
from routes.media import get_video_registry, last_vr_activity

router = APIRouter()

_STARTED_AT = time.time()
# The desktop sends a snapshot at least every 15 s; older than this means the
# renderer is stuck or the snapshots stopped.
STALE_AFTER_S = 45
# A headset scene request this recent counts as an active headset session.
HEADSET_ACTIVE_S = 15 * 60


def _iso(ts):
    if not ts:
        return None
    return datetime.fromtimestamp(ts, timezone.utc).isoformat(timespec="seconds")


def _age(ts, now):
    return round(now - ts, 1) if ts else None


def _issues(desktop, status, headset_age, now):
    issues = []
    if not desktop["connected"]:
        issues.append("Desktop app is not connected to the backend (app closed, starting, or its renderer is stuck).")
    if status is None:
        if desktop["connected"]:
            issues.append("Desktop app is connected but hasn't sent a status yet.")
        return issues
    age = _age(desktop["status_at"], now)
    if age is not None and age > STALE_AFTER_S:
        issues.append(f"Desktop status is {age:.0f} s old: the renderer may be frozen or busy.")

    devices = status.get("devices") or {}
    tcode = devices.get("tcode") or {}
    link = tcode.get("link") or {}
    state = link.get("state")
    if state in ("waiting", "reconnecting", "connecting"):
        since = link.get("downSince")
        err = (link.get("lastError") or {}).get("text")
        doing = {
            "waiting": "not answering yet",
            "reconnecting": "connection lost, reconnecting",
            "connecting": "connecting",
        }[state]
        text = f"TCode device {link.get('url')}: {doing}"
        if since:
            text += f" since {since}"
        text += f" ({link.get('attempts', 0)} attempts)"
        if err:
            text += f". Last failure: {err}"
        issues.append(text + ".")
    if not any((d or {}).get("connected") for d in devices.values()):
        issues.append("No device is connected.")

    vr = status.get("vr") or {}
    if headset_age is not None and headset_age <= HEADSET_ACTIVE_S:
        if not vr.get("connected"):
            issues.append(
                f"The headset asked for a scene {headset_age:.0f} s ago, but the VR companion bridge "
                "isn't connected, so devices aren't following the headset video "
                "(HereSphere's timestamp server off or unreachable)."
            )
        elif vr.get("linkState") == "waiting":
            issues.append("VR companion bridge is connected but has had no packets from the player for over 5 s.")
    return issues


@router.get("/status")
async def get_status():
    now = time.time()
    desktop = remote_sync.desktop_state()
    status = desktop["status"]
    phone = remote_sync.phone_state()

    vr_req = last_vr_activity()
    headset_age = _age(vr_req.get("timestamp"), now)
    vr_video = get_video_registry().get(vr_req.get("videoId")) if vr_req.get("videoId") else None

    return {
        "at": _iso(now),
        "issues": _issues(desktop, status, headset_age, now),
        "backend": {
            "pid": os.getpid(),
            "startedAt": _iso(_STARTED_AT),
            "uptimeS": round(now - _STARTED_AT),
            "videos": len(get_video_registry()),
        },
        "desktop": {
            "connected": desktop["connected"],
            "connectedSince": _iso(desktop["connected_since"]),
            "statusAt": _iso(desktop["status_at"]),
            "statusAgeS": _age(desktop["status_at"], now),
        },
        "devices": (status or {}).get("devices"),
        "vrBridge": (status or {}).get("vr"),
        "session": (status or {}).get("session"),
        "player": (status or {}).get("player"),
        "headset": {
            "lastRequestAt": _iso(vr_req.get("timestamp")),
            "lastRequestAgeS": headset_age,
            "clientIp": vr_req.get("clientIp"),
            "videoId": vr_req.get("videoId"),
            "video": (vr_video or {}).get("name"),
        },
        "phone": {
            "connected": phone["connected"],
            "ip": phone["ip"],
            "videoId": phone["video_id"],
            "video": os.path.basename(phone["video_path"]) if phone["video_path"] else None,
            "lastState": phone["last_state"],
        },
    }
