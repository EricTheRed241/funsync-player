"""VR Content API — DeoVR and HereSphere compatible endpoints.

Serves FunSync's library in the JSON format that VR headset players expect.
DeoVR: GET /deovr (library), GET /deovr/{id} (scene detail)
HereSphere: GET /heresphere (library), GET|POST /heresphere/{id} (scene detail)

HereSphere is two-way for ratings, favourites and playlists: the scene sets
`writeRating` / `writeFavorite` / `writeTags`, and the edits HereSphere POSTs
back become library edits for the desktop (`services.library_edits`). The
request fields are the ones XBVR (`pkg/api/heresphere.go`,
`HereSphereAuthRequest`) and stash-vr (`internal/api/heresphere/http.go`,
`videoDataRequestDto`) read: `rating` (number, 0-5 in 0.5 steps),
`isFavorite` (bool) and `tags` (`[{"name", "start", "end", "track"}]`, the
full list). Only `Playlist:<name>` tags are acted on; every other tag edit
is ignored (the generated `Studio:` / `Format:` / `Category:` / `Tag:` ...
tags simply come back on the next load). `hsp` and `deleteFile` are ignored
too (`writeHSP` stays false; nothing is ever deleted from the headset).

Whether a video is VR follows the desktop first (VR Format panel
projection, then the manual toggle), then the filename, then the frame
shape (`detect_vr_format_for_video`); `GET /api/media/vr-flags` reports
that decision per video.
"""

import asyncio
import json
import logging
import os
import re
import time

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import JSONResponse
from starlette.responses import Response

from routes.media import get_categories, get_chip_tags, get_playlists, get_ratings, get_video_categories, get_video_registry, get_vr_projection, _path_to_id, record_vr_activity
from routes.remote_sync import submit_library_edit
from services import library_edits, video_info, vr_sections
from services.network import get_local_ip

router = APIRouter()
log = logging.getLogger("funsync.vr")

# --- VR Format Detection ---
#
# Matching strategy mirrors renderer/js/vr-detect.js: any common separator
# (`_ - . space / \ [ ] ( )`) is accepted around tokens so we catch dot-separated,
# bracketed, and space-delimited filenames — not just underscore-delimited ones.

# Character class matching any common filename separator.
SEP = r'[_\-. /\\\[\]()]'

VR_PATTERNS = [
    # 360° patterns (must be checked before 180° to avoid false matches)
    (rf'(?:^|{SEP})360(?:x180)?{SEP}(?:3dh|sbs|LR)(?={SEP}|$)', 'sphere', 'sbs'),
    (rf'(?:^|{SEP})360(?:x180)?{SEP}(?:3dv|TB)(?={SEP}|$)', 'sphere', 'tb'),
    (rf'(?:^|{SEP})(?:mono360|360{SEP}mono)(?={SEP}|$)', 'sphere', 'off'),
    # 180° TB (must be checked before generic 180° SBS)
    (rf'(?:^|{SEP})(?:180(?:x180)?{SEP}(?:3dv|TB)|TB{SEP}180|3dv)(?={SEP}|$)', 'dome', 'tb'),
    # 180° SBS
    (rf'(?:^|{SEP})(?:180(?:x180)?(?:{SEP}3dh)?(?:{SEP}LR)?|LR{SEP}180|3dh)(?={SEP}|$)', 'dome', 'sbs'),
    # 180° mono
    (rf'(?:^|{SEP})(?:mono180|180{SEP}mono)(?={SEP}|$)', 'dome', 'off'),
    # Fisheye types
    (rf'(?:^|{SEP})MKX200(?={SEP}|$)', 'mkx200', 'sbs'),
    (rf'(?:^|{SEP})MKX220(?={SEP}|$)', 'mkx220', 'sbs'),
    (rf'(?:^|{SEP})RF52(?={SEP}|$)', 'rf52', 'sbs'),
    (rf'(?:^|{SEP})FISHEYE190(?={SEP}|$)', 'fisheye', 'sbs'),
    (rf'(?:^|{SEP})VRCA220(?={SEP}|$)', 'fisheye', 'sbs'),
    (rf'(?:^|{SEP})FB360(?={SEP}|$)', 'sphere', 'sbs'),
    (rf'(?:^|{SEP})EAC360(?={SEP}|$)', 'sphere', 'sbs'),
    # Generic SBS/TB/LR/3DH/3DV (no FOV specified — assume 180)
    (rf'(?:^|{SEP})sbs(?={SEP}|$)', 'dome', 'sbs'),
    (rf'(?:^|{SEP})(?:tb|ou)(?={SEP}|$)', 'dome', 'tb'),
    (rf'(?:^|{SEP})lr(?={SEP}|$)', 'dome', 'sbs'),
    (rf'(?:^|{SEP})3dh(?={SEP}|$)', 'dome', 'sbs'),
    (rf'(?:^|{SEP})3dv(?={SEP}|$)', 'dome', 'tb'),
]

# Japanese VR studio catalog prefixes with per-studio projection defaults.
# Value: (screenType, stereoMode) — fed directly into detect_vr_format return.
#
# Most modern FANZA/DMM studios shoot with fisheye lenses (~200° FOV, MKX200-like).
# Older or Western-style studios tend to use equirectangular 180° SBS.
# Mapping based on the dominant format each label ships on DMM/FANZA.
#
# screenType key:
#   'mkx200'  → fisheye 200° SBS (most modern Japanese VR)
#   'fisheye' → generic fisheye SBS
#   'dome'    → equirectangular 180° SBS
VR_STUDIO_FORMATS = {
    # --- Major FANZA labels (fisheye ~200°) ---
    'SIVR':    ('mkx200', 'sbs'),   # S1 VR
    'KAVR':    ('mkx200', 'sbs'),   # Kawaii VR
    'SAVR':    ('mkx200', 'sbs'),   # SOD Create VR
    'DSVR':    ('mkx200', 'sbs'),   # Deeps VR
    'PRVR':    ('mkx200', 'sbs'),   # Premium/Faleno VR
    'IPVR':    ('mkx200', 'sbs'),   # IdeaPocket VR
    'MDVR':    ('mkx200', 'sbs'),   # MOODYZ VR
    'WAVR':    ('mkx200', 'sbs'),   # Wanz Factory VR
    'NHVR':    ('mkx200', 'sbs'),   # Natural High VR
    'EBVR':    ('mkx200', 'sbs'),   # E-Body VR
    'HNVR':    ('mkx200', 'sbs'),   # HonNaka VR
    'MTVR':    ('mkx200', 'sbs'),   # Muteki VR
    'ATVR':    ('mkx200', 'sbs'),   # Attackers VR
    'EXVR':    ('mkx200', 'sbs'),   # EX VR
    'WPVR':    ('mkx200', 'sbs'),   # Waap VR
    'PXVR':    ('mkx200', 'sbs'),   # Pixel VR
    'TMAVR':   ('mkx200', 'sbs'),   # TMA VR
    'FSVR':    ('mkx200', 'sbs'),   # First Star VR
    'UNVR':    ('mkx200', 'sbs'),   # Unfinished VR
    'DOVR':    ('mkx200', 'sbs'),   # Dogma VR
    'JUVR':    ('mkx200', 'sbs'),   # S-Cute VR
    'MXVR':    ('mkx200', 'sbs'),   # MAX-A VR
    # --- KMP group (fisheye) ---
    'KMVR':    ('mkx200', 'sbs'),   # KMP VR
    'VRKM':    ('mkx200', 'sbs'),   # V&R KMP
    'BIKMVR':  ('mkx200', 'sbs'),   # BIK VR
    'CBIKMVR': ('mkx200', 'sbs'),   # CBIK VR
    'KIWVR':   ('mkx200', 'sbs'),   # Kawaii (alt)
    # --- Misc fisheye ---
    'VRSP':    ('mkx200', 'sbs'),   # VR SP
    'URVRSP':  ('mkx200', 'sbs'),   # URER VR SP
    'AVOPVR':  ('mkx200', 'sbs'),   # AVO Premium VR
    'GOPJ':    ('mkx200', 'sbs'),   # GO Pro Japan
    # --- Caribbean/older equirect ---
    'CJVR':    ('dome', 'sbs'),     # Caribbean VR — equirectangular 180°

    # --- Western studios — equirectangular 180° SBS (the vast majority) ---
    # Sourced from XBVR scraper definitions, SLR scene metadata, and
    # studio sample filenames. Most Western VR is shot equirect 180 SBS.
    'WANKZVR':          ('dome', 'sbs'),  # WankzVR
    'NAVR':             ('dome', 'sbs'),  # NaughtyAmericaVR
    'NAUGHTYAMERICAVR': ('dome', 'sbs'),
    'BADOINKVR':        ('dome', 'sbs'),  # BadoinkVR
    'BAVR':             ('dome', 'sbs'),
    'MILFVR':           ('dome', 'sbs'),  # MilfVR (Badoink network)
    'POVR':             ('dome', 'sbs'),  # POVR / POVRfilms
    'SINSVR':           ('dome', 'sbs'),  # SinsVR
    'REALJAMVR':        ('dome', 'sbs'),  # RealJamVR
    'RJVR':             ('dome', 'sbs'),
    'CZECHVR':          ('dome', 'sbs'),  # CzechVR
    'CZECHVRCASTING':   ('dome', 'sbs'),
    'CZECHVRFETISH':    ('dome', 'sbs'),
    'CZECHVRNETWORK':   ('dome', 'sbs'),
    'TMWVRNET':         ('dome', 'sbs'),  # TmwVRnet (TeenMegaWorld)
    'VIRTUALREALPORN':  ('dome', 'sbs'),  # VRP
    'VRP':              ('dome', 'sbs'),
    'LETHALHARDCOREVR': ('dome', 'sbs'),
    'LHVR':             ('dome', 'sbs'),
    'SLRORIGINALS':     ('dome', 'sbs'),  # SLR Originals
    'DARKROOMVR':       ('dome', 'sbs'),
    'SWEETLIFEVR':      ('dome', 'sbs'),
    'HOLOGIRLSVR':      ('dome', 'sbs'),
    'STASYQVR':         ('dome', 'sbs'),
    'VRALLURE':         ('dome', 'sbs'),
    'VRHUSH':           ('dome', 'sbs'),
    'GROOBYVR':         ('dome', 'sbs'),  # GroobyVR — trans
    'GROVR':            ('dome', 'sbs'),  # Alt Grooby code
    'KINKVR':           ('dome', 'sbs'),
    '18VR':             ('dome', 'sbs'),  # 18VR (Badoink network)
    'EVILANGELVR':      ('dome', 'sbs'),
    'EAVR':             ('dome', 'sbs'),
    'METAVERSEVR':      ('dome', 'sbs'),
    'ZEXYVR':           ('dome', 'sbs'),
    'REALHOTVR':        ('dome', 'sbs'),
    'VRLATINA':         ('dome', 'sbs'),
    'PORNHATVR':        ('dome', 'sbs'),
    'COSPLAYBABESVR':   ('dome', 'sbs'),
    'VRTRANSTASTY':     ('dome', 'sbs'),  # VRBangers trans brand

    # --- Western studios — migrated to fisheye (~190–200°) ---
    # VRBangers announced their fisheye migration in 2022; SLR/HereSphere
    # auto-detect MKX200 on newer VRB scenes.
    'VRBANGERS':        ('mkx200', 'sbs'),
    'VRB':              ('mkx200', 'sbs'),  # Primary VRBangers code
    'VRCONK':           ('mkx200', 'sbs'),  # VRBangers network brand

    # --- Rip-group / obfuscated codes sometimes applied to Western
    # equirect content (GroobyVR / VRBTrans / SLR bundles). Mapped to
    # dome based on user testing — if you see these misbehave in future
    # dumps, override via a manual association.
    'VRBTS':            ('dome', 'sbs'),
    'VRBTNS':           ('dome', 'sbs'),
    'VRBS':             ('dome', 'sbs'),
    'VRBANS':           ('dome', 'sbs'),
}

# VR resolution / projection tags (VR180, 8KVR, VR7K, 180VR, ...)
_VR_TAG_RE = re.compile(
    rf'(?:^|{SEP})(?:VR\d{{2,4}}|\d{{2,4}}VR|VR\d+K|\d+KVR)(?={SEP}|$)',
    re.IGNORECASE,
)

# Broad XXVR-### / VRXX-### fallback for unlisted studios (any separator).
_VR_STUDIO_RE = re.compile(
    rf'(?:^|{SEP})(?:[A-Z]{{1,8}}VR[A-Z]{{0,3}}|VR[A-Z]{{1,4}}){SEP}*\d{{2,5}}',
    re.IGNORECASE,
)

# Bare "VR" as its own whitespace/punctuation-delimited token.
_BARE_VR_RE = re.compile(
    rf'(?:^|{SEP})VR(?={SEP}|$)',
    re.IGNORECASE,
)

# Token split (for studio-prefix lookup). Strips all separators.
_SEP_SPLIT_RE = re.compile(rf'{SEP}+')

# Studio code fused to a number: "SIVR178" (no separator).
_STUDIO_COMBO_RE = re.compile(r'^([A-Z]{2,8})(\d{2,5})$')

# VR Format panel projections (`library.vrFormat[path].projection`, see
# renderer/components/vr-format-panel.js) that name a geometry the headsets
# can play. The side-by-side planar ones (`sbs-half`, `sbs-full`) say only
# "stereo, side by side": they keep the filename's layout, or the default.
_PROJECTION_FORMATS = {
    'tb-half': ('dome', 'tb'),
    'tb-full': ('dome', 'tb'),
    'equirect-180': ('dome', 'sbs'),
    'equirect-360': ('sphere', 'sbs'),
    'fisheye-180': ('fisheye', 'sbs'),
    'fisheye-190': ('fisheye', 'sbs'),
    'fisheye-200': ('mkx200', 'sbs'),
    'mkx200': ('mkx200', 'sbs'),
    'rf52': ('rf52', 'sbs'),
}

# Layout for a video known to be VR whose filename doesn't say which.
_DEFAULT_VR_FORMAT = ('mkx200', 'sbs')


def _dims_suggest_vr(width, height):
    """Conservative resolution/aspect fallback for untagged VR files.

    A stereoscopic 180°/360° master is a full frame ~2:1 (each eye is
    roughly 1:1, placed side-by-side = 2:1) and always high-resolution
    (VR masters are 4K–8K wide). Requiring BOTH a near-2:1 aspect and a
    high width keeps ordinary 2D content out:
      - 16:9  (1.78:1) — excluded by aspect
      - 2.35:1 cinemascope — excluded by aspect
      - a 2:1 but low-res clip — excluded by the width floor
    Community-reported: a user's VR files had no filename tokens (no
    180/SBS/studio code) so they were served flat and HereSphere played
    them in 2D. This catches that case without misfiring on 2D libraries.
    """
    try:
        w, h = int(width), int(height)
    except (TypeError, ValueError):
        return False
    if w <= 0 or h <= 0:
        return False
    aspect = w / h
    return w >= 3200 and 1.9 <= aspect <= 2.1


def detect_vr_format_for_video(video, dims=None):
    """Return `(screenType, stereoMode, is3d)` for a registered video dict.

    The desktop's own decision comes first, in the renderer's order
    (`renderer/components/library.js`, VR override store):

      1. The VR Format panel's projection (`library.vrFormat`, registered
         as `vrFormat`): `'flat'` forces `('flat', 'off', False)`; any other
         projection makes the video VR, laid out per `_PROJECTION_FORMATS`,
         else as the filename says, else `_DEFAULT_VR_FORMAT`.
      2. The manual toggle (`video['manualVRType']`): `'flat'` forces flat;
         `'vr'` uses the filename's layout when it has one, else the
         default (`mkx200` SBS) so HereSphere has a projection to play with.
      3. Filename heuristics (`detect_vr_format`).

    `dims` is an optional `(width, height)` tuple. When the filename gives
    no VR signal AND the desktop decided nothing, a conservative
    resolution/aspect heuristic (`_dims_suggest_vr`) upgrades the video to
    VR SBS so an untagged VR master isn't served flat. Callers pass dims
    from cached ffprobe results — never triggering a probe just to group
    the library (see the library-listing loops, which use cached-only dims).
    """
    video = video or {}
    projection = get_vr_projection(video.get('path'))
    if projection == 'flat':
        return 'flat', 'off', False
    if projection in _PROJECTION_FORMATS:
        return (*_PROJECTION_FORMATS[projection], True)

    override = None if projection else video.get('manualVRType')
    if override == 'flat':
        return 'flat', 'off', False

    screen, stereo, is_3d = detect_vr_format(video.get('name', '') or '')
    if is_3d:
        return screen, stereo, is_3d

    if projection or override == 'vr':
        return (*_DEFAULT_VR_FORMAT, True)

    # Resolution/aspect fallback — only when filename said flat and the
    # user hasn't set an override either way. Matches the manual 'vr'
    # default projection (mkx200 SBS).
    if override is None and dims and _dims_suggest_vr(*dims):
        return (*_DEFAULT_VR_FORMAT, True)

    return screen, stereo, is_3d


def vr_decision(vid_id, video):
    """True / False when the backend knows whether the video is VR, None
    while that hinges on dimensions not probed yet (no desktop decision, no
    filename signal). Never probes."""
    dims = _dims_for_detection(vid_id, video)
    _, _, is_3d = detect_vr_format_for_video(video, dims=dims)
    if is_3d or dims is not None:
        return is_3d
    video = video or {}
    if get_vr_projection(video.get('path')) or video.get('manualVRType') is not None:
        return False
    return None


def vr_flags(registry):
    """`{"flags": {path: bool}, "pending": n}` for every registered video:
    the VR decision per path, leaving out (and counting as pending) the
    videos `vr_decision` can't decide yet. Their dimensions are being
    probed since registration (`queue_dims_warmup`), so asking again later
    settles them."""
    flags, pending = {}, 0
    for vid_id, video in registry.items():
        path = video.get('path')
        if not path:
            continue
        decision = vr_decision(vid_id, video)
        if decision is None:
            pending += 1
        else:
            flags[path] = decision
    return {'flags': flags, 'pending': pending}


def detect_vr_format(filename):
    """Detect VR projection and stereo mode from filename.

    Returns (screenType, stereoMode, is3d). Mirrors renderer/js/vr-detect.js —
    optimized for recall so the HereSphere/DeoVR VR group matches the library
    VR filter.

    Priority:
      1. Explicit projection/stereo tags (return accurate defaults)
      2. Known studio prefix — tokenised, separator-agnostic
      3. Broad XXVR-### / VRXX-### fallback
      4. VR resolution tags (VR180, 8KVR, ...)
      5. Bare "VR" token
    """
    name = filename or ''
    if not name:
        return 'flat', 'off', False

    # Strip trailing file extension so regex anchors work on the stem.
    stem = re.sub(r'\.[^./\\]+$', '', name)

    # 1. Explicit projection / stereo tags — most accurate defaults.
    for pattern, screen_type, stereo_mode in VR_PATTERNS:
        if re.search(pattern, stem, re.IGNORECASE):
            return screen_type, stereo_mode, stereo_mode != 'off'

    # 2. Known studio prefix — tokenise to catch both `SIVR-178` and `SIVR178`.
    for tok in _SEP_SPLIT_RE.split(stem):
        if not tok:
            continue
        upper = tok.upper()
        if upper in VR_STUDIO_FORMATS:
            screen_type, stereo_mode = VR_STUDIO_FORMATS[upper]
            return screen_type, stereo_mode, True
        m = _STUDIO_COMBO_RE.match(upper)
        if m and m.group(1) in VR_STUDIO_FORMATS:
            screen_type, stereo_mode = VR_STUDIO_FORMATS[m.group(1)]
            return screen_type, stereo_mode, True

    # 3. Broad XXVR-### / VRXX-### studio fallback.
    if _VR_STUDIO_RE.search(stem):
        return 'mkx200', 'sbs', True

    # 4. VR resolution / projection tags (VR180, 8KVR, ...).
    if _VR_TAG_RE.search(stem):
        return 'mkx200', 'sbs', True

    # 5. Bare "VR" as its own token — weakest signal.
    if _BARE_VR_RE.search(stem):
        return 'mkx200', 'sbs', True

    return 'flat', 'off', False


def _get_base_url(request: Request):
    """Build base URL from request for constructing media URLs."""
    host = request.headers.get('host', f'{get_local_ip()}:5123')
    scheme = 'https' if request.url.scheme == 'https' else 'http'
    return f'{scheme}://{host}'


def _get_duration(video):
    """Get video duration in seconds (from metadata or estimate)."""
    d = video.get('duration')
    if d and isinstance(d, (int, float)) and d > 0:
        return int(d)
    return 0


# Served when a file's resolution can't be read (ffprobe missing / failed).
_DEFAULT_VIDEO_INFO = (1920, 1080, 'h264')


async def _scene_video_info(video_id, path):
    """`(width, height, codec)` for a scene-detail request. Probes THIS
    video only (off the event loop) when nothing valid is cached, so VR
    auto-detect and the media block see real dimensions on first open —
    and never waits behind the registration warm-up."""
    info = video_info.cached_video_info(video_id)
    if info is None and path:
        info = await asyncio.to_thread(video_info.get_video_info, video_id, path)
    return info or _DEFAULT_VIDEO_INFO


def queue_dims_warmup(registry) -> int:
    """Make every registered video's dimensions known without anyone opening
    it (called at registration). Videos whose VR status hinges on their
    dimensions — no filename VR tag, no desktop decision (VR Format panel
    projection or manual toggle) — are probed first; the rest only feed the
    scene-detail media block. Returns the number of videos queued for
    probing."""
    first, later = [], []
    for vid_id, video in registry.items():
        if video.get('width') and video.get('height'):
            continue
        path = video.get('path')
        if not path:
            continue
        _, _, tagged = detect_vr_format_for_video(video)
        decided = tagged or video.get('manualVRType') or get_vr_projection(path)
        (later if decided else first).append((vid_id, path))
    return video_info.queue_warmup(first + later)


def _dims_for_detection(video_id, video):
    """Best-effort `(width, height)` for VR auto-detect WITHOUT probing.

    Prefers dimensions the scan already attached to the video dict, then
    the resolution cache — filled at registration from the persisted cache
    file and the background warm-up, and by scene-detail probes. Returns
    None when neither is available: the library-listing loops call this per
    video and must never probe (or even stat) a file.
    """
    w = (video or {}).get('width')
    h = (video or {}).get('height')
    if w and h:
        return int(w), int(h)
    cached = video_info.cached_video_info(video_id)
    if cached:
        return cached[0], cached[1]
    return None


def _filtered_videos(registry, vr_filter):
    """Yield `(vid_id, video, is_3d)` for the library listings, applying the
    `?filter=vr|flat|scripted` query param. Never probes or stats a file."""
    for vid_id, video in registry.items():
        _, _, is_3d = detect_vr_format_for_video(video, dims=_dims_for_detection(vid_id, video))
        if vr_filter == 'vr' and not is_3d:
            continue
        if vr_filter == 'flat' and is_3d:
            continue
        if vr_filter == 'scripted' and not video.get('funscriptPath'):
            continue
        yield vid_id, video, is_3d


def _scene_tag_names(video, is_3d, playlists):
    """Scene-detail tags, the same for HereSphere and DeoVR:
    `Studio:<source>`, `Feature:Is scripted`, the filter tags (`Format:`,
    `Speed:`, `Length:`, hand-made multi-axis), then `Playlist:<name>` per
    playlist holding the video (two-way in HereSphere), `Category:<name>`
    per desktop category and `Tag:<tag>` per chip tag (both read-only)."""
    tags = []
    path = video.get('path', '')
    source_name = video.get('sourceName') or os.path.basename(os.path.dirname(path))
    if source_name:
        tags.append(f'Studio:{source_name}')
    if video.get('hasFunscript'):
        tags.append('Feature:Is scripted')
    tags.extend(vr_sections.filter_tags(video, is_3d))
    tags.extend(vr_sections.playlist_tags(path, playlists))
    tags.extend(vr_sections.category_tags(path, get_categories(), get_video_categories()))
    tags.extend(vr_sections.chip_tag_names(get_chip_tags(path)))
    return tags


def parse_heresphere_update(body, path, ratings, playlists):
    """Library edits for one HereSphere scene POST body.

    HereSphere POSTs the scene URL both to load it (body: credentials only)
    and to save edits. A field counts only when present AND different from
    the current state, so a plain load, or a save that re-sends unchanged
    values, produces nothing:

      - `rating` / `isFavorite` → one `rating` edit with the changed fields.
        An invalid rating (off the 0.5 grid / out of range) is ignored.
      - `tags` (the full list) → `playlist-add` per `Playlist:<name>` tag
        not matching a playlist the video is in (case-insensitive; a new
        name creates the playlist), `playlist-remove` per playlist the video
        is in that has no tag. Any other tag is ignored.
    """
    if not isinstance(body, dict) or not path:
        return []
    edits = []
    current_rating, current_favorite = library_edits.rating_entry(ratings, path)

    rating = None
    if 'rating' in body and body['rating'] is not None:
        rating = library_edits.normalize_rating(body['rating'])
        if rating is None:
            log.warning('[heresphere] Ignoring invalid rating %r', body['rating'])
        elif rating == current_rating:
            rating = None
    favorite = body.get('isFavorite')
    if not isinstance(favorite, bool) or favorite == current_favorite:
        favorite = None
    if rating is not None or favorite is not None:
        edits.append(library_edits.make_rating_edit(path, rating=rating, favorite=favorite, source='heresphere'))

    tags = body.get('tags')
    if isinstance(tags, list):
        wanted = vr_sections.playlist_names_from_tags(tags)
        wanted_keys = {n.casefold() for n in wanted}
        member_of = [p for p in playlists or [] if path in (p.get('videoPaths') or [])]
        member_keys = {(p.get('name') or '').strip().casefold() for p in member_of}
        for name in wanted:
            if name.casefold() in member_keys:
                continue
            existing = library_edits.find_playlist(playlists or [], name=name)
            edits.append(library_edits.make_playlist_edit(
                library_edits.KIND_PLAYLIST_ADD, path,
                playlist_id=existing.get('id') if existing else None,
                name=existing.get('name') if existing else name,
                source='heresphere',
            ))
        for p in member_of:
            name = (p.get('name') or '').strip()
            if name and name.casefold() not in wanted_keys:
                edits.append(library_edits.make_playlist_edit(
                    library_edits.KIND_PLAYLIST_REMOVE, path,
                    playlist_id=p.get('id'), name=name, source='heresphere',
                ))
    return edits


async def _apply_heresphere_update(request: Request, path):
    """Turn a scene POST into library edits for the desktop. A body that
    isn't JSON, or a full edit queue, never fails the scene load."""
    try:
        body = await request.json()
    except Exception:
        return
    for edit in parse_heresphere_update(body, path, get_ratings(), get_playlists()):
        try:
            await submit_library_edit(edit)
        except library_edits.QueueFull as e:
            log.warning('[heresphere] Edit dropped: %s', e)
            return



# === DeoVR API ===

@router.api_route("/deovr", methods=["GET", "POST"])
async def deovr_library(request: Request):
    """DeoVR library listing — groups of scenes.

    Groups are the shared headset sections (`services.vr_sections`): VR /
    Flat, speed, length, hand-made multi-axis, recently added. Empty ones
    are left out.

    DeoVR sends both GET and POST to this endpoint.
    Query params:
      ?filter=vr    — only VR videos
      ?filter=flat  — only flat (non-VR) videos
      ?filter=scripted — only videos with funscripts
    """
    registry = get_video_registry()
    base_url = _get_base_url(request)
    entries = []
    for vid_id, video, is_3d in _filtered_videos(registry, request.query_params.get('filter', '')):
        item = {
            'title': os.path.splitext(video.get('name', ''))[0],
            'videoLength': _get_duration(video),
            'thumbnailUrl': f'{base_url}/api/media/thumb/{vid_id}',
            'video_url': f'{base_url}/deovr/{vid_id}',
        }
        entries.append((item, video, is_3d))
    entries.sort(key=lambda e: e[0]['title'].lower())

    sections = vr_sections.build_sections(entries, ratings=get_ratings(), playlists=get_playlists())
    scenes = [{'name': name, 'list': items} for name, items in sections]

    # DeoVR requires Content-Length (chunked encoding breaks its JSON parser)
    body = json.dumps({'scenes': scenes, 'authorized': '0'}, ensure_ascii=False)
    return Response(
        content=body,
        media_type='application/json',
        headers={
            'Access-Control-Allow-Origin': '*',
            'Content-Length': str(len(body.encode('utf-8'))),
        },
    )


@router.api_route("/deovr/{video_id}", methods=["GET", "POST"])
async def deovr_scene(video_id: str, request: Request):
    """DeoVR scene detail — full metadata for one video."""
    registry = get_video_registry()
    video = registry.get(video_id)
    if not video:
        raise HTTPException(status_code=404, detail="Scene not found")

    # Record VR activity for auto-connect companion bridge
    client_ip = request.client.host if request.client else None
    if client_ip and client_ip not in ('127.0.0.1', '::1', 'localhost'):
        record_vr_activity(client_ip, video_id)

    base_url = _get_base_url(request)
    name = video.get('name', '')
    title = os.path.splitext(name)[0]
    duration = _get_duration(video)
    path = video.get('path', '')

    width, height, codec = await _scene_video_info(video_id, path)

    screen_type, stereo_mode, is_3d = detect_vr_format_for_video(video, dims=(width, height))

    # Build encodings
    file_size = 0
    try:
        file_size = os.path.getsize(path)
    except OSError:
        pass

    size_label = f'{file_size / (1024**3):.1f} GB' if file_size > 1024**3 else f'{file_size / (1024**2):.0f} MB'

    encodings = [{
        'name': codec,
        'videoSources': [{
            'resolution': height,
            'height': height,
            'width': width,
            'size': file_size,
            'url': f'{base_url}/api/media/stream/{video_id}',
        }],
    }]

    # Build funscript array
    fleshlight = []
    fs_path = video.get('funscriptPath')
    if fs_path:
        fs_name = os.path.basename(fs_path)
        fleshlight.append({
            'title': fs_name,
            'url': f'{base_url}/api/media/script/{video_id}',
        })

    # Auto-generate chapter timestamps every 5 minutes for navigation
    timestamps = []
    if duration > 300:  # only for videos > 5 min
        interval = 300  # 5 minutes
        t = interval
        while t < duration:
            mins = int(t // 60)
            timestamps.append({'ts': int(t), 'name': f'{mins} min'})
            t += interval

    scene = {
        'id': video_id,
        'title': title,
        'authorized': 1,
        'description': '',
        'date': int(time.time()),
        'is3d': is_3d,
        'screenType': screen_type,
        'stereoMode': stereo_mode,
        'videoLength': duration,
        'thumbnailUrl': f'{base_url}/api/media/thumb/{video_id}',
        'encodings': encodings,
        'fleshlight': fleshlight,
        'timeStamps': timestamps,
        'isScripted': bool(fs_path),
        'categories': [
            {'tag': {'id': i, 'name': name}}
            for i, name in enumerate(_scene_tag_names(video, is_3d, get_playlists()), start=1)
        ],
        'isFavorite': library_edits.rating_entry(get_ratings(), path)[1],
        'fullVideoReady': True,
        'fullAccess': True,
    }

    body = json.dumps(scene, ensure_ascii=False)
    return Response(
        content=body,
        media_type='application/json',
        headers={
            'Access-Control-Allow-Origin': '*',
            'Content-Length': str(len(body.encode('utf-8'))),
        },
    )


# === HereSphere API ===

@router.api_route("/heresphere", methods=["GET", "POST"])
async def heresphere_library(request: Request):
    """HereSphere library listing.

    Sections are the shared headset sections (`services.vr_sections`), in
    plain text — HereSphere can't render emoji. Empty ones are left out.

    Query params:
      ?filter=vr    — only VR videos
      ?filter=flat  — only flat (non-VR) videos
      ?filter=scripted — only videos with funscripts
    """
    registry = get_video_registry()
    base_url = _get_base_url(request)
    entries = [
        (f'{base_url}/heresphere/{vid_id}', video, is_3d)
        for vid_id, video, is_3d in _filtered_videos(registry, request.query_params.get('filter', ''))
    ]
    entries.sort(key=lambda e: os.path.splitext(e[1].get('name', ''))[0].lower())

    library = [
        {'name': name, 'list': urls}
        for name, urls in vr_sections.build_sections(entries, ratings=get_ratings(), playlists=get_playlists())
    ]

    return JSONResponse(
        content={'access': 1, 'library': library},
        headers={
            'Access-Control-Allow-Origin': '*',
            'HereSphere-JSON-Version': '1',
        },
    )


@router.api_route("/heresphere/{video_id}", methods=["GET", "POST"])
async def heresphere_scene(video_id: str, request: Request):
    """HereSphere scene detail."""
    registry = get_video_registry()
    video = registry.get(video_id)
    if not video:
        raise HTTPException(status_code=404, detail="Scene not found")

    # Record VR activity for auto-connect companion bridge
    client_ip = request.client.host if request.client else None
    if client_ip and client_ip not in ('127.0.0.1', '::1', 'localhost'):
        record_vr_activity(client_ip, video_id)

    base_url = _get_base_url(request)
    name = video.get('name', '')
    title = os.path.splitext(name)[0]
    duration = _get_duration(video)
    path = video.get('path', '')

    # Ratings, favourites and playlist tags edited in the headset.
    if request.method == 'POST':
        await _apply_heresphere_update(request, path)
    rating, favorite = library_edits.rating_entry(get_ratings(), path)
    playlists = get_playlists()

    width, height, codec = await _scene_video_info(video_id, path)

    screen_type, stereo_mode, is_3d = detect_vr_format_for_video(video, dims=(width, height))

    # Map DeoVR screenType to HereSphere projection
    projection_map = {
        'dome': 'equirectangular',
        'sphere': 'equirectangular360',
        'mkx200': 'fisheye',
        'mkx220': 'fisheye',
        'rf52': 'fisheye',
        'fisheye': 'fisheye',
        'flat': 'perspective',
    }
    projection = projection_map.get(screen_type, 'perspective')

    # Lens for fisheye types
    lens_map = {'mkx200': 'MKX200', 'mkx220': 'MKX220', 'rf52': 'RF52', 'fisheye': 'Linear'}
    lens = lens_map.get(screen_type, 'Linear')

    # FOV
    fov = 180.0 if screen_type in ('dome', 'mkx200', 'mkx220', 'rf52', 'fisheye') else 360.0 if screen_type == 'sphere' else 0.0

    # File info
    file_size = 0
    try:
        file_size = os.path.getsize(path)
    except OSError:
        pass

    size_label = f'{file_size / (1024**3):.1f} GB' if file_size > 1024**3 else f'{file_size / (1024**2):.0f} MB'

    # Media sources
    media = [{
        'name': f'{height}p - {size_label}',
        'sources': [{
            'resolution': str(height),
            'height': height,
            'width': width,
            'size': file_size,
            'url': f'{base_url}/api/media/stream/{video_id}',
        }],
    }]

    # Scripts
    scripts = []
    fs_path = video.get('funscriptPath')
    if fs_path:
        fs_name = os.path.basename(fs_path)
        scripts.append({
            'name': fs_name,
            'url': f'{base_url}/api/media/script/{video_id}',
        })

    # Tags: studio, scripted, filter tags, then Playlist: (removing one in
    # the headset takes the video out of that playlist, adding
    # `Playlist:<name>` puts it in, creating it), Category: and Tag: (both
    # read-only); chapters follow.
    tags = [{'name': name} for name in _scene_tag_names(video, is_3d, playlists)]

    # Auto-generate chapter tags for navigation (every 5 min)
    if duration > 300:
        interval = 300
        t = interval
        while t < duration:
            mins = int(t // 60)
            tags.append({
                'name': f'{mins} min',
                'start': int(t * 1000),
                'end': int(min(t + interval, duration) * 1000),
                'track': 0,
            })
            t += interval

    # Subtitles
    subtitles = []
    sub_path = video.get('subtitlePath')
    if sub_path:
        sub_name = os.path.basename(sub_path)
        sub_ext = os.path.splitext(sub_name)[1].lower()
        subtitles.append({
            'name': sub_name,
            'language': 'English',
            'url': f'{base_url}/api/media/subtitle/{video_id}',
        })

    stereo_hs = 'sbs' if stereo_mode == 'sbs' else 'tb' if stereo_mode == 'tb' else 'mono'

    scene = {
        'access': 1,
        'title': title,
        'description': '',
        'thumbnailImage': f'{base_url}/api/media/thumb/{video_id}',
        'dateReleased': '',
        'dateAdded': '',
        'duration': duration * 1000,  # HereSphere uses milliseconds
        'rating': rating,
        'isFavorite': favorite,
        'projection': projection,
        'stereo': stereo_hs,
        'fov': fov,
        'lens': lens,
        'scripts': scripts,
        'subtitles': subtitles,
        'tags': tags,
        'media': media,
        'writeFavorite': True,
        'writeRating': True,
        'writeTags': True,
        'writeHSP': False,
    }

    return JSONResponse(
        content=scene,
        headers={
            'Access-Control-Allow-Origin': '*',
            'HereSphere-JSON-Version': '1',
        },
    )
