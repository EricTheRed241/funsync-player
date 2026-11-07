"""Local device dashboard: GET /device and GET /api/device/telemetry.

Only answers requests from this machine, even when the backend listens on
the LAN for the web remote and the headset: the page shows the device link,
the commands it receives and the playback source in detail. Data comes from
services/device_telemetry.py; each telemetry poll also renews the desktop's
streaming lease.
"""

import os
import time

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import FileResponse, RedirectResponse

from routes import remote_sync
from routes.status import get_status
from services.device_telemetry import LEASE_MS, telemetry

router = APIRouter()

_PAGE_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "device-dashboard")
_PAGE_FILES = {"": "index.html", "dashboard.js": "dashboard.js", "dashboard.css": "dashboard.css"}
_LOCAL_HOSTS = {"127.0.0.1", "::1"}


def _require_local(request: Request) -> None:
    host = request.client.host if request.client else None
    if host not in _LOCAL_HOSTS:
        raise HTTPException(status_code=403, detail="The device dashboard is only available on this computer.")


@router.get("/device", include_in_schema=False)
async def device_page_redirect(request: Request):
    _require_local(request)
    return RedirectResponse("/device/")


@router.get("/device/{name:path}", include_in_schema=False)
async def device_page(name: str, request: Request):
    _require_local(request)
    file = _PAGE_FILES.get(name)
    if file is None:
        raise HTTPException(status_code=404)
    return FileResponse(os.path.join(_PAGE_DIR, file), headers={"Cache-Control": "no-cache"})


@router.get("/api/device/telemetry")
async def device_telemetry(request: Request, since: int = 0, cmdSince: int = 0, eventSince: int = 0):
    """Everything newer than the caller's cursors, plus /api/status."""
    _require_local(request)
    now = time.time()
    if telemetry.lease_due(now) and await remote_sync.notify_desktop({"type": "telemetry-lease", "ms": LEASE_MS}):
        telemetry.lease_sent(now)
    # The first poll gets the last 30 s of commands, not all that's kept.
    cmd_since = cmdSince or round((now - 30) * 1000)
    return {
        "now": round(now * 1000),
        **telemetry.view(since, cmd_since, eventSince, now),
        "status": await get_status(),
    }
