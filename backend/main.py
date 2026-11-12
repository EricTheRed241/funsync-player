"""FunSync Player — Python Backend (FastAPI)"""

import argparse
import logging
import os
import sys
import uvicorn
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.staticfiles import StaticFiles

from routes.thumbnails import router as thumbnails_router
from routes.metadata import router as metadata_router
from routes.media import router as media_router, set_thumb_cache_dir
from routes.vr_api import router as vr_api_router
from routes.remote import router as remote_router
from routes.remote_sync import router as remote_sync_router
from routes.tcode_relay import router as tcode_relay_router
from routes.locale import router as locale_router, set_user_data_dir
from routes.status import router as status_router
from routes.device import router as device_router
from services import library_edits, stereo_detect, video_info
import access_log


def install_app_logging() -> None:
    """Give the backend's own loggers (`funsync.*`) a handler. Without one,
    INFO never reached main.log and warnings came out without their level,
    so the desktop logged them as errors. Same `LEVEL:` prefix as uvicorn's
    lines, which the desktop maps back to the level (python-bridge.js)."""
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(logging.Formatter("%(levelname)s:  [%(name)s] %(message)s"))
    logger = logging.getLogger("funsync")
    logger.addHandler(handler)
    logger.setLevel(logging.INFO)
    logger.propagate = False


app = FastAPI(title="FunSync Backend", version="0.1.0")

# Allow Electron renderer to call the backend
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(thumbnails_router, prefix="/thumbnails", tags=["thumbnails"])
app.include_router(metadata_router, prefix="/metadata", tags=["metadata"])
app.include_router(media_router, prefix="/api/media", tags=["media"])
app.include_router(vr_api_router, tags=["vr"])
app.include_router(remote_router, prefix="/api/remote", tags=["remote"])
app.include_router(remote_sync_router, prefix="/api/remote", tags=["remote-sync"])
app.include_router(tcode_relay_router, prefix="/api/remote", tags=["tcode-relay"])
app.include_router(locale_router, prefix="/api/remote", tags=["locale"])
app.include_router(status_router, prefix="/api", tags=["status"])
# Local-only device dashboard at /device/ (routes/device.py).
app.include_router(device_router, tags=["device"])

# Web remote SPA served at /remote/ — static files live in backend/web-remote/.
_remote_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "web-remote")
if os.path.isdir(_remote_dir):
    app.mount("/remote", StaticFiles(directory=_remote_dir, html=True), name="web-remote")


# Stale-asset guard for the web remote (2026-08-04): after an app update the
# phone would mix cached index.html/style.css with freshly-fetched app.js —
# broken hybrid UI until a manual reload (bit Dave live on the 0.9 remote
# rework; versioned URLs wouldn't fully fix it because app.js's sibling ES-
# module imports cache independently). `no-cache` forces revalidation on
# every /remote/ asset: StaticFiles serves ETags, so unchanged files are
# 304s — trivial on LAN — and changed files are always picked up.
@app.middleware("http")
async def _remote_no_stale_cache(request, call_next):
    response = await call_next(request)
    path = request.url.path
    if path == "/remote" or path.startswith("/remote/") or path.startswith("/locales/"):
        response.headers["Cache-Control"] = "no-cache"
    return response

# Locale bundles for the web-remote i18n module, at <project>/renderer/locales/
# (two dirs up from backend/main.py). Web-remote fetches `/locales/<lang>.json`
# at boot.
def _resolve_locales_dir() -> str | None:
    here = os.path.dirname(os.path.abspath(__file__))
    locales = os.path.abspath(os.path.join(here, "..", "renderer", "locales"))
    return locales if os.path.isdir(locales) else None

_locales_dir = _resolve_locales_dir()
if _locales_dir:
    app.mount("/locales", StaticFiles(directory=_locales_dir), name="locales")


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.get("/")
async def root():
    """Root URL — simple text so DeoVR's auto-discovery requests /deovr instead.

    DeoVR only enters 'Json mode' when the URL path contains '/deovr'.
    Returning JSON here would cause DeoVR to display raw text.
    """
    return {"status": "FunSync VR Server", "deovr": "/deovr", "heresphere": "/heresphere"}


@app.get("/network-info")
async def network_info():
    """Return the local network IP for VR server URL display."""
    from services.network import get_local_ip
    return {"ip": get_local_ip()}


if __name__ == "__main__":
    install_app_logging()
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=5123)
    parser.add_argument("--host", type=str, default="0.0.0.0")
    # Path to electron-conf's userData dir (contains config.json). Used
    # by the web-remote locale endpoint to mirror the desktop's chosen
    # language, and to persist the VR resolution cache. Optional — without
    # it the locale falls back to 'en' and the cache is memory-only.
    parser.add_argument("--user-data-dir", type=str, default=None)
    args = parser.parse_args()

    if args.user_data_dir:
        set_user_data_dir(args.user_data_dir)
        video_info.set_cache_dir(args.user_data_dir)
        stereo_detect.set_cache_dir(args.user_data_dir)
        # Phone / headset edits waiting for the desktop survive a restart.
        library_edits.queue.set_persist_dir(args.user_data_dir)
        # Same folder the desktop registers. Set here too so thumbnails work
        # even when the desktop's first registration lands before the server
        # is listening (it is fire-and-forget) or after a backend restart.
        set_thumb_cache_dir(os.path.join(args.user_data_dir, "thumb-cache"))

    # Keep the desktop's own 1 Hz polling out of main.log.
    access_log.install()
    uvicorn.run(app, host=args.host, port=args.port)
