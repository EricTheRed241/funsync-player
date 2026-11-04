// Desktop status snapshot for the backend's `GET /api/status`.
//
// The renderer owns the device managers, the VR companion bridge and the
// session tracker, so it builds this snapshot and sends it over the
// observer socket (`desktop-status`); the backend keeps the latest one and
// serves it with its own state. Lets a session going wrong (mid HereSphere
// video, phone remote) be diagnosed with one HTTP call instead of reading
// main.log.

const iso = (ms) => (Number.isFinite(ms) ? new Date(ms).toISOString() : null);
const basename = (p) => (p ? String(p).split(/[\\/]/).pop() : null);

/**
 * @param {object} deps — the app's managers; any may be missing.
 * @returns {object} JSON-safe snapshot.
 */
export function buildDesktopStatus({
  handyManager, buttplugManager, tcodeManager, tcodeSync, autoblowManager,
  vrBridge, sessionTracker, videoPlayer, currentVideoPath, now = Date.now(),
} = {}) {
  const video = videoPlayer?.video;
  const session = sessionTracker?.getSession?.() || null;
  return {
    at: iso(now),
    devices: {
      handy: handyManager ? { connected: !!handyManager.connected } : null,
      buttplug: buttplugManager ? {
        connected: !!buttplugManager.connected,
        devices: buttplugManager.connected ? (buttplugManager.devices || []).map((d) => d.name) : [],
      } : null,
      tcode: tcodeManager ? {
        connected: !!tcodeManager.connected,
        waiting: !!tcodeManager.waiting,
        syncActive: !!tcodeSync?._active,
        link: tcodeManager.status || null,
      } : null,
      autoblow: autoblowManager ? {
        connected: !!autoblowManager.connected,
        model: autoblowManager.connected ? (autoblowManager.isUltra ? 'Autoblow Ultra' : 'VacuGlide 2') : null,
      } : null,
    },
    vr: vrBridge ? {
      connected: !!vrBridge.connected,
      linkState: vrBridge.linkState,
      playerType: vrBridge.playerType || null,
      host: vrBridge.connected ? vrBridge.host : null,
      video: basename(vrBridge.currentVideoPath),
      jitterMs: vrBridge.connected ? (vrBridge.getNetworkJitterMs?.() ?? null) : null,
    } : null,
    session: session ? {
      source: session.source,
      identifier: session.identifier,
      state: session.state,
      video: session.videoName || null,
      currentTime: session.currentTime,
      duration: session.duration,
      startedAt: iso(session.startedAt),
    } : null,
    player: {
      video: basename(currentVideoPath),
      playing: !!(video && !video.paused && !video.ended),
      currentTime: video ? video.currentTime : null,
    },
  };
}
