// Runs one of the checkout's Rust binaries next to the app and keeps it running: built
// first (`cargo build --release -p <package>`, quick when nothing changed), started from the
// checkout, and started again (rebuilt) `restartDelayMs` after it exits. A process the app
// starts is covered by the access macOS gave the app (Documents for the repo, removable
// volumes for the library), which launchd agents aren't. Used by the job runner
// (electron/job-runner.js) and the local sites (electron/sites.js).
const { spawn } = require('child_process');
const path = require('path');
const os = require('os');
const log = require('./logger');

// An app started at login (LaunchAgent) gets a bare PATH; the binaries need cargo,
// node / npm, ffmpeg and omp.
function toolsEnv(env = process.env, home = os.homedir()) {
  const parts = String(env.PATH || '').split(':').filter(Boolean);
  const extra = [path.join(home, '.cargo', 'bin'), '/opt/homebrew/bin', '/usr/local/bin'].filter((p) => !parts.includes(p));
  return { ...env, PATH: [...extra, ...parts].join(':') };
}

/** Off for a packaged app (no checkout or cargo) and when `env[flag]` is 0 / false / no / off. */
function enabled(flag, { env = process.env, isPackaged = false } = {}) {
  if (isPackaged) return false;
  const value = String(env[flag] || '').trim().toLowerCase();
  return !['0', 'false', 'no', 'off'].includes(value);
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

/**
 * Build and run `target/release/<bin> <args>` from `appPath`, restarting it after it exits.
 * Log lines carry `[tag]`. Returns `{ stop }`: stop kills it and ends the restarts.
 */
function supervise({ tag, pkg, bin = pkg, args = [], appPath, env, restartDelayMs }) {
  let child = null;
  let stopping = false;
  let restartTimer = null;

  const scheduleRestart = (why) => {
    if (stopping) return;
    log.warn(`[${tag}] ${why}; trying again in ${restartDelayMs / 1000} s`);
    clearTimeout(restartTimer);
    restartTimer = setTimeout(launch, restartDelayMs);
  };

  function launch() {
    if (stopping || child) return;
    const build = spawn('cargo', ['build', '-q', '--release', '-p', pkg], {
      cwd: appPath, env, stdio: ['ignore', 'ignore', 'pipe'],
    });
    let buildErr = '';
    build.stderr.on('data', (d) => { buildErr = (buildErr + d).slice(-2000); });
    build.on('error', (err) => scheduleRestart(`can't build ${pkg} (${err.message})`));
    build.on('exit', (code) => {
      if (code !== 0) {
        if (code !== null) scheduleRestart(`building ${pkg} failed (exit ${code}): ${buildErr.trim()}`);
        return;
      }
      if (stopping) return;
      child = spawn(path.join(appPath, 'target', 'release', bin), args, { cwd: appPath, env, stdio: ['ignore', 'pipe', 'pipe'] });
      log.info(`[${tag}] started (pid ${child.pid})`);
      pipeLines(child.stdout, (line) => log.info(`[${tag}] ${line}`));
      pipeLines(child.stderr, (line) => log.warn(`[${tag}] ${line}`));
      child.on('error', (err) => log.warn(`[${tag}] ${err.message}`));
      child.on('exit', (code, signal) => {
        child = null;
        scheduleRestart(`exited (${signal || `exit ${code}`})`);
      });
    });
  }

  launch();
  return {
    stop() {
      stopping = true;
      clearTimeout(restartTimer);
      if (child) {
        child.kill('SIGTERM');
        child = null;
      }
    },
  };
}

module.exports = { supervise, toolsEnv, enabled };
