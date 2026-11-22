// Starts the scheduled-jobs scheduler (crates/job-runner, `job-runner serve`)
// next to the app, so the jobs in deploy/jobs.json run on this Mac with the
// app's macOS permissions (electron/rust-process.js). Restarted a minute after
// it exits; it stops when the app quits, and runs it started carry on in their
// own process group. Off in a packaged app and with FUNSYNC_JOB_RUNNER=0.
const log = require('./logger');
const { supervise, toolsEnv, enabled } = require('./rust-process');

const RESTART_DELAY_MS = 60_000;

let runner = null;

function startJobRunner({ appPath, userData, isPackaged }) {
  if (!enabled('FUNSYNC_JOB_RUNNER', { isPackaged })) {
    log.info('[jobs] scheduler off (packaged app or FUNSYNC_JOB_RUNNER=0)');
    return;
  }
  runner = supervise({
    tag: 'jobs',
    pkg: 'job-runner',
    args: ['serve'],
    appPath,
    env: { ...toolsEnv(), FUNSYNC_USER_DATA: userData },
    restartDelayMs: RESTART_DELAY_MS,
  });
}

function stopJobRunner() {
  runner?.stop();
  runner = null;
}

module.exports = { startJobRunner, stopJobRunner };
