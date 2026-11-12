/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The CLIs in a Kubernetes pod (scripts/host-env.mjs, scripts/discovery-io.mjs):
// FUNSYNC_USER_DATA over the app's folder, FUNSYNC_CONFIG_READONLY never
// writing config.json and counting the app as running, and lock files that
// judge another host's lock by its age, never by its pid.
import { describe, it, expect, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  FOREIGN_LOCK_MAX_AGE_MS, assertConfigWritable, configReadonly, defaultUserDataDir, describeHolder,
  lockHolds, lockText, parseLock, takePidLock,
} from '../../scripts/host-env.mjs';
import { appRunning, ensureLibrarySource, writeConfig } from '../../scripts/discovery-io.mjs';

const HOUR = 3600_000;
let tmp = null;
const tempDir = () => (tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'host-env-')));
afterEach(() => { if (tmp) fs.rmSync(tmp, { recursive: true, force: true }); tmp = null; });

describe('defaultUserDataDir', () => {
  it('is FUNSYNC_USER_DATA when set, else the app folder of the OS', () => {
    const pod = '/Users/me/Library/Application Support/funsync-player';
    expect(defaultUserDataDir('linux', { FUNSYNC_USER_DATA: pod }, '/root')).toBe(pod);
    expect(defaultUserDataDir('darwin', { FUNSYNC_USER_DATA: '' }, '/Users/me')).toBe('/Users/me/Library/Application Support/funsync-player');
    expect(defaultUserDataDir('darwin', {}, '/Users/me')).toBe('/Users/me/Library/Application Support/funsync-player');
    expect(defaultUserDataDir('linux', {}, '/home/me')).toBe('/home/me/.config/funsync-player');
    expect(defaultUserDataDir('linux', { XDG_CONFIG_HOME: '/x' }, '/home/me')).toBe('/x/funsync-player');
  });
});

describe('FUNSYNC_CONFIG_READONLY', () => {
  it('is on for 1 / true / yes only', () => {
    expect(configReadonly({ FUNSYNC_CONFIG_READONLY: '1' })).toBe(true);
    expect(configReadonly({ FUNSYNC_CONFIG_READONLY: 'true' })).toBe(true);
    expect(configReadonly({ FUNSYNC_CONFIG_READONLY: '0' })).toBe(false);
    expect(configReadonly({})).toBe(false);
    expect(() => assertConfigWritable({ FUNSYNC_CONFIG_READONLY: '1' })).toThrow(/FUNSYNC_CONFIG_READONLY/);
    expect(() => assertConfigWritable({})).not.toThrow();
  });

  it('makes the app count as running, lock or not', () => {
    const ud = tempDir();
    expect(appRunning(ud, { env: {} })).toBe(false);
    expect(appRunning(ud, { env: { FUNSYNC_CONFIG_READONLY: '1' } })).toBe(true);
  });

  it('blocks writeConfig and the library-source add', () => {
    const ud = tempDir();
    const file = path.join(ud, 'config.json');
    const config = { settings: { library: { sources: [{ id: '1', name: 'Other', path: path.join(ud, 'Other') }] } } };
    fs.writeFileSync(file, JSON.stringify(config));
    const before = fs.readFileSync(file, 'utf8');
    process.env.FUNSYNC_CONFIG_READONLY = '1';
    try {
      expect(() => writeConfig(ud, { changed: true })).toThrow(/FUNSYNC_CONFIG_READONLY/);
      ensureLibrarySource(ud, { name: 'EroScripts', path: path.join(ud, 'EroScripts') });
    } finally {
      delete process.env.FUNSYNC_CONFIG_READONLY;
    }
    expect(fs.readFileSync(file, 'utf8')).toBe(before);
    expect(fs.readdirSync(ud)).toEqual(['config.json']);
  });
});

describe('appRunning (SingletonLock `<host>-<pid>`)', () => {
  const withLock = (target) => {
    const ud = tempDir();
    fs.symlinkSync(target, path.join(ud, 'SingletonLock'));
    return ud;
  };

  it('this host: running while the pid lives', () => {
    const ud = withLock('mac-mini.local-4242');
    expect(appRunning(ud, { env: {}, host: 'mac-mini.local', alive: (pid) => pid === 4242 })).toBe(true);
    expect(appRunning(ud, { env: {}, host: 'mac-mini.local', alive: () => false })).toBe(false);
  });

  it('another host: running, its pid is never checked', () => {
    const ud = withLock('mac-mini.local-4242');
    const alive = () => { throw new Error('checked a foreign pid'); };
    expect(appRunning(ud, { env: {}, host: 'eroscripts-scout-2911-x7k2p', alive })).toBe(true);
  });

  it('no lock: closed; an unreadable pid: running', () => {
    expect(appRunning(tempDir(), { env: {} })).toBe(false);
    expect(appRunning(withLock('host-nopid'), { env: {}, host: 'host' })).toBe(true);
  });
});

describe('pid lock files', () => {
  it('reads `<pid>@<host>` and the old bare `<pid>`', () => {
    expect(parseLock('42@eroscripts-scout-2911-x7k2p\n')).toEqual({ pid: 42, host: 'eroscripts-scout-2911-x7k2p' });
    expect(parseLock('42')).toEqual({ pid: 42, host: null });
    expect(parseLock('not a pid')).toBe(null);
    expect(parseLock('')).toBe(null);
    expect(lockText(7, 'mac')).toBe('7@mac');
    expect(describeHolder({ pid: 7, host: 'mac' })).toBe('pid 7 on mac');
    expect(describeHolder({ pid: 7, host: null })).toBe('pid 7');
  });

  it('another host: held until 12 h old, whatever its pid', () => {
    const alive = () => { throw new Error('checked a foreign pid'); };
    const foreign = { pid: 42, host: 'mac' };
    expect(lockHolds(foreign, { ageMs: 0, host: 'pod', alive })).toBe(true);
    expect(lockHolds(foreign, { ageMs: FOREIGN_LOCK_MAX_AGE_MS - 1, host: 'pod', alive })).toBe(true);
    expect(lockHolds(foreign, { ageMs: FOREIGN_LOCK_MAX_AGE_MS, host: 'pod', alive })).toBe(false);
    expect(FOREIGN_LOCK_MAX_AGE_MS).toBe(12 * HOUR);
  });

  it('this host, and the old format: held while the pid lives, at any age', () => {
    for (const holder of [{ pid: 42, host: 'pod' }, { pid: 42, host: null }]) {
      expect(lockHolds(holder, { ageMs: 100 * HOUR, host: 'pod', alive: () => true })).toBe(true);
      expect(lockHolds(holder, { ageMs: 0, host: 'pod', alive: () => false })).toBe(false);
    }
  });

  it('takePidLock writes `<pid>@<host>`, refuses a live holder and takes over an ended one', () => {
    const file = path.join(tempDir(), 'eroscripts-scout.lock');
    const now = () => Date.now();
    // Another host's, an hour old: held.
    fs.writeFileSync(file, '99@mac-mini.local');
    fs.utimesSync(file, new Date(Date.now() - HOUR), new Date(Date.now() - HOUR));
    expect(takePidLock(file, { host: 'pod-a', now })).toEqual({ pid: 99, host: 'mac-mini.local' });
    // 13 hours old: taken over.
    fs.utimesSync(file, new Date(Date.now() - 13 * HOUR), new Date(Date.now() - 13 * HOUR));
    expect(takePidLock(file, { host: 'pod-a', now })).toBe(null);
    expect(fs.readFileSync(file, 'utf8')).toBe(`${process.pid}@pod-a`);
    // This host's live run (this process): refused.
    expect(takePidLock(file, { host: 'pod-a', now })).toEqual({ pid: process.pid, host: 'pod-a' });
    // The old format with a dead pid: taken over; with a live one: refused.
    fs.writeFileSync(file, '99');
    expect(takePidLock(file, { host: 'pod-a', now, alive: () => false })).toBe(null);
    fs.writeFileSync(file, '99');
    expect(takePidLock(file, { host: 'pod-a', now, alive: () => true })).toEqual({ pid: 99, host: null });
    // Unreadable: taken over.
    fs.writeFileSync(file, 'garbage');
    expect(takePidLock(file, { host: 'pod-a', now })).toBe(null);
  });
});
