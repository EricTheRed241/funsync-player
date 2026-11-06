"""Quiet uvicorn access logging for the desktop app's own polling."""

import logging
from urllib.parse import urlsplit

# Endpoints the desktop app polls every second or so from 127.0.0.1. Their
# successful access lines made up ~97% of main.log, burying everything else.
QUIET_POLL_PATHS = frozenset({
    "/api/media/vr-activity",
    "/api/media/rescan-request",
    "/health",
})

_LOOPBACK_HOSTS = {"127.0.0.1", "::1", "localhost"}


def _client_host(client_addr: str) -> str:
    """Strip the port from uvicorn's "host:port" / "[::1]:port" client string."""
    if client_addr.startswith("["):
        return client_addr[1:].split("]", 1)[0]
    host, sep, _port = client_addr.rpartition(":")
    return host if sep and ":" not in host else client_addr


class QuietLocalPollingFilter(logging.Filter):
    """Drop successful loopback GETs of QUIET_POLL_PATHS; keep everything else."""

    def filter(self, record: logging.LogRecord) -> bool:
        args = record.args
        # uvicorn: (client_addr, method, full_path, http_version, status_code)
        if not isinstance(args, tuple) or len(args) != 5:
            return True
        client_addr, method, full_path, _version, status = args
        try:
            return not (
                method == "GET"
                and int(status) < 400
                and urlsplit(str(full_path)).path in QUIET_POLL_PATHS
                and _client_host(str(client_addr)) in _LOOPBACK_HOSTS
            )
        except (TypeError, ValueError):
            return True


def install() -> None:
    logging.getLogger("uvicorn.access").addFilter(QuietLocalPollingFilter())
