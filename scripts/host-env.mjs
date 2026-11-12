// What changes when a CLI runs somewhere other than the Mac that runs the
// app (a Kubernetes pod with userData and the library mounted at the same
// paths): `FUNSYNC_USER_DATA` names the userData folder,
// `FUNSYNC_CONFIG_READONLY=1` keeps a CLI from ever writing config.json, and
// lock files record the host that wrote them, since a pid means something
// only on that host. No renderer imports: any CLI can load it.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

/**
 * The userData folder when no `--user-data` is given: `FUNSYNC_USER_DATA`, else
 * where the desktop app keeps it on this OS.
 */
export function defaultUserDataDir(platform = process.platform, env = process.env, home = env.HOME || env.USERPROFILE || '') {
  if (env.FUNSYNC_USER_DATA) return path.resolve(env.FUNSYNC_USER_DATA);
  if (platform === 'darwin') return path.join(home, 'Library', 'Application Support', 'funsync-player');
  if (platform === 'win32') return path.join(env.APPDATA || path.join(home, 'AppData', 'Roaming'), 'funsync-player');
  return path.join(env.XDG_CONFIG_HOME || path.join(home, '.config'), 'funsync-player');
}

/** A lock written on another host counts as held until it is this old. */
export const FOREIGN_LOCK_MAX_AGE_MS = 12 * 3600_000;

/** `FUNSYNC_CONFIG_READONLY=1`: never write config.json; the app counts as running. */
export const configReadonly = (env = process.env) => ['1', 'true', 'yes'].includes(String(env.FUNSYNC_CONFIG_READONLY || '').trim().toLowerCase());

/** Throws under FUNSYNC_CONFIG_READONLY; call before any config.json write. */
export function assertConfigWritable(env = process.env) {
  if (configReadonly(env)) throw new Error('FUNSYNC_CONFIG_READONLY is set: config.json is not written.');
}

/** Whether a process with this pid runs on this host. */
export function pidAlive(pid) {
  try { process.kill(pid, 0); return true; } catch (err) { return err.code === 'EPERM'; }
}

/**
 * A lock file's holder: `<pid>@<host>`, or a bare `<pid>` (written before
 * hosts were recorded; read as this host's, `host: null`). Null when unreadable.
 */
export function parseLock(text) {
  const s = String(text ?? '').trim();
  const at = s.indexOf('@');
  const pid = Number(at < 0 ? s : s.slice(0, at));
  if (!Number.isInteger(pid) || pid <= 0) return null;
  return { pid, host: at < 0 ? null : s.slice(at + 1) };
}

/** The lock file's contents for this process. */
export const lockText = (pid = process.pid, host = os.hostname()) => `${pid}@${host}`;

/**
 * Whether a lock still holds, `ageMs` after it was written: on this host while
 * its pid runs; on another host (whose pids can't be checked from here) until
 * FOREIGN_LOCK_MAX_AGE_MS.
 */
export function lockHolds(holder, { ageMs, host = os.hostname(), alive = pidAlive }) {
  if (holder.host != null && holder.host !== host) return ageMs < FOREIGN_LOCK_MAX_AGE_MS;
  return alive(holder.pid);
}

/**
 * Take `file` (`<pid>@<host>`, freed at exit). Null when taken, else the
 * holder (`{pid, host}`) of the live run that has it. A lock whose run has
 * ended, or that is unreadable, is taken over.
 */
export function takePidLock(file, { host = os.hostname(), now = Date.now, alive = pidAlive } = {}) {
  const mine = lockText(process.pid, host);
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      fs.writeFileSync(file, mine, { flag: 'wx' });
      process.on('exit', () => {
        try { if (fs.readFileSync(file, 'utf8') === mine) fs.rmSync(file); } catch { /* already gone */ }
      });
      return null;
    } catch (err) {
      if (err.code !== 'EEXIST') throw err;
    }
    let holder = null;
    let ageMs = 0;
    try {
      holder = parseLock(fs.readFileSync(file, 'utf8'));
      ageMs = now() - fs.statSync(file).mtimeMs;
    } catch { /* gone meanwhile */ }
    if (holder && lockHolds(holder, { ageMs, host, alive })) return holder;
    fs.rmSync(file, { force: true });
  }
  throw new Error(`Could not take ${file}`);
}

/** `pid 12` or `pid 12 on <host>`, for messages. */
export const describeHolder = ({ pid, host }) => (host ? `pid ${pid} on ${host}` : `pid ${pid}`);
