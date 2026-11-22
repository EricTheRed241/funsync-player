"""Access PIN for devices on the home network.

`<userData>/access-pin.json` (written by the desktop, mode 0600):
`{"version": 1, "pin": "<4-12 digits>", "secret": "<64 hex>", "changedAt": "<ISO>"}`.
No file: no PIN, everything is open. A file that can't be read or parsed
fails closed: every non-loopback request is refused until it's fixed.

A device that entered the PIN holds a token, `v1.<expiry>.<hex HMAC-SHA256
(secret, "v1.<expiry>")>`, valid for 30 days, in the `funsync_access`
cookie or (headsets fetching media) the `k` query parameter. Changing the
PIN regenerates the secret, which logs every device out. The Rust server
(port 5131) reads the same file and accepts the same tokens, so one login
unlocks both ports.

Thumbnail URLs handed to headsets carry a thumbnail-only key instead,
`t1.<hex HMAC-SHA256(secret, "t1.thumbs")>` (make_thumb_token): it never
expires and opens `GET /api/media/thumb/…` and nothing else, so those URLs
stay the same until the PIN changes and the headset's thumbnail cache keeps
working (a media token's URLs change every day).

Requests from this computer (127.0.0.1, ::1) never need the PIN: the
desktop app, the CLIs and the scheduled jobs.
"""

import hashlib
import hmac
import json
import logging
import os
import re
import threading
import time
from collections import deque
from dataclasses import dataclass

log = logging.getLogger("funsync.access_pin")

PIN_FILE_NAME = "access-pin.json"
COOKIE_NAME = "funsync_access"
QUERY_KEY = "k"
TOKEN_LIFETIME = 30 * 24 * 3600
MAX_FAILURES = 5
FAILURE_WINDOW = 10 * 60
LOOPBACK_HOSTS = frozenset({"127.0.0.1", "::1", "::ffff:127.0.0.1"})

_PIN_RE = re.compile(r"[0-9]{4,12}")
_SECRET_RE = re.compile(r"[0-9a-f]{64}")
_TOKEN_RE = re.compile(r"v1\.([0-9]{1,12})\.([0-9a-f]{64})")


class PinFileError(Exception):
    """access-pin.json exists but can't be used: refuse non-loopback access."""


@dataclass(frozen=True)
class AccessPin:
    pin: str
    secret: bytes


_user_data_dir: str | None = None
_lock = threading.Lock()
# (stat key, AccessPin | None | PinFileError) of the last load.
_cached: tuple[tuple, object] | None = None


def set_user_data_dir(path: str | None) -> None:
    global _user_data_dir, _cached
    with _lock:
        _user_data_dir = path
        _cached = None


def pin_file_path() -> str | None:
    return os.path.join(_user_data_dir, PIN_FILE_NAME) if _user_data_dir else None


def _parse(raw: bytes) -> AccessPin:
    try:
        data = json.loads(raw)
    except ValueError as e:
        raise PinFileError(f"not JSON: {e}") from None
    if not isinstance(data, dict) or data.get("version") != 1:
        raise PinFileError("unsupported version")
    pin, secret = data.get("pin"), data.get("secret")
    if not isinstance(pin, str) or not _PIN_RE.fullmatch(pin):
        raise PinFileError("pin must be 4-12 digits")
    if not isinstance(secret, str) or not _SECRET_RE.fullmatch(secret):
        raise PinFileError("secret must be 64 lowercase hex digits")
    return AccessPin(pin=pin, secret=bytes.fromhex(secret))


def load() -> AccessPin | None:
    """The current PIN, None when there is none. Raises PinFileError when the
    file exists but is unusable. Re-reads the file whenever it changes."""
    global _cached
    path = pin_file_path()
    if path is None:
        return None
    try:
        st = os.stat(path)
        key = (st.st_mtime_ns, st.st_size, st.st_ino)
    except FileNotFoundError:
        key = ("missing",)
    except OSError as e:
        key = ("stat-error", str(e))
    with _lock:
        if _cached is None or _cached[0] != key:
            if key == ("missing",):
                result: object = None
            elif key[0] == "stat-error":
                result = PinFileError(f"can't read {path}: {key[1]}")
            else:
                try:
                    with open(path, "rb") as f:
                        result = _parse(f.read())
                except OSError as e:
                    result = PinFileError(f"can't read {path}: {e}")
                except PinFileError as e:
                    result = PinFileError(f"{path}: {e}")
            if isinstance(result, PinFileError):
                log.error("Access PIN file unusable, refusing other devices until it's fixed: %s", result)
            elif _cached is not None:
                log.info("Access PIN %s", "set" if result else "removed")
            _cached = (key, result)
        result = _cached[1]
    if isinstance(result, PinFileError):
        raise result
    return result  # type: ignore[return-value]


def is_loopback(host: str | None) -> bool:
    return host in LOOPBACK_HOSTS


def pin_matches(access: AccessPin, candidate: str) -> bool:
    return hmac.compare_digest(candidate.encode("utf-8"), access.pin.encode("utf-8"))


def _mac(secret: bytes, expiry: int) -> str:
    return hmac.new(secret, f"v1.{expiry}".encode("ascii"), hashlib.sha256).hexdigest()


def make_token(secret: bytes, now: float | None = None, expiry: int | None = None) -> str:
    """A login token valid for TOKEN_LIFETIME from `now` (or until `expiry`)."""
    if expiry is None:
        expiry = int(now if now is not None else time.time()) + TOKEN_LIFETIME
    return f"v1.{expiry}.{_mac(secret, expiry)}"


def make_media_token(secret: bytes, now: float | None = None) -> str:
    """Token for the `k` on media URLs handed to headsets (videos, scripts,
    subtitles, events). The expiry is rounded down to a whole UTC day (29-30
    days left), so the URLs stay the same all day."""
    now = time.time() if now is None else now
    return make_token(secret, expiry=int(now + TOKEN_LIFETIME) // 86400 * 86400)


THUMB_PATH_PREFIX = "/api/media/thumb/"
_THUMB_TOKEN_RE = re.compile(r"t1\.([0-9a-f]{64})")


def make_thumb_token(secret: bytes) -> str:
    """The `k` for thumbnail URLs handed to headsets: no expiry, valid for
    thumbnails only (verify_thumb_token), the same until the PIN changes."""
    return "t1." + hmac.new(secret, b"t1.thumbs", hashlib.sha256).hexdigest()


def verify_thumb_token(secret: bytes, token: str | None) -> bool:
    return bool(token and _THUMB_TOKEN_RE.fullmatch(token)) and hmac.compare_digest(
        make_thumb_token(secret), token)


def verify_token(secret: bytes, token: str | None, now: float | None = None) -> bool:
    if not token:
        return False
    m = _TOKEN_RE.fullmatch(token)
    if not m:
        return False
    expiry = int(m.group(1))
    if expiry <= (time.time() if now is None else now):
        return False
    return hmac.compare_digest(_mac(secret, expiry), m.group(2))


def with_key(url: str, token: str | None) -> str:
    """`url` with `k=<token>` appended; unchanged without a token."""
    if not token:
        return url
    return f"{url}{'&' if '?' in url else '?'}{QUERY_KEY}={token}"


class RateLimiter:
    """Wrong PINs per client IP: after MAX_FAILURES within FAILURE_WINDOW the
    IP is refused until the oldest of them is FAILURE_WINDOW old."""

    def __init__(self, max_failures: int = MAX_FAILURES, window: float = FAILURE_WINDOW, clock=time.monotonic):
        self._max = max_failures
        self._window = window
        self._clock = clock
        self._failures: dict[str, deque] = {}
        self._lock = threading.Lock()

    def _recent(self, ip: str, now: float) -> deque | None:
        q = self._failures.get(ip)
        if q is None:
            return None
        while q and q[0] <= now - self._window:
            q.popleft()
        if not q:
            del self._failures[ip]
            return None
        return q

    def retry_after(self, ip: str) -> float:
        """Seconds until `ip` may try again; 0 when it may now."""
        with self._lock:
            now = self._clock()
            q = self._recent(ip, now)
            if q is None or len(q) < self._max:
                return 0.0
            return max(q[-self._max] + self._window - now, 0.0)

    def record_failure(self, ip: str) -> None:
        with self._lock:
            now = self._clock()
            for other in list(self._failures):
                self._recent(other, now)
            self._failures.setdefault(ip, deque(maxlen=self._max)).append(now)
            if len(self._failures[ip]) >= self._max:
                log.warning("Access PIN: %s entered %d wrong PINs, refused for %d minutes",
                            ip, self._max, int(self._window // 60))
            else:
                log.info("Access PIN: wrong PIN from %s", ip)

    def reset(self, ip: str) -> None:
        with self._lock:
            self._failures.pop(ip, None)


limiter = RateLimiter()
