"""Access PIN gate, login page and headset login (services/access_pin.py).

With a PIN set, a request from another device needs a valid token (cookie
`funsync_access` or query `k`). Without one:
  - page requests (GET of `/`, `/remote…`, or anything asking for HTML) get
    303 to `/login?next=<path>`;
  - everything else gets 401 `{"error": "pin required"}`;
  - WebSockets are accepted and closed at once with code 4401.
`/login` and `/logout` are always open. The HereSphere and DeoVR library and
scene routes do their own login (`headset_access`): those players send the
password in the request body and show a login prompt on `access: 0` /
`authorized: "0"`.
"""

import html
import json
from dataclasses import dataclass
from urllib.parse import parse_qs, quote

from fastapi import APIRouter, Request
from fastapi.responses import HTMLResponse, Response
from starlette.requests import HTTPConnection

from services import access_pin

router = APIRouter()

OPEN_PATHS = frozenset({"/login", "/logout"})
# Library / scene routes of the headset players; they check the body login.
_HEADSET_ROOTS = ("heresphere", "deovr")

COOKIE_ATTRS = f"Path=/; Max-Age={access_pin.TOKEN_LIFETIME}; HttpOnly; SameSite=Lax"
INVALID_FILE_MESSAGE = (
    "The access PIN file on the FunSync computer is unreadable, so other devices are locked out. "
    "Set the PIN again in FunSync's settings."
)


def _is_headset_route(path: str) -> bool:
    parts = path.strip("/").split("/")
    return parts[0] in _HEADSET_ROOTS and len(parts) <= 2 and not (len(parts) == 2 and parts[1] == "events")


def _is_page_request(method: str, path: str, accept: str) -> bool:
    if method not in ("GET", "HEAD"):
        return False
    return path == "/" or path == "/remote" or path.startswith("/remote/") or "text/html" in accept


def safe_next(value: str | None) -> str:
    """Only a same-site relative path of printable ASCII without spaces;
    anything else (or the login pages themselves) becomes '/'."""
    if (
        not value
        or not value.startswith("/")
        or value.startswith("//")
        or value.startswith("/\\")
        or any(not ("!" <= c <= "~") for c in value)
        or value.split("?", 1)[0] in OPEN_PATHS
    ):
        return "/"
    return value


def login_url(next_path: str) -> str:
    return f"/login?next={quote(safe_next(next_path), safe='')}"


def _has_valid_token(conn: HTTPConnection, secret: bytes) -> bool:
    return (
        access_pin.verify_token(secret, conn.cookies.get(access_pin.COOKIE_NAME))
        or access_pin.verify_token(secret, conn.query_params.get(access_pin.QUERY_KEY))
    )


def _thumb_key_fits(conn: HTTPConnection, secret: bytes) -> bool:
    """A headset's thumbnail-only `k` (access_pin.make_thumb_token) on a thumbnail GET."""
    return (
        conn.scope["type"] == "http"
        and conn.scope["method"] in ("GET", "HEAD")
        and conn.url.path.startswith(access_pin.THUMB_PATH_PREFIX)
        and access_pin.verify_thumb_token(secret, conn.query_params.get(access_pin.QUERY_KEY))
    )


def _client_ip(conn: HTTPConnection) -> str:
    return conn.client.host if conn.client else ""


class AccessGate:
    """ASGI middleware: the PIN gate for HTTP and WebSocket requests."""

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope["type"] not in ("http", "websocket"):
            return await self.app(scope, receive, send)
        conn = HTTPConnection(scope)
        if access_pin.is_loopback(_client_ip(conn)) or conn.url.path in OPEN_PATHS:
            return await self.app(scope, receive, send)
        try:
            pin = access_pin.load()
        except access_pin.PinFileError:
            pin = None
            invalid = True
        else:
            invalid = False
            if pin is None:
                return await self.app(scope, receive, send)
            if _has_valid_token(conn, pin.secret) or _thumb_key_fits(conn, pin.secret):
                return await self.app(scope, receive, send)
        path = conn.url.path
        if scope["type"] == "http" and _is_headset_route(path):
            return await self.app(scope, receive, send)

        if scope["type"] == "websocket":
            await send({"type": "websocket.accept"})
            await send({"type": "websocket.close", "code": 4401, "reason": "pin required"})
            return

        method = scope["method"]
        if invalid:
            if _is_page_request(method, path, conn.headers.get("accept", "")):
                response: Response = _page(INVALID_FILE_MESSAGE, status=503)
            else:
                response = _json({"error": "access pin file unreadable", "message": INVALID_FILE_MESSAGE}, 503)
        elif _is_page_request(method, path, conn.headers.get("accept", "")):
            target = path + (f"?{conn.url.query}" if conn.url.query else "")
            response = Response(status_code=303, headers={"Location": login_url(target), "Cache-Control": "no-store"})
        else:
            response = _json({"error": "pin required"}, 401)
        await response(scope, receive, send)


def _json(body: dict, status: int) -> Response:
    return Response(json.dumps(body), status_code=status, media_type="application/json",
                    headers={"Cache-Control": "no-store"})


_PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark light">
<meta name="robots" content="noindex">
<title>FunSync</title>
<style>
:root {{ color-scheme: dark light; --bg: #1a1a2e; --card: #24243e; --fg: #eee; --muted: #9a9ab0; --accent: #7c5cff; --err: #ff6b6b; }}
@media (prefers-color-scheme: light) {{ :root {{ --bg: #f2f2f7; --card: #fff; --fg: #1a1a2e; --muted: #666; --err: #c62828; }} }}
* {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: flex; align-items: center; justify-content: center;
  background: var(--bg); color: var(--fg); font: 16px/1.4 system-ui, -apple-system, sans-serif; padding: 16px; }}
main {{ background: var(--card); border-radius: 16px; padding: 28px 24px; width: 100%; max-width: 340px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, .25); text-align: center; }}
h1 {{ margin: 0 0 4px; font-size: 1.5rem; }}
p {{ margin: 0 0 20px; color: var(--muted); }}
p.err {{ color: var(--err); }}
input, button {{ width: 100%; font: inherit; border-radius: 10px; padding: 14px; }}
input {{ border: 1px solid var(--muted); background: transparent; color: var(--fg); font-size: 1.6rem;
  letter-spacing: .4em; text-align: center; margin-bottom: 14px; }}
button {{ border: 0; background: var(--accent); color: #fff; font-weight: 600; cursor: pointer; }}
</style>
</head>
<body>
<main>
<h1>FunSync</h1>
{body}
</main>
</body>
</html>
"""

_FORM = """<p{cls}>{message}</p>
<form method="post" action="/login">
<input type="hidden" name="next" value="{next}">
<input name="pin" type="password" inputmode="numeric" pattern="[0-9]*" minlength="4" maxlength="12"
 autocomplete="current-password" aria-label="PIN" required autofocus>
<button type="submit">Unlock</button>
</form>
"""


def _page(message: str, status: int = 200, form_next: str | None = None, error: bool = False,
          headers: dict | None = None) -> HTMLResponse:
    if form_next is None:
        body = f'<p class="err">{html.escape(message)}</p>'
    else:
        body = _FORM.format(cls=' class="err"' if error else "", message=html.escape(message),
                            next=html.escape(form_next, quote=True))
    return HTMLResponse(_PAGE.format(body=body), status_code=status,
                        headers={"Cache-Control": "no-store", **(headers or {})})


def _too_many(retry_after: float, next_path: str) -> HTMLResponse:
    minutes = max(1, int(-(-retry_after // 60)))
    return _page(f"Too many wrong PINs. Try again in {minutes} minute{'s' if minutes != 1 else ''}.",
                 status=429, form_next=next_path, error=True,
                 headers={"Retry-After": str(int(retry_after) + 1)})


def _redirect(location: str, cookie: str | None = None) -> Response:
    headers = {"Location": location, "Cache-Control": "no-store"}
    if cookie is not None:
        headers["Set-Cookie"] = cookie
    return Response(status_code=303, headers=headers)


@router.get("/login", include_in_schema=False)
async def login_page(request: Request, next: str = "/"):
    target = safe_next(next)
    try:
        pin = access_pin.load()
    except access_pin.PinFileError:
        return _page(INVALID_FILE_MESSAGE, status=503)
    if pin is None:
        return _redirect(target)
    return _page("Enter the PIN to use FunSync on this device.", form_next=target)


async def _form(request: Request) -> dict[str, str]:
    raw = await request.body()
    fields = parse_qs(raw.decode("utf-8", "replace"), keep_blank_values=True)
    return {k: v[0] for k, v in fields.items()}


@router.post("/login", include_in_schema=False)
async def login_submit(request: Request):
    form = await _form(request)
    target = safe_next(form.get("next"))
    try:
        pin = access_pin.load()
    except access_pin.PinFileError:
        return _page(INVALID_FILE_MESSAGE, status=503)
    if pin is None:
        return _redirect(target)
    ip = _client_ip(request)
    wait = access_pin.limiter.retry_after(ip)
    if wait > 0:
        return _too_many(wait, target)
    if access_pin.pin_matches(pin, form.get("pin", "").strip()):
        access_pin.limiter.reset(ip)
        token = access_pin.make_token(pin.secret)
        return _redirect(target, f"{access_pin.COOKIE_NAME}={token}; {COOKIE_ATTRS}")
    access_pin.limiter.record_failure(ip)
    return _page("Wrong PIN", status=401, form_next=target, error=True)


@router.post("/logout", include_in_schema=False)
async def logout():
    return _redirect("/", f"{access_pin.COOKIE_NAME}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax")


# --- Headset players --------------------------------------------------------

@dataclass(frozen=True)
class HeadsetAccess:
    """`state`: 1 allowed, 0 login required, -1 login failed. `token`: the
    `k` to append to media URLs, `thumb_token` the one for thumbnail URLs
    (both None when no PIN or on this computer)."""
    state: int
    token: str | None = None
    thumb_token: str | None = None

    @property
    def message(self) -> str:
        return "Login Failed" if self.state == -1 else "Login Required"


def headset_access(request: Request, password: str | None) -> HeadsetAccess:
    """Login for HereSphere / DeoVR: any username, password = the PIN, or a
    valid cookie / `k`. Wrong passwords count toward the login rate limit."""
    ip = _client_ip(request)
    if access_pin.is_loopback(ip):
        return HeadsetAccess(1)
    try:
        pin = access_pin.load()
    except access_pin.PinFileError:
        return HeadsetAccess(-1)
    if pin is None:
        return HeadsetAccess(1)
    if _has_valid_token(request, pin.secret):
        return _headset_allowed(pin.secret)
    if not password:
        return HeadsetAccess(0)
    if access_pin.limiter.retry_after(ip) > 0:
        return HeadsetAccess(-1)
    if access_pin.pin_matches(pin, password.strip()):
        access_pin.limiter.reset(ip)
        return _headset_allowed(pin.secret)
    access_pin.limiter.record_failure(ip)
    return HeadsetAccess(-1)


def _headset_allowed(secret: bytes) -> HeadsetAccess:
    return HeadsetAccess(1, access_pin.make_media_token(secret), access_pin.make_thumb_token(secret))
