"""Web remote routes — phone-friendly library listing and triage edits.

The SPA itself lives in `backend/web-remote/` and is served via a
`StaticFiles` mount on `/remote/` (wired up in `main.py`). This module
exposes the JSON endpoints the SPA calls: the flat video list, the
collection/playlist/category/artist groupings, and the triage writes (rating,
favourite, add to / remove from a playlist, tag scores, favourite artists).

The writes never touch the library here: the desktop owns it. Each becomes
a library edit (`services.library_edits`) delivered to the desktop over the
observer socket, queued while the desktop isn't connected. Responses carry
`delivered` so the phone can say "saved, applies when the desktop app
reconnects"; the GET endpoints reflect queued edits straight away.

The heavy lifting (video streaming with Range support, funscript and
subtitle serving, ffmpeg thumbnails) is already handled by the existing
`media.py` routes. The remote SPA points at those same URLs — no
duplication.
"""

import os
import uuid

from fastapi import APIRouter, Body, HTTPException

from routes.media import (
    get_video_registry,
    get_collections,
    get_playlists,
    get_ratings,
    get_categories,
    get_video_categories,
    get_chip_tags,
    get_preference_score,
    get_tag_scores,
    get_tag_score_info,
    get_artists,
    get_favourite_artists,
    get_favourite_artist_score,
    get_sources,
    bump_rescan_request,
    _path_to_id,
)
from routes.remote_sync import desktop_connected, submit_library_edit
from routes.vr_api import vr_decision
from services import library_edits

router = APIRouter()

# Changes on every app start, appended to the stream URLs the remote uses.
#
# A phone that cached partial responses from a previous run cannot be reached
# by a server-side header change — the entries are already on the device. A
# fresh URL per boot sidesteps them without asking the user to find a "clear
# website data" button on a phone (Dave, 2026-08-21).
_BOOT_TOKEN = uuid.uuid4().hex[:8]


@router.post("/seek-timing")
async def seek_timing(payload: dict = Body(default={})):
    """Record one seek from the web remote, for diagnosis.

    The desktop cannot see the phone's side of a seek, and that is where the
    answer lives: whether the wait was for BYTES (a buffer miss, so network or
    range serving) or for the round trip. The 2026-08-18 stall fix needed wire
    captures for exactly this reason. One small POST per seek puts the numbers
    in main.log instead, where Dave already looks.

    Deliberately print(), not a logger: backend stdout is forwarded into
    main.log with a [Backend] prefix, which is the same path uvicorn's own
    access lines take.
    """
    def _num(key, default=-1.0):
        try:
            return float(payload.get(key, default))
        except (TypeError, ValueError):
            return default

    to_s = _num("to")
    from_s = _num("from")
    seeked_ms = _num("seekedMs")
    playing_ms = _num("playingMs")
    buffered = "hit" if payload.get("buffered") else "miss"
    delta = to_s - from_s if to_s >= 0 and from_s >= 0 else 0.0
    kind = str(payload.get("kind", "?"))[:16]

    print(
        f"[remote-seek] {kind} to={to_s:.1f}s d={delta:+.1f}s buffered={buffered} "
        f"seeked={seeked_ms:.0f}ms playing={playing_ms:.0f}ms",
        flush=True,
    )
    return {"ok": True}


@router.post("/request-rescan")
async def request_rescan():
    """Ask the desktop to rescan its source folders and re-register videos.

    The phone can only see the snapshot the desktop last pushed, so a file the
    user just dropped into a source folder is invisible until the desktop
    rescans. The Refresh button on the web remote calls this; the desktop polls
    the counter (via /api/media/rescan-request) and rescans when it advances.
    Returns the new counter so the phone could correlate if it wanted to.
    """
    return {"seq": bump_rescan_request()}


@router.get("/videos")
async def list_videos():
    """Return every registered video in a phone-friendly JSON shape.

    URLs are returned as relative paths so the phone resolves them against
    whatever origin it reached the backend on (LAN IP, mDNS, reverse proxy).
    """
    registry = get_video_registry()
    ratings = get_ratings()
    categories = get_categories()
    video_categories = get_video_categories()
    out = []
    for vid_id, v in registry.items():
        name = v.get("name", "")
        stem = os.path.splitext(name)[0] or name
        # Surface script variants (`<base> (Soft).funscript`, `<base>.intense
        # .funscript`, etc.) the desktop already detected during library
        # scan. Empty list when the video has 0 or 1 variant — the phone
        # treats that as "no variant chip needed" so single-script videos
        # render exactly as before. Each variant exposes a fetchable
        # `scriptUrl` so the phone can render mini-heatmap previews per
        # variant in the expanded list.
        variants_raw = v.get("variants") or []
        variants_out = []
        for variant in variants_raw:
            label = (variant.get("label") or "").strip()
            if not label:
                continue
            variants_out.append({
                "label": label,
                "scriptUrl": f"/api/media/script/{vid_id}?variant={label}",
            })

        out.append({
            "id": vid_id,
            "name": stem,
            # Absolute path — needed by the phone's folder-browse tree
            # builder. Only exposed on the LAN port where the user has
            # already allowed their video library to be streamed, so no
            # new attack surface.
            "path": v.get("path") or "",
            "hasFunscript": bool(v.get("funscriptPath")),
            "hasSubtitle": bool(v.get("subtitlePath")),
            "duration": v.get("duration") or 0,
            # Speed stats in units/s (0-999 typical). Null = not yet probed;
            # phone treats null same as 0 but can keep polling. 0 = probe
            # ran and the script was empty/unreadable.
            "avgSpeed": v.get("avgSpeed"),
            "maxSpeed": v.get("maxSpeed"),
            # File mtime in ms (populated by the Electron scan). Lets the
            # phone sort by "recently added" without needing its own stat
            # access. Missing → 0, sorts to the "oldest" end.
            "dateAdded": v.get("dateAdded") or 0,
            "sourceName": v.get("sourceName") or "Library",
            "streamUrl": f"/api/media/stream/{vid_id}?v={_BOOT_TOKEN}",
            "scriptUrl": f"/api/media/script/{vid_id}" if v.get("funscriptPath") else None,
            "subtitleUrl": f"/api/media/subtitle/{vid_id}" if v.get("subtitlePath") else None,
            "thumbUrl": f"/api/media/thumb/{vid_id}",
            "variants": variants_out,
            # Manual VR override (`'vr'` or `'flat'`); absent / null = use the
            # filename heuristic. The phone-side `vr-detect.js` consults its
            # override store before the heuristic so the VR filter agrees with
            # the desktop.
            "manualVRType": v.get("manualVRType") or None,
            # The backend's VR decision (VR Format projection, manual toggle,
            # filename, frame shape — `routes.vr_api`), the same one the
            # headsets get. False while the frame shape is still unknown.
            "isVR": bool(vr_decision(vid_id, v)),
            # Chip tags (EroScripts / FapTap post tags without the noise,
            # `services.video_tags`) and desktop category names.
            "tags": get_chip_tags(v.get("path")),
            "categories": _category_names(v.get("path"), categories, video_categories),
            # Triage state (0 = unrated), queued phone/headset edits included.
            "rating": library_edits.rating_entry(ratings, v.get("path"))[0],
            "favorite": library_edits.rating_entry(ratings, v.get("path"))[1],
            # The desktop's preference score (renderer/js/library-scores.js:
            # the scouts' ranking applied to the scene); null = not scored yet.
            "score": get_preference_score(v.get("path")),
        })
    out.sort(key=lambda v: v["name"].lower())
    return {"videos": out}


def _category_names(path: str, categories: list[dict], video_categories: dict) -> list[str]:
    """Names of the desktop categories holding `path`, in category order."""
    ids = set(video_categories.get(path) or []) if path else set()
    if not ids:
        return []
    return [
        (c.get("name") or "").strip()
        for c in categories
        if c.get("id") in ids and (c.get("name") or "").strip()
    ]
    out.sort(key=lambda v: v["name"].lower())
    return {"videos": out}


def _video_ids_from_paths(paths: list[str]) -> list[str]:
    """Convert a list of absolute file paths into their registered video IDs.

    Paths that aren't in the registry (disconnected source, deleted file)
    are silently dropped — same "view only" behaviour the desktop has when
    a playlist references a missing file.
    """
    registry = get_video_registry()
    out = []
    for p in paths or []:
        if not p:
            continue
        vid_id = _path_to_id(p)
        if vid_id in registry:
            out.append(vid_id)
    return out


def _total_duration(video_ids: list[str]) -> float:
    """Sum the duration (seconds) of the given registered videos.

    Silently ignores IDs missing a duration (not all videos have been
    probed by ffprobe yet when the phone polls). Returns 0 for an empty
    list so clients can treat missing duration the same as "zero".
    """
    registry = get_video_registry()
    total = 0.0
    for vid_id in video_ids:
        v = registry.get(vid_id)
        if not v:
            continue
        dur = v.get("duration") or 0
        try:
            total += float(dur)
        except (TypeError, ValueError):
            continue
    return total


@router.get("/sources")
async def list_sources():
    """Return the enabled library source folders the phone can use to seed
    its folder-browse tree. {id, name, path} per source."""
    out = []
    for s in get_sources():
        out.append({
            "id": s.get("id"),
            "name": s.get("name") or "",
            "path": s.get("path") or "",
        })
    out.sort(key=lambda s: (s["name"] or "").lower())
    return {"sources": out}


@router.get("/collections")
async def list_collections():
    """Return every collection with its video IDs (view-only).

    The desktop shape stores `videoPaths` as absolute file paths; we
    convert to the registry IDs the phone already uses so the client
    doesn't need to know about file paths at all.
    """
    out = []
    for c in get_collections():
        video_ids = _video_ids_from_paths(c.get("videoPaths") or [])
        out.append({
            "id": c.get("id"),
            "name": c.get("name") or "(unnamed)",
            "videoCount": len(video_ids),
            "totalDuration": _total_duration(video_ids),
            "videoIds": video_ids,
        })
    out.sort(key=lambda c: (c["name"] or "").lower())
    return {"collections": out}


@router.get("/playlists")
async def list_playlists():
    """Return every playlist with its video IDs (view-only)."""
    out = []
    for p in get_playlists():
        video_ids = _video_ids_from_paths(p.get("videoPaths") or [])
        out.append({
            "id": p.get("id"),
            "name": p.get("name") or "(unnamed)",
            "videoCount": len(video_ids),
            "totalDuration": _total_duration(video_ids),
            "videoIds": video_ids,
            "createdAt": p.get("createdAt") or 0,
            # Created on the phone / headset, not yet applied by the desktop.
            "pending": bool(p.get("pending")),
        })
    out.sort(key=lambda p: (p["name"] or "").lower())
    return {"playlists": out}


def _video_path_or_404(video_id) -> str:
    entry = get_video_registry().get(video_id) if isinstance(video_id, str) else None
    if not entry or not entry.get("path"):
        raise HTTPException(status_code=404, detail="Unknown video")
    return entry["path"]


async def _submit(edit: dict) -> dict:
    try:
        stamped, delivered = await submit_library_edit(edit)
    except library_edits.QueueFull as e:
        raise HTTPException(status_code=503, detail=str(e))
    return {"ok": True, "seq": stamped["seq"], "delivered": delivered}


@router.post("/rating")
async def set_rating(payload: dict = Body(default={})):
    """Set a video's rating and/or favourite flag.

    Body `{videoId, rating?, favorite?}`: `rating` 0-5 in steps of 0.5
    (0 clears), `favorite` a bool; an absent field keeps its value. 400 on an
    invalid value or nothing to change, 404 on an unknown video. Returns
    `{ok, seq, delivered, rating, favorite}` with the resulting state.
    """
    path = _video_path_or_404(payload.get("videoId"))
    try:
        edit = library_edits.make_rating_edit(
            path, rating=payload.get("rating"), favorite=payload.get("favorite"), source="remote",
        )
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    result = await _submit(edit)
    rating, favorite = library_edits.rating_entry(get_ratings(), path)
    return {**result, "rating": rating, "favorite": favorite}


def _playlist_edit(kind: str, payload: dict) -> dict:
    path = _video_path_or_404(payload.get("videoId"))
    name = payload.get("name")
    try:
        return library_edits.make_playlist_edit(
            kind, path,
            playlist_id=payload.get("playlistId") if isinstance(payload.get("playlistId"), str) else None,
            name=name if isinstance(name, str) else None,
            source="remote",
        )
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))


@router.post("/playlists/add-video")
async def add_video_to_playlist(payload: dict = Body(default={})):
    """Add a video to a playlist: `{videoId, playlistId?, name?}`. An
    unknown `playlistId` falls back to `name`; a `name` matching no playlist
    (case-insensitive) creates it. Adding a video that is already there is a
    no-op on the desktop."""
    return await _submit(_playlist_edit(library_edits.KIND_PLAYLIST_ADD, payload))


@router.post("/playlists/remove-video")
async def remove_video_from_playlist(payload: dict = Body(default={})):
    """Remove a video from a playlist: `{videoId, playlistId?, name?}`."""
    return await _submit(_playlist_edit(library_edits.KIND_PLAYLIST_REMOVE, payload))


@router.get("/tag-scores")
async def list_tag_scores():
    """Every tag's score, for the Tags view.

    `{scores: {tag: score}, pending: [tag, ...], recalculating: bool,
    groups: {tag: group}, rules: {id: number}}`: `scores` holds only tags
    with a score (absent = 0; the desktop's starting scores included) and
    already includes edits waiting for the desktop; `pending` names those
    tags; `recalculating` is true while a Recalculate scores request waits
    for the desktop. `groups` names the group of each grouped tag (a video
    counts each group once); `rules` are the built-in weights that aren't
    about a tag (read-only).
    """
    pending = library_edits.queue.pending()
    return {
        "scores": get_tag_scores(),
        "pending": sorted({e["tag"] for e in pending if e.get("kind") == library_edits.KIND_TAG_SCORE}),
        "recalculating": any(e.get("kind") == library_edits.KIND_RECALCULATE_SCORES for e in pending),
        **get_tag_score_info(),
    }


@router.post("/tag-score")
async def set_tag_score(payload: dict = Body(default={})):
    """Set the score of a tag: `{tag, score}`. The score is rounded to 0.5
    and clamped to -10 … +10. The desktop stores it in `library.tagScores`
    (as a change from the tag's starting score) and scores the library
    again, so the videos' `score` follows once it has applied the edit."""
    try:
        edit = library_edits.make_tag_score_edit(payload.get("tag"), payload.get("score"), source="remote")
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    result = await _submit(edit)
    return {**result, "tag": edit["tag"], "score": edit["score"]}


@router.post("/recalculate-scores")
async def recalculate_scores():
    """Ask the desktop to score every video again now (library-scores.js)
    and re-register the scores. One request waits at a time: asking again
    while one is pending returns that one. Done when `GET /tag-scores`
    reports `recalculating: false`."""
    for edit in library_edits.queue.pending():
        if edit.get("kind") == library_edits.KIND_RECALCULATE_SCORES:
            return {"ok": True, "seq": edit["seq"], "delivered": desktop_connected(), "alreadyPending": True}
    return await _submit(library_edits.make_recalculate_edit(source="remote"))


@router.get("/artists")
async def list_artists():
    """Every artist of the library plus the favourites with no videos, for
    the Artists view.

    `{artists: [{id, name, videoCount, totalDuration, videoIds, favourite,
    pending}], score, defaultScore, scorePending}`. `id` is the artist's
    folded name (`library_edits.artist_key`); `favourite` and `score` (the
    favourite-artist bonus) already include edits waiting for the desktop,
    which `pending` / `scorePending` flag. Favourites first, then by name.
    """
    pending = library_edits.queue.pending()
    pending_keys = {
        library_edits.artist_key(e.get("artist"))
        for e in pending if e.get("kind") == library_edits.KIND_FAVOURITE_ARTIST
    }
    favourites = get_favourite_artists()
    favourite_keys = {library_edits.artist_key(n) for n in favourites}

    def entry(key, name, video_ids):
        return {
            "id": key,
            "name": name,
            "videoCount": len(video_ids),
            "totalDuration": _total_duration(video_ids),
            "videoIds": video_ids,
            "favourite": key in favourite_keys,
            "pending": key in pending_keys,
        }

    out, seen = [], set()
    for artist in get_artists():
        key = library_edits.artist_key(artist["name"])
        seen.add(key)
        out.append(entry(key, artist["name"], _video_ids_from_paths(artist["videoPaths"])))
    for name in favourites:
        key = library_edits.artist_key(name)
        if key not in seen:
            seen.add(key)
            out.append(entry(key, name, []))
    out.sort(key=lambda a: (not a["favourite"], a["name"].casefold()))
    score = get_favourite_artist_score()
    return {
        "artists": out,
        "score": score["score"],
        "defaultScore": score["default"],
        "scorePending": any(e.get("kind") == library_edits.KIND_FAVOURITE_SCORE for e in pending),
    }


@router.post("/favourite-artist")
async def set_favourite_artist(payload: dict = Body(default={})):
    """Make an artist a favourite or not: `{artist, favourite}`. The desktop
    stores it in `library.favouriteArtists` (as a change from the built-in
    favourites) and scores the library again."""
    try:
        edit = library_edits.make_favourite_artist_edit(payload.get("artist"), payload.get("favourite"), source="remote")
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    result = await _submit(edit)
    return {**result, "artist": edit["artist"], "favourite": edit["favourite"]}


@router.post("/favourite-score")
async def set_favourite_score(payload: dict = Body(default={})):
    """Set the bonus a favourite artist's scenes get: `{score}`, rounded to
    0.5 and clamped to -10 … +10 like a tag score. The desktop stores it in
    `library.favouriteArtistScore` and scores the library again."""
    try:
        edit = library_edits.make_favourite_score_edit(payload.get("score"), source="remote")
    except ValueError as e:
        raise HTTPException(status_code=400, detail=str(e))
    result = await _submit(edit)
    return {**result, "score": edit["score"]}


@router.get("/categories")
async def list_categories():
    """Return every category with its colour + video IDs (view-only).

    Categories are stored as `{id, name, color}` in settings, and the
    videoPath→[categoryId] mapping lives in a separate table. We invert
    it server-side so the phone gets a single hop: category → videoIds.
    """
    # Build categoryId → set(videoPath) from the flat video-categories map
    inverted: dict[str, list[str]] = {}
    for video_path, cat_ids in get_video_categories().items():
        for cat_id in cat_ids or []:
            inverted.setdefault(cat_id, []).append(video_path)

    out = []
    for c in get_categories():
        paths = inverted.get(c.get("id"), [])
        video_ids = _video_ids_from_paths(paths)
        out.append({
            "id": c.get("id"),
            "name": c.get("name") or "(unnamed)",
            "color": c.get("color") or "#888",
            "videoCount": len(video_ids),
            "totalDuration": _total_duration(video_ids),
            "videoIds": video_ids,
        })
    out.sort(key=lambda c: (c["name"] or "").lower())
    return {"categories": out}
