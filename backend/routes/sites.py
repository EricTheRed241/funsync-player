"""The local sites on the backend's own port (deploy/sites.json).

The desktop app runs each Rust site on 127.0.0.1:<port> (electron/sites.js); this
module relays `<prefix>/…` to it, so every page is reachable on one port behind the
access PIN (routes/access.py runs first; the sites themselves see loopback requests,
which they leave open):

  - `<prefix>` redirects to `<prefix>/`, which serves the site's `home` page;
  - `<prefix>/<rest>` is the site's `/<rest>`, query included;
  - a `Location` the site answers with (`/x`) comes back as `<prefix>/x`
    (`<prefix>/` for its home);
  - HTML pages get `<base href="<prefix>/">` (their own URLs are relative, so they
    resolve under the prefix) and the nav bar's script right after `<head>`.

`GET /site-nav.js` is the nav bar every page loads (the web remote too): links to the
web remote and each site, the current one marked.
"""

import json
import os
import re

import httpx
from fastapi import APIRouter, Request
from fastapi.responses import HTMLResponse, RedirectResponse, Response

router = APIRouter()

_ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
SITES_FILE = os.path.join(_ROOT, "deploy", "sites.json")
NAV_SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "site-nav.js")

# Request headers passed on, and answer headers passed back.
_REQUEST_HEADERS = ("accept", "content-type", "if-none-match", "if-modified-since", "range")
_ANSWER_HEADERS = ("content-type", "cache-control", "etag", "last-modified", "retry-after",
                   "content-range", "accept-ranges")
_HEAD = re.compile(rb"<head[^>]*>", re.IGNORECASE)

# A deck is rebuilt from the scouts' files per request and a frame is taken with ffmpeg
# on first view: a generous read timeout, a short connect one (down = not running).
_client = httpx.AsyncClient(timeout=httpx.Timeout(60.0, connect=2.0), trust_env=False)


def load_sites(path: str = SITES_FILE) -> list[dict]:
    with open(path, encoding="utf-8") as f:
        return json.load(f)["sites"]


def upstream_path(site: dict, rest: str) -> str:
    """The site's path for `<prefix>/<rest>`: its home for the bare prefix."""
    return site["home"] if rest == "" else "/" + rest


def outside_location(site: dict, location: str) -> str:
    """A `Location` from the site as the browser must follow it."""
    if not location.startswith("/") or location.startswith("//"):
        return location
    path, sep, query = location.partition("?")
    path = "/" if path == site["home"] else path
    return f"{site['prefix']}{path}{sep}{query}"


def inject(site: dict, page: bytes) -> bytes:
    """`<base>` and the nav bar script right after `<head>` (unchanged without one)."""
    tag = (f'<base href="{site["prefix"]}/">'
           '<script src="/site-nav.js" defer></script>').encode()
    return _HEAD.sub(lambda m: m.group(0) + tag, page, count=1)


def _down_page(site: dict, why: str) -> HTMLResponse:
    return HTMLResponse(
        "<!doctype html><html><head><meta charset=utf-8>"
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>{site['name']}</title><script src=\"/site-nav.js\" defer></script></head>"
        "<body style=\"font:15px -apple-system,sans-serif;padding:24px\">"
        f"<p><b>{site['name']}</b> isn't answering ({why}).</p>"
        "<p>The desktop app runs it and starts it again within seconds; if this stays, "
        "restart the app.</p></body></html>",
        status_code=502,
    )


async def relay(site: dict, rest: str, request: Request) -> Response:
    url = f"http://127.0.0.1:{site['port']}{upstream_path(site, rest)}"
    if request.url.query:
        url += "?" + request.url.query
    headers = {k: v for k, v in request.headers.items() if k.lower() in _REQUEST_HEADERS}
    body = await request.body() if request.method not in ("GET", "HEAD") else None
    try:
        answer = await _client.request(request.method, url, headers=headers, content=body)
    except httpx.HTTPError as e:
        why = "not running" if isinstance(e, httpx.ConnectError) else type(e).__name__
        if "text/html" in request.headers.get("accept", ""):
            return _down_page(site, why)
        return Response(f"{site['name']} isn't answering ({why})\n", status_code=502,
                        media_type="text/plain")
    out = {k: v for k, v in answer.headers.items() if k.lower() in _ANSWER_HEADERS}
    if "location" in answer.headers:
        out["location"] = outside_location(site, answer.headers["location"])
    content = answer.content
    if answer.headers.get("content-type", "").startswith("text/html"):
        content = inject(site, content)
    return Response(content=content, status_code=answer.status_code, headers=out)


def nav_script(sites: list[dict]) -> str:
    links = [{"name": "Remote", "href": "/remote/"}] + [
        {"name": s["name"], "href": s["prefix"] + "/"} for s in sites]
    with open(NAV_SCRIPT, encoding="utf-8") as f:
        return f"window.FUNSYNC_NAV = {json.dumps(links)};\n" + f.read()


def _add_site(site: dict) -> None:
    prefix = site["prefix"]

    @router.api_route(prefix, methods=["GET", "HEAD"], include_in_schema=False)
    async def bare(request: Request):
        query = f"?{request.url.query}" if request.url.query else ""
        return RedirectResponse(f"{prefix}/{query}", status_code=307)

    @router.api_route(prefix + "/{rest:path}", methods=["GET", "HEAD", "POST"], include_in_schema=False)
    async def proxied(rest: str, request: Request):
        return await relay(site, rest, request)


SITES = load_sites() if os.path.isfile(SITES_FILE) else []
for _site in SITES:
    _add_site(_site)


@router.get("/site-nav.js", include_in_schema=False)
async def site_nav():
    return Response(nav_script(SITES), media_type="text/javascript",
                    headers={"Cache-Control": "no-cache"})
