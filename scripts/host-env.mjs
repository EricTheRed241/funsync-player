// What the CLIs share about where they run: `FUNSYNC_USER_DATA` names the
// userData folder, `FUNSYNC_CONFIG_READONLY=1` keeps a CLI from ever writing
// config.json, and pid lock files keep two runs of one kind apart. No
// renderer imports: any CLI can load it.

import fs from 'node:fs';
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

/** `FUNSYNC_CONFIG_READONLY=1`: never write config.json; the app counts as running. */
export const configReadonly = (env = process.env) => ['1', 'true', 'yes'].includes(String(env.FUNSYNC_CONFIG_READONLY || '').trim().toLowerCase());

/** Throws under FUNSYNC_CONFIG_READONLY; call before any config.json write. */
export function assertConfigWritable(env = process.env) {
  if (configReadonly(env)) throw new Error('FUNSYNC_CONFIG_READONLY is set: config.json is not written.');
}

/** Whether a process with this pid runs on this machine. */
export function pidAlive(pid) {
  try { process.kill(pid, 0); return true; } catch (err) { return err.code === 'EPERM'; }
}

/**
 * A lock file's holder (`{pid}`): a bare `<pid>`; files from older versions
 * hold `<pid>@<host>`, whose host is ignored. Null when unreadable.
 */
export function parseLock(text) {
  const s = String(text ?? '').trim();
  const at = s.indexOf('@');
  const pid = Number(at < 0 ? s : s.slice(0, at));
  if (!Number.isInteger(pid) || pid <= 0) return null;
  return { pid };
}

/** The lock file's contents for this process. */
export const lockText = (pid = process.pid) => String(pid);

/** Whether a lock still holds: while its pid runs. */
export const lockHolds = (holder, { alive = pidAlive } = {}) => alive(holder.pid);

/**
 * Take `file` (`<pid>`, freed at exit or by releasePidLock). Null when taken,
 * else the holder (`{pid}`) of the live run that has it. A lock whose pid is
 * gone, or that is unreadable, is taken over.
 */
export function takePidLock(file, { alive = pidAlive } = {}) {
  const mine = lockText();
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      fs.writeFileSync(file, mine, { flag: 'wx' });
      process.on('exit', () => releasePidLock(file));
      return null;
    } catch (err) {
      if (err.code !== 'EEXIST') throw err;
    }
    let holder = null;
    try { holder = parseLock(fs.readFileSync(file, 'utf8')); } catch { /* gone meanwhile */ }
    if (holder && lockHolds(holder, { alive })) return holder;
    fs.rmSync(file, { force: true });
  }
  throw new Error(`Could not take ${file}`);
}

/** Free `file` when this process holds it (takePidLock), before the run ends; another holder's is left alone. */
export function releasePidLock(file) {
  try { if (fs.readFileSync(file, 'utf8') === lockText()) fs.rmSync(file); } catch { /* already gone */ }
}

/** `pid 12`, for messages. */
export const describeHolder = ({ pid }) => `pid ${pid}`;
