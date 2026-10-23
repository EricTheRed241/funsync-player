"""Thumbnail generation routes — generate and serve video thumbnails."""

import base64
import os
from fastapi import APIRouter, HTTPException, Query
from fastapi.responses import FileResponse

from services.ffmpeg import generate_thumbnails, generate_single_thumbnail, sample_gray_frames

router = APIRouter()


@router.post("/generate")
def generate(
    video_path: str = Query(..., description="Absolute path to video file"),
    interval: int = Query(10, description="Seconds between thumbnails"),
):
    """Generate thumbnail images for a video file using ffmpeg.

    Declared `def` (not `async def`) because `generate_thumbnails` calls
    `subprocess.run(ffmpeg)` synchronously; FastAPI runs sync handlers
    in its threadpool so a slow ffmpeg doesn't stall the event loop.
    """
    try:
        manifest = generate_thumbnails(video_path, interval=interval)
        return manifest
    except FileNotFoundError as e:
        raise HTTPException(status_code=404, detail=str(e))
    except RuntimeError as e:
        raise HTTPException(status_code=500, detail=str(e))


@router.post("/single")
def single(
    video_path: str = Query(..., description="Absolute path to video file"),
    seek_pct: float = Query(0.1, description="Where in the video (0-1)"),
    width: int = Query(320, description="Output width in pixels"),
    exact: bool = Query(False, description="Frame-accurate grab at seek_pct "
                        "(user-picked thumbnail frame; skips the keyframe-only "
                        "fast path and the 10s minimum-seek heuristic)"),
):
    """Generate ONE thumbnail for library card display. Cached on disk
    by content hash so repeat calls are nearly free. Replaces the
    renderer's hidden-<video> decode path which was the dominant
    startup cost on small libraries (~3s per thumbnail × 12 visible
    cards × concurrency 3 = ~12s).

    Declared `def` (not `async def`) because `generate_single_thumbnail`
    calls `subprocess.run(ffmpeg, timeout=30)` synchronously; FastAPI
    runs sync handlers in its threadpool so parallel library card
    requests don't serialise on the event loop.
    """
    try:
        return generate_single_thumbnail(video_path, seek_pct=seek_pct, width=width, exact=exact)
    except FileNotFoundError as e:
        raise HTTPException(status_code=404, detail=str(e))
    except RuntimeError as e:
        raise HTTPException(status_code=500, detail=str(e))


@router.post("/samples")
def samples(
    video_path: str = Query(..., description="Absolute path to video file"),
    min_count: int = Query(12, description="Fewest frames to return"),
    width: int = Query(128, description="Frame width in pixels"),
):
    """Grayscale frames across the video, for matching EroScripts preview
    images against it (electron/preview-validator.js). Each frame's raw
    width*height luma bytes are base64-encoded.

    Sync `def`: the ffmpeg pass blocks, FastAPI runs it in its threadpool.
    """
    try:
        result = sample_gray_frames(video_path, min_count=min_count, width=width)
    except FileNotFoundError as e:
        raise HTTPException(status_code=404, detail=str(e))
    except RuntimeError as e:
        raise HTTPException(status_code=500, detail=str(e))
    return {
        **result,
        "frames": [
            {"pct": f["pct"], "gray": base64.b64encode(f["gray"]).decode("ascii")}
            for f in result["frames"]
        ],
    }


@router.get("/image")
async def get_thumbnail(path: str = Query(..., description="Absolute path to thumbnail file")):
    """Serve a single thumbnail image file."""
    if not os.path.exists(path):
        raise HTTPException(status_code=404, detail="Thumbnail not found")
    return FileResponse(path, media_type="image/jpeg")
