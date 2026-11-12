"""TCode browser serial relay — desktop app → backend → web remote tab → device.

A USB-serial TCode device plugged into another computer is driven through
the web remote open there: the tab opens the device with Web Serial and
attaches here as the `sink`; the desktop app's RemoteSerialTransport
(electron/tcode-transports.js) attaches as the `desktop` and sends its TCode
output, which goes to the sink verbatim. The relay is device-agnostic: it
never parses TCode and doesn't care about the baud rate beyond passing the
desktop's `config` on.

`/api/remote/tcode-relay?role=desktop|sink` (WebSocket):

- desktop: loopback client without an `Origin` header (Node's WebSocket
  sends none, every browser does, so no web page can drive the device).
  Text frames starting with `{` are JSON control (`config`, cached and
  passed to the sink); everything else is TCode. Receives `sink` status
  (right after accept and on every attach / detach / port change) and
  `reply` lines of device output.
- sink: loopback or private-network client whose `Origin` host:port equals
  the request's `Host` (a page served by this backend). Receives `config`
  and the raw TCode frames; sends `port` and `reply` (JSON).

One of each at a time, last wins (desktop: old one closed with 4001; sink:
old one gets `kicked`, then 4000). TCode arriving while no sink is attached
or its port isn't open is dropped, not queued, so a late sink never plays
stale motion. The per-sink outgoing queue is bounded (oldest dropped) and
drained by one sender task, so a slow browser never stalls the desktop.

State lives at module scope (process-level singleton), like remote_sync.
"""

import asyncio
import ipaddress
import json
import logging
import time
from collections import deque
from datetime import datetime, timezone
from urllib.parse import urlsplit

from fastapi import APIRouter, WebSocket, WebSocketDisconnect

router = APIRouter()
log = logging.getLogger("funsync.remote")

QUEUE_MAX = 64          # frames waiting for one sink
REPLY_MAX_CHARS = 256   # device output line, truncated
REPLY_MAX_PER_S = 20    # device output lines forwarded per second

CLOSE_SINK_REPLACED = 4000
CLOSE_DESKTOP_REPLACED = 4001

# Where a sink may connect from: loopback, RFC 1918, link-local, IPv6 ULA.
SINK_NETWORKS = tuple(ipaddress.ip_network(n) for n in (
    "127.0.0.0/8", "10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16",
    "169.254.0.0/16", "::1/128", "fc00::/7", "fe80::/10",
))


class _Sink:
    """The attached browser tab: its socket, device port and outgoing queue."""

    def __init__(self, ws: WebSocket, ip: str):
        self.ws = ws
        self.ip = ip
        self.port_open = False
        self.port: dict | None = None
        self.baud_rate: int | None = None
        self.queue: deque[tuple[str, bool]] = deque()  # (frame, is_tcode)
        self.wakeup = asyncio.Event()
        self.sender: asyncio.Task | None = None
        self.reply_window = 0.0
        self.reply_count = 0


# --- Module-level relay state ---------------------------------------------

_desktop: WebSocket | None = None
_sink: _Sink | None = None
_config: dict | None = None        # last `config` from the desktop
_since: float | None = None        # when the current sink state began
_forwarded = 0                      # TCode frames delivered to a sink
_dropped = 0                        # TCode frames dropped (no port, queue full)


def _iso(ts):
    if not ts:
        return None
    return datetime.fromtimestamp(ts, timezone.utc).isoformat(timespec="seconds")


def _client_ip(ws: WebSocket):
    """The client address as an ip_address (IPv4-mapped IPv6 unwrapped), or None."""
    try:
        ip = ipaddress.ip_address(ws.client.host if ws.client else "")
    except ValueError:
        return None
    if ip.version == 6 and ip.ipv4_mapped:
        return ip.ipv4_mapped
    return ip


def _desktop_allowed(ws: WebSocket) -> bool:
    ip = _client_ip(ws)
    return ip is not None and ip.is_loopback and "origin" not in ws.headers


def _sink_allowed(ws: WebSocket) -> bool:
    ip = _client_ip(ws)
    if ip is None or not any(ip in net for net in SINK_NETWORKS if net.version == ip.version):
        return False
    origin = ws.headers.get("origin")
    host = ws.headers.get("host")
    if not origin or not host:
        return False
    return urlsplit(origin).netloc.lower() == host.lower()


def _sink_status() -> dict:
    """The `sink` message (and the info endpoint's `sink` fields)."""
    s = _sink
    return {
        "type": "sink",
        "attached": s is not None,
        "ip": s.ip if s else None,
        "portOpen": bool(s and s.port_open),
        "port": s.port if s else None,
        "baudRate": s.baud_rate if s else None,
        "since": _iso(_since),
    }


async def _safe_send(ws: WebSocket | None, payload: dict) -> bool:
    if ws is None:
        return False
    try:
        await ws.send_text(json.dumps(payload))
        return True
    except Exception:
        return False


async def _safe_close(ws: WebSocket | None, code: int = 1000) -> None:
    if ws is None:
        return
    try:
        await ws.close(code=code)
    except Exception:
        pass


async def _notify_desktop_sink() -> None:
    await _safe_send(_desktop, _sink_status())


async def _receive_text(ws: WebSocket) -> str | None:
    """Next text frame; None for a binary frame. Raises WebSocketDisconnect."""
    message = await ws.receive()
    if message["type"] == "websocket.disconnect":
        raise WebSocketDisconnect(message.get("code", 1000))
    return message.get("text")


# --- Sink queue -------------------------------------------------------------

def _enqueue(sink: _Sink, frame: str, is_tcode: bool) -> None:
    """Queue a frame for the sink, dropping the oldest when full."""
    global _dropped
    if len(sink.queue) >= QUEUE_MAX:
        _, old_is_tcode = sink.queue.popleft()
        if old_is_tcode:
            _dropped += 1
    sink.queue.append((frame, is_tcode))
    sink.wakeup.set()


async def _drain(sink: _Sink) -> None:
    """The sink's single sender: writes queued frames in order until a send fails."""
    global _forwarded
    try:
        while True:
            await sink.wakeup.wait()
            sink.wakeup.clear()
            while sink.queue:
                frame, is_tcode = sink.queue.popleft()
                await sink.ws.send_text(frame)
                if is_tcode:
                    _forwarded += 1
    except asyncio.CancelledError:
        raise
    except Exception:
        pass  # socket gone; the sink's receive loop cleans up


def _stop_sender(sink: _Sink) -> None:
    if sink.sender is not None:
        sink.sender.cancel()
        sink.sender = None
    sink.queue.clear()


def _forward_tcode(frame: str) -> None:
    global _dropped
    if _sink is None or not _sink.port_open:
        _dropped += 1
        return
    _enqueue(_sink, frame, True)


# --- Desktop endpoint ---------------------------------------------------------

async def _desktop_session(ws: WebSocket) -> None:
    global _desktop, _config
    await ws.accept()
    if _desktop is not None and _desktop is not ws:
        old = _desktop
        _desktop = None  # so the old socket's cleanup doesn't clear the new one
        await _safe_close(old, code=CLOSE_DESKTOP_REPLACED)
        log.info("[tcode-relay] Desktop replaced by a newer connection")
    _desktop = ws
    log.info("[tcode-relay] Desktop attached")
    await _notify_desktop_sink()

    try:
        while True:
            frame = await _receive_text(ws)
            if not frame:
                continue
            if frame[0] != "{":
                _forward_tcode(frame)
                continue
            try:
                msg = json.loads(frame)
            except (ValueError, TypeError):
                continue
            if not isinstance(msg, dict) or msg.get("type") != "config":
                continue
            baud = msg.get("baudRate")
            if isinstance(baud, int) and not isinstance(baud, bool) and baud > 0:
                _config = {"type": "config", "baudRate": baud}
                if _sink is not None:
                    _enqueue(_sink, json.dumps(_config), False)
    except WebSocketDisconnect:
        pass
    except Exception as e:
        log.exception("[tcode-relay] desktop socket error: %s", e)
    finally:
        if _desktop is ws:
            _desktop = None
            log.info("[tcode-relay] Desktop detached (forwarded %d, dropped %d)", _forwarded, _dropped)
        await _safe_close(ws)


# --- Sink endpoint ----------------------------------------------------------------

def _port_info(info) -> dict | None:
    if not isinstance(info, dict):
        return None
    return {k: info[k] for k in ("usbVendorId", "usbProductId")
            if isinstance(info.get(k), int) and not isinstance(info.get(k), bool)}


async def _handle_port(sink: _Sink, msg: dict) -> None:
    global _since
    baud = msg.get("baudRate")
    new = (
        bool(msg.get("open")),
        _port_info(msg.get("info")),
        baud if isinstance(baud, int) and not isinstance(baud, bool) else None,
    )
    if new == (sink.port_open, sink.port, sink.baud_rate):
        return
    sink.port_open, sink.port, sink.baud_rate = new
    _since = time.time()
    if sink.port_open:
        usb = sink.port or {}
        log.info("[tcode-relay] Device port open on %s (USB %s:%s, %s baud)", sink.ip,
                 format(usb["usbVendorId"], "04x") if "usbVendorId" in usb else "?",
                 format(usb["usbProductId"], "04x") if "usbProductId" in usb else "?",
                 sink.baud_rate)
    else:
        log.info("[tcode-relay] Device port closed on %s (forwarded %d, dropped %d)", sink.ip, _forwarded, _dropped)
    await _notify_desktop_sink()


async def _handle_reply(sink: _Sink, msg: dict) -> None:
    text = msg.get("text")
    if not isinstance(text, str):
        return
    now = time.monotonic()
    if now - sink.reply_window >= 1.0:
        sink.reply_window = now
        sink.reply_count = 0
    if sink.reply_count >= REPLY_MAX_PER_S:
        return
    sink.reply_count += 1
    await _safe_send(_desktop, {"type": "reply", "text": text[:REPLY_MAX_CHARS]})


async def _sink_session(ws: WebSocket) -> None:
    global _sink, _since
    await ws.accept()
    sink = _Sink(ws, ws.client.host)

    if _sink is not None:
        old = _sink
        _sink = None  # so the old socket's cleanup doesn't clear the new one
        _stop_sender(old)
        await _safe_send(old.ws, {"type": "kicked"})
        await _safe_close(old.ws, code=CLOSE_SINK_REPLACED)
        log.info("[tcode-relay] Browser sink %s replaced by %s", old.ip, sink.ip)

    _sink = sink
    _since = time.time()
    sink.sender = asyncio.create_task(_drain(sink))
    if _config is not None:
        _enqueue(sink, json.dumps(_config), False)
    log.info("[tcode-relay] Browser sink attached from %s", sink.ip)
    await _notify_desktop_sink()

    try:
        while True:
            raw = await _receive_text(ws)
            try:
                msg = json.loads(raw) if raw else None
            except (ValueError, TypeError):
                continue
            if not isinstance(msg, dict):
                continue
            if msg.get("type") == "port":
                await _handle_port(sink, msg)
            elif msg.get("type") == "reply":
                await _handle_reply(sink, msg)
    except WebSocketDisconnect:
        pass
    except Exception as e:
        log.exception("[tcode-relay] sink socket error: %s", e)
    finally:
        _stop_sender(sink)
        if _sink is sink:
            _sink = None
            _since = time.time()
            log.info("[tcode-relay] Browser sink %s detached (forwarded %d, dropped %d)", sink.ip, _forwarded, _dropped)
            await _notify_desktop_sink()
        await _safe_close(ws)


@router.websocket("/tcode-relay")
async def tcode_relay(ws: WebSocket) -> None:
    role = ws.query_params.get("role")
    if role == "desktop" and _desktop_allowed(ws):
        await _desktop_session(ws)
    elif role == "sink" and _sink_allowed(ws):
        await _sink_session(ws)
    else:
        log.info("[tcode-relay] Refused %s connection from %s (origin %s)",
                 role, ws.client.host if ws.client else None, ws.headers.get("origin"))
        await ws.close(code=1008)  # before accept: the handshake gets HTTP 403


@router.get("/tcode-relay")
async def tcode_relay_info():
    sink = _sink_status() if _sink is not None else None
    if sink is not None:
        del sink["type"]
    return {"desktop": _desktop is not None, "sink": sink, "forwarded": _forwarded, "dropped": _dropped}


def relay_state() -> dict:
    """Relay links and counters, for /api/status."""
    return {
        "desktop": _desktop is not None,
        "sink": {k: v for k, v in _sink_status().items() if k != "type"},
        "config": _config,
        "forwarded": _forwarded,
        "dropped": _dropped,
    }


# --- Test-only helpers ----------------------------------------------------------

def _reset_state_for_tests() -> None:
    """Reset module state between tests. Not part of the public API. Runs
    after the test client's event loop is gone, so it only drops references."""
    global _desktop, _sink, _config, _since, _forwarded, _dropped
    _desktop = None
    _sink = None
    _config = None
    _since = None
    _forwarded = 0
    _dropped = 0
