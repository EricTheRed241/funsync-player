"""Device telemetry for the local device dashboard (GET /device).

Two inputs, both from the desktop's observer socket (routes/remote_sync.py):

- `desktop-status` snapshots (renderer/js/desktop-status.js), sent on every
  device or link change and every 15 s, whether or not a dashboard is open.
  They feed the event timeline, so drops and reconnects from before the
  dashboard was opened are there too.
- `device-telemetry` samples (renderer/js/device-telemetry.js), one a second
  while a lease runs. The dashboard's polls renew the lease; when it closes,
  the desktop stops sending after LEASE_MS.

Everything lives in memory for the backend's lifetime (the app's run).
"""

from collections import deque
from datetime import datetime, timezone

LEASE_MS = 30_000
# Renew the lease well before it runs out; a missed poll or two must not
# stop the stream.
LEASE_RENEW_S = 10
SAMPLE_KEEP = 900  # 15 min at one sample a second
COMMAND_KEEP = 6_000  # ~100 s at 60 commands a second
EVENT_KEEP = 500
# The dashboard shows "streaming" while the newest sample is this recent.
STREAMING_WITHIN_S = 3


def _iso(ts):
    return datetime.fromtimestamp(ts, timezone.utc).isoformat(timespec="milliseconds")


def _view_from_status(status):
    """The fields events are derived from, out of a desktop-status snapshot."""
    devices = status.get("devices") or {}
    tcode = devices.get("tcode") or {}
    vr = status.get("vr") or {}
    bp = devices.get("buttplug") or {}
    return {
        "tcode": tcode.get("link") if tcode else None,
        "syncActive": tcode.get("syncActive") if tcode else None,
        "vr": vr or None,
        "buttplug": bp.get("connected") if bp else None,
        "source": None,
        "script": None,
    }


def _view_from_sample(sample):
    tcode = sample.get("tcode") or {}
    sync = sample.get("sync") or {}
    bp = sample.get("buttplug") or {}
    return {
        "tcode": tcode.get("link") if tcode else None,
        "syncActive": sync.get("active") if sync else None,
        "vr": sample.get("vr"),
        "buttplug": bp.get("connected") if bp else None,
        "source": sample.get("source"),
        "script": sample.get("script"),
        "axes": [a.get("axis") for a in (sync.get("axes") or []) if isinstance(a, dict)],
    }


def _link_text(link):
    state = link.get("state") or "unknown"
    where = link.get("url") or link.get("transport") or "device"
    if state == "connected":
        ip = f" ({link['ip']})" if link.get("ip") else ""
        return f"TCode link connected: {where}{ip}"
    if state == "reconnecting":
        return f"TCode link lost, reconnecting: {where}"
    if state == "waiting":
        return f"TCode link waiting for the device: {where}"
    if state == "stopped":
        return "TCode link stopped"
    return f"TCode link {state}: {where}"


class Telemetry:
    def __init__(self):
        self.reset()

    def reset(self):
        self.samples = deque(maxlen=SAMPLE_KEEP)  # (seq, ts, sample)
        self.commands = deque(maxlen=COMMAND_KEEP)  # (t_ms, cmd)
        self.events = deque(maxlen=EVENT_KEEP)
        self._seq = 0
        self._event_seq = 0
        self._prev = None
        self._lease_sent_at = None

    # --- inputs -------------------------------------------------------------

    def record_status(self, status, now):
        if isinstance(status, dict):
            self._observe(_view_from_status(status), now)

    def record_sample(self, sample, now):
        if not isinstance(sample, dict):
            return
        self._seq += 1
        self.samples.append((self._seq, now, sample))
        last_t = self.commands[-1][0] if self.commands else 0
        for c in (sample.get("tcode") or {}).get("commands") or []:
            t, cmd = (c.get("t"), c.get("cmd")) if isinstance(c, dict) else (None, None)
            # Samples cover disjoint windows; anything older than what we hold
            # is a resend after an observer reconnect.
            if isinstance(t, (int, float)) and isinstance(cmd, str) and t > last_t:
                self.commands.append((t, cmd))
                last_t = t
        self._observe(_view_from_sample(sample), now)

    def observer_changed(self, connected, now):
        """The desktop's observer socket opened or closed. A new renderer has
        no lease, so the next poll sends one right away."""
        self._lease_sent_at = None
        self._add(now, "desktop", "info" if connected else "warn",
                  "Desktop app connected to the backend" if connected
                  else "Desktop app disconnected from the backend")

    # --- lease --------------------------------------------------------------

    def lease_due(self, now):
        return self._lease_sent_at is None or now - self._lease_sent_at >= LEASE_RENEW_S

    def lease_sent(self, now):
        self._lease_sent_at = now

    # --- events -------------------------------------------------------------

    def _add(self, now, kind, level, text):
        self._event_seq += 1
        self.events.append({"seq": self._event_seq, "at": _iso(now), "kind": kind, "level": level, "text": text})

    def _observe(self, view, now):
        prev = self._prev or {}
        # No transport reads as null in a status snapshot and as `stopped`
        # from main's live snapshot; treat both the same so they don't flap.
        view = {**view, "tcode": view.get("tcode") or {"state": "stopped"}}
        self._observe_tcode(prev.get("tcode") or {}, view["tcode"], now)

        sync, prev_sync = view.get("syncActive"), prev.get("syncActive")
        if sync is not None and bool(sync) != bool(prev_sync):
            self._add(now, "sync", "info", "Sync engine started" if sync else "Sync engine stopped")

        self._observe_vr(prev.get("vr") or {}, view.get("vr"), now)

        bp, prev_bp = view.get("buttplug"), prev.get("buttplug")
        if bp is not None and bool(bp) != bool(prev_bp):
            self._add(now, "buttplug", "info", "Intiface connected" if bp else "Intiface disconnected")

        # Only samples know the source and the script; keep the last known
        # values across status snapshots so they don't flap.
        source = view.get("source") if "axes" in view else prev.get("source")
        if "axes" in view and source != prev.get("source"):
            self._add(now, "sync", "info", f"Sync follows: {source}" if source else "Sync follows nothing")
        script = view.get("script") if "axes" in view else prev.get("script")
        if "axes" in view and (script or {}).get("name") != (prev.get("script") or {}).get("name"):
            if script:
                axes = ", ".join(view.get("axes") or []) or "none"
                self._add(now, "script", "info",
                          f"Script loaded: {script.get('name')} ({script.get('actions')} actions; axes {axes})")
            else:
                self._add(now, "script", "info", "Script unloaded")

        self._prev = {**view, "source": source, "script": script}

    def _observe_tcode(self, prev, link, now):
        state = link.get("state")
        drop = link.get("lastDisconnect") or {}
        new_drop = drop.get("at") and drop.get("at") != (prev.get("lastDisconnect") or {}).get("at")
        if state != prev.get("state"):
            text = _link_text(link)
            if new_drop and state in ("reconnecting", "stopped"):
                text += f" ({drop.get('close')})"
            self._add(now, "tcode", "info" if state == "connected" else "warn", text)
        elif new_drop:
            self._add(now, "tcode", "warn", f"TCode connection dropped ({drop.get('close')})")
        err = (link.get("lastError") or {}).get("text")
        if err and err != (prev.get("lastError") or {}).get("text") and state != "connected":
            attempts = link.get("attempts")
            tail = f" (attempt {attempts})" if attempts else ""
            self._add(now, "tcode", "warn", f"Reconnect failing: {err}{tail}")
        if state == "connected" and prev.get("state") == "connected" and link.get("ip") and prev.get("ip") \
                and link["ip"] != prev["ip"]:
            self._add(now, "tcode", "warn", f"TCode device IP changed: {prev['ip']} → {link['ip']}")

    def _observe_vr(self, prev, vr, now):
        if not vr:
            return
        connected, was = bool(vr.get("connected")), bool(prev.get("connected"))
        if connected != was:
            if connected:
                self._add(now, "vr", "info", f"VR bridge connected: {vr.get('playerType') or 'player'} at {vr.get('host')}")
            else:
                err = vr.get("lastError")
                self._add(now, "vr", "warn", "VR bridge disconnected" + (f" ({err})" if err else ""))
            return
        state, prev_state = vr.get("linkState"), prev.get("linkState")
        if connected and state != prev_state and prev_state:
            if state == "waiting":
                self._add(now, "vr", "warn", "VR bridge: no timestamp packets for 5 s")
            elif state == "receiving":
                self._add(now, "vr", "info", "VR bridge: packets flowing again")
        if connected and vr.get("video") and vr.get("video") != prev.get("video"):
            self._add(now, "vr", "info", f"Headset video: {vr['video']}")

    # --- output -------------------------------------------------------------

    def view(self, since_seq, since_cmd_ms, since_event, now):
        latest = self.samples[-1] if self.samples else None
        return {
            "streaming": bool(latest and now - latest[1] <= STREAMING_WITHIN_S),
            "latest": latest[2] if latest else None,
            "latestAgeS": round(now - latest[1], 1) if latest else None,
            "seq": latest[0] if latest else 0,
            "points": [_point(seq, ts, s) for seq, ts, s in self.samples if seq > since_seq],
            "commands": [{"t": t, "cmd": c} for t, c in self.commands if t > since_cmd_ms],
            "events": [e for e in self.events if e["seq"] > since_event],
        }


def _point(seq, ts, sample):
    """The charted numbers of one sample."""
    tcode = sample.get("tcode") or {}
    link = tcode.get("link") or {}
    sent = link.get("sent") or {}
    output = tcode.get("output") or {}
    sync = sample.get("sync") or {}
    vr = sample.get("vr") or {}
    return {
        "seq": seq,
        "t": round(ts * 1000),
        "linkState": link.get("state"),
        "sent": sent.get("commands"),
        "handed": output.get("commands"),
        "failed": output.get("failed"),
        "mediaMs": sync.get("mediaTimeMs"),
        "paused": sync.get("paused"),
        "vrPacketAgeMs": vr.get("lastPacketAgeMs"),
        "vrJitterMs": vr.get("jitterMs"),
    }


telemetry = Telemetry()
