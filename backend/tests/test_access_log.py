import logging

import pytest

from access_log import QUIET_POLL_PATHS, QuietLocalPollingFilter


def _record(client, method, path, status):
    return logging.LogRecord(
        "uvicorn.access", logging.INFO, __file__, 0,
        '%s - "%s %s HTTP/%s" %d', (client, method, path, "1.1", status), None,
    )


f = QuietLocalPollingFilter()


@pytest.mark.parametrize("path", sorted(QUIET_POLL_PATHS))
@pytest.mark.parametrize("client", ["127.0.0.1:64287", "[::1]:5000"])
def test_loopback_success_dropped(client, path):
    assert f.filter(_record(client, "GET", path, 200)) is False


def test_query_string_dropped():
    assert f.filter(_record("127.0.0.1:1", "GET", "/api/media/vr-activity?t=1", 200)) is False


def test_lan_client_kept():
    assert f.filter(_record("192.168.1.20:5555", "GET", "/api/media/vr-activity", 200)) is True


def test_loopback_failure_kept():
    assert f.filter(_record("127.0.0.1:1", "GET", "/health", 500)) is True


def test_other_path_kept():
    assert f.filter(_record("127.0.0.1:1", "GET", "/network-info", 200)) is True


def test_non_get_kept():
    assert f.filter(_record("127.0.0.1:1", "POST", "/health", 200)) is True


def test_non_access_record_kept():
    rec = logging.LogRecord("uvicorn.access", logging.INFO, __file__, 0, "hello", None, None)
    assert f.filter(rec) is True
