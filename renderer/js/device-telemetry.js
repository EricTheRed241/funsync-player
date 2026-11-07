// Live device telemetry for the local device dashboard (backend `GET /device`).
//
// The dashboard asks the backend for data; the backend sends the desktop a
// `telemetry-lease` over the observer socket, and while a lease runs the
// renderer sends one `device-telemetry` sample a second: the TCode link as
// main sees it right now (live send counters), the sync engine's state, the
// commands sent since the previous sample, and the VR bridge. Nothing is
// collected or sent while no dashboard is open.

const iso = (ms) => (Number.isFinite(ms) ? new Date(ms).toISOString() : null);
const basename = (p) => (p ? String(p).split(/[\\/]/).pop() : null);

// Commands per sample are capped so a stalled renderer can't send a huge
// catch-up message; the dashboard only charts the last 30 s anyway.
const MAX_COMMANDS_PER_SAMPLE = 600;

/**
 * @param {object} deps — the app's managers; any may be missing.
 * @param {object|null} deps.tcodeLink — main's live link snapshot
 *   (`tcode-status` IPC); falls back to the manager's last pushed one.
 * @param {string|null} deps.source — what the sync engines follow:
 *   'desktop' | 'heresphere' | 'deovr' | 'phone' | 'player-window'.
 * @param {number} deps.commandsSince — epoch ms of the previous sample; the
 *   sample carries the commands sent after it, up to `now`.
 * @returns {object} JSON-safe sample.
 */
export function buildDeviceTelemetry({
  tcodeManager, tcodeLink, tcodeSync, funscriptEngine, buttplugManager,
  vrBridge, source = null, commandsSince = 0, now = Date.now(),
} = {}) {
  const script = funscriptEngine?.isLoaded ? funscriptEngine.getInfo() : null;
  const commands = tcodeManager ? tcodeManager.recentCommands(commandsSince, now) : [];
  return {
    at: iso(now),
    source,
    tcode: tcodeManager ? {
      connected: !!tcodeManager.connected,
      waiting: !!tcodeManager.waiting,
      precision: tcodeManager.precision,
      link: tcodeLink || tcodeManager.status || null,
      output: tcodeManager.output,
      commands: commands.length > MAX_COMMANDS_PER_SAMPLE ? commands.slice(-MAX_COMMANDS_PER_SAMPLE) : commands,
    } : null,
    sync: tcodeSync ? tcodeSync.snapshot() : null,
    script: script ? {
      name: script.filename,
      actions: script.actionCount,
      durationMs: script.durationMs,
    } : null,
    vr: vrBridge ? {
      connected: !!vrBridge.connected,
      linkState: vrBridge.linkState,
      playerType: vrBridge.playerType || null,
      host: vrBridge.host || null,
      video: basename(vrBridge.currentVideoPath),
      jitterMs: vrBridge.connected ? (vrBridge.getNetworkJitterMs?.() ?? null) : null,
      lastPacketAgeMs: vrBridge.lastPacketAgeMs,
      lastError: vrBridge.lastError || null,
    } : null,
    buttplug: buttplugManager ? {
      connected: !!buttplugManager.connected,
      devices: buttplugManager.connected ? (buttplugManager.devices || []).map((d) => d.name) : [],
    } : null,
  };
}

/**
 * Sends samples while a lease runs. `extend(ms)` starts or lengthens the
 * lease; the stream stops by itself when it runs out, so a closed dashboard
 * costs nothing after at most one lease.
 */
export class TelemetryStream {
  /**
   * @param {object} opts
   * @param {(sinceMs: number, nowMs: number) => Promise<object>|object} opts.collect —
   *   builds the sample for the window (sinceMs, nowMs]: when the previous
   *   sample was taken, and this one.
   */
  constructor({
    collect, send, intervalMs = 1000, now = () => Date.now(),
    setInterval: setIntervalFn = setInterval, clearInterval: clearIntervalFn = clearInterval,
  }) {
    this._collect = collect;
    this._send = send;
    this._intervalMs = intervalMs;
    this._now = now;
    this._setInterval = setIntervalFn;
    this._clearInterval = clearIntervalFn;
    this._until = 0;
    this._timer = null;
    this._busy = false;
    this._lastAt = 0;
  }

  get running() { return this._timer !== null; }

  extend(ms) {
    if (!(ms > 0)) return;
    this._until = Math.max(this._until, this._now() + ms);
    if (this._timer) return;
    // The first sample only carries commands from the lease start on.
    this._lastAt = this._now();
    this._timer = this._setInterval(() => { this._tick(); }, this._intervalMs);
    this._tick();
  }

  stop() {
    if (this._timer) this._clearInterval(this._timer);
    this._timer = null;
    this._until = 0;
  }

  async _tick() {
    if (this._now() >= this._until) { this.stop(); return; }
    // A slow collect (IPC to main) must not stack up behind the timer.
    if (this._busy) return;
    this._busy = true;
    try {
      const since = this._lastAt;
      const until = this._now();
      this._lastAt = until;
      this._send(await this._collect(since, until));
    } catch (err) {
      console.warn('[Telemetry] sample failed:', err?.message || err);
    } finally {
      this._busy = false;
    }
  }
}
