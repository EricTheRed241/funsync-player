// Starts the scheduled-jobs scheduler (crates/job-runner, `job-runner serve`)
// next to the app, so the jobs in deploy/jobs.json run on this Mac with the
// app's macOS permissions: a process the app starts is covered by the access
// macOS gave the app (Documents for the repo, removable volumes for the
// library), which launchd agents and Docker containers aren't.
//
// The scheduler is built from the checkout first (`cargo build --release`,
// quick when nothing changed) and restarted a minute after it exits. It
// stops when the app quits; runs it started carry on in their own process
// group. Off in a packaged app (no checkout or cargo) and with
// FUNSYNC_JOB_RUNNER=0.
const { spawn } = require('child_process');
const path = require('path');
const os = require('os');
const log = require('./logger');

const RESTART_DELAY_MS = 60_000;

let child = null;
let stopping = true;
let restartTimer = null;
let context = null;

function runnerEnabled({ env = process.env, isPackaged = false } = {}) {
  if (isPackaged) return false;
  const value = String(env.FUNSYNC_JOB_RUNNER || '').trim().toLowerCase();
  return !['0', 'false', 'no', 'off'].includes(value);
}

// An app started at login (LaunchAgent) gets a bare PATH; jobs need cargo,
// node / npm, ffmpeg and omp.
function jobsEnv(env = process.env, home = os.homedir()) {
  const parts = String(env.PATH || '').split(':').filter(Boolean);
  const extra = [path.join(home, '.cargo', 'bin'), '/opt/homebrew/bin', '/usr/local/bin'].filter((p) => !parts.includes(p));
  return { ...env, PATH: [...extra, ...parts].join(':') };
}

function pipeLines(stream, write) {
  let rest = '';
  stream.setEncoding('utf8');
  stream.on('data', (chunk) => {
    const lines = (rest + chunk).split('\n');
    rest = lines.pop();
    for (const line of lines) if (line.trim()) write(line);
  });
  stream.on('end', () => { if (rest.trim()) write(rest); });
}

function scheduleRestart(why) {
  if (stopping) return;
  log.warn(`[jobs] ${why}; trying again in ${RESTART_DELAY_MS / 1000} s`);
  clearTimeout(restartTimer);
  restartTimer = setTimeout(launch, RESTART_DELAY_MS);
}

function launch() {
  if (stopping || child) return;
  const { appPath, userData } = context;
  const env = { ...jobsEnv(), FUNSYNC_USER_DATA: userData };
  const build = spawn('cargo', ['build', '-q', '--release', '-p', 'job-runner'], {
    cwd: appPath, env, stdio: ['ignore', 'ignore', 'pipe'],
  });
  let buildErr = '';
  build.stderr.on('data', (d) => { buildErr = (buildErr + d).slice(-2000); });
  build.on('error', (err) => scheduleRestart(`can't build the scheduler (${err.message})`));
  build.on('exit', (code) => {
    if (code !== 0) {
      if (code !== null) scheduleRestart(`building the scheduler failed (exit ${code}): ${buildErr.trim()}`);
      return;
    }
    if (stopping) return;
    const bin = path.join(appPath, 'target', 'release', 'job-runner');
    child = spawn(bin, ['serve'], { cwd: appPath, env, stdio: ['ignore', 'pipe', 'pipe'] });
    log.info(`[jobs] scheduler started (pid ${child.pid})`);
    pipeLines(child.stdout, (line) => log.info(`[jobs] ${line}`));
    pipeLines(child.stderr, (line) => log.warn(`[jobs] ${line}`));
    child.on('error', (err) => log.warn(`[jobs] scheduler: ${err.message}`));
    child.on('exit', (code, signal) => {
      child = null;
      scheduleRestart(`scheduler exited (${signal || `exit ${code}`})`);
    });
  });
}

function startJobRunner({ appPath, userData, isPackaged }) {
  if (!runnerEnabled({ isPackaged })) {
    log.info('[jobs] scheduler off (packaged app or FUNSYNC_JOB_RUNNER=0)');
    return;
  }
  context = { appPath, userData };
  stopping = false;
  launch();
}

function stopJobRunner() {
  stopping = true;
  clearTimeout(restartTimer);
  if (child) {
    child.kill('SIGTERM');
    child = null;
  }
}

module.exports = { startJobRunner, stopJobRunner };
