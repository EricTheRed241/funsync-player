// Runs the local sites of deploy/sites.json (the swipe deck, Creators, Status) next to the
// app, each on 127.0.0.1:<port> only: the backend serves them on its own port under their
// prefix (backend/routes/sites.py), behind the access PIN. Built from the checkout at start
// and rebuilt and restarted a few seconds after one exits, so killing a site's process
// reloads it after a merge (electron/rust-process.js). Off in a packaged app and with
// FUNSYNC_SITES=0.
const fs = require('fs');
const path = require('path');
const log = require('./logger');
const { supervise, toolsEnv, enabled } = require('./rust-process');

const RESTART_DELAY_MS = 5_000;

let running = [];

/** deploy/sites.json's sites. */
function readSites(appPath) {
  return JSON.parse(fs.readFileSync(path.join(appPath, 'deploy', 'sites.json'), 'utf8')).sites;
}

function startSites({ appPath, userData, isPackaged }) {
  if (!enabled('FUNSYNC_SITES', { isPackaged })) {
    log.info('[sites] off (packaged app or FUNSYNC_SITES=0)');
    return;
  }
  let sites;
  try {
    sites = readSites(appPath);
  } catch (err) {
    log.warn(`[sites] deploy/sites.json: ${err.message}`);
    return;
  }
  const env = { ...toolsEnv(), FUNSYNC_USER_DATA: userData };
  running = sites.map((s) => supervise({
    tag: `site ${s.package}`,
    pkg: s.package,
    args: ['--port', String(s.port), '--user-data', userData],
    appPath,
    env,
    restartDelayMs: RESTART_DELAY_MS,
  }));
}

function stopSites() {
  for (const r of running) r.stop();
  running = [];
}

module.exports = { startSites, stopSites, readSites };
