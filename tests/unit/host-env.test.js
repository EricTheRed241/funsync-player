/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// What the CLIs share (scripts/host-env.mjs, scripts/discovery-io.mjs):
// FUNSYNC_USER_DATA over the app's folder, FUNSYNC_CONFIG_READONLY never
// writing config.json and counting the app as running, and pid lock files.
import { describe, it, expect, afterEach } from 'vitest';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  assertConfigWritable, configReadonly, defaultUserDataDir, describeHolder,
  lockHolds, lockText, parseLock, releasePidLock, takePidLock,
} from '../../scripts/host-env.mjs';
import { appRunning, ensureLibrarySource, writeConfig } from '../../scripts/discovery-io.mjs';

let tmp = null;
const tempDir = () => (tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'host-env-')));
afterEach(() => { if (tmp) fs.rmSync(tmp, { recursive: true, force: true }); tmp = null; });

describe('defaultUserDataDir', () => {
  it('is FUNSYNC_USER_DATA when set, else the app folder of the OS', () => {
    const ud = '/Volumes/Data/funsync-player';
    expect(defaultUserDataDir('linux', { FUNSYNC_USER_DATA: ud }, '/root')).toBe(ud);
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

  it('running while the pid lives, whatever the host', () => {
    const ud = withLock('mac-mini.local-4242');
    expect(appRunning(ud, { env: {}, alive: (pid) => pid === 4242 })).toBe(true);
    expect(appRunning(ud, { env: {}, alive: () => false })).toBe(false);
  });

  it('no lock: closed; an unreadable pid: running', () => {
    expect(appRunning(tempDir(), { env: {} })).toBe(false);
    expect(appRunning(withLock('host-nopid'), { env: {} })).toBe(true);
  });
});

describe('pid lock files', () => {
  it('reads `<pid>` and the old `<pid>@<host>`', () => {
    expect(parseLock('42\n')).toEqual({ pid: 42 });
    expect(parseLock('42@mac-mini.local')).toEqual({ pid: 42 });
    expect(parseLock('not a pid')).toBe(null);
    expect(parseLock('')).toBe(null);
    expect(lockText(7)).toBe('7');
    expect(describeHolder({ pid: 7 })).toBe('pid 7');
  });

  it('held while the pid lives', () => {
    expect(lockHolds({ pid: 42 }, { alive: (pid) => pid === 42 })).toBe(true);
    expect(lockHolds({ pid: 42 }, { alive: () => false })).toBe(false);
  });

  it('takePidLock writes `<pid>`, refuses a live holder and takes over an ended one', () => {
    const file = path.join(tempDir(), 'eroscripts-scout.lock');
    // The old format with a dead pid: taken over.
    fs.writeFileSync(file, '99@mac-mini.local');
    expect(takePidLock(file, { alive: () => false })).toBe(null);
    expect(fs.readFileSync(file, 'utf8')).toBe(String(process.pid));
    // This process's live run: refused.
    expect(takePidLock(file)).toEqual({ pid: process.pid });
    // A live pid: refused, left in place.
    fs.writeFileSync(file, '99');
    expect(takePidLock(file, { alive: () => true })).toEqual({ pid: 99 });
    expect(fs.readFileSync(file, 'utf8')).toBe('99');
    // Unreadable: taken over.
    fs.writeFileSync(file, 'garbage');
    expect(takePidLock(file)).toBe(null);
  });

  it('releasePidLock frees only a lock this process holds, so a step can hand it on before the run ends', () => {
    const file = path.join(tempDir(), 'eroscripts-scout-frames.lock');
    expect(takePidLock(file)).toBe(null);
    releasePidLock(file);
    expect(fs.existsSync(file)).toBe(false);
    // Free now: another run takes it; this process's release leaves it alone.
    fs.writeFileSync(file, '99');
    releasePidLock(file);
    expect(fs.readFileSync(file, 'utf8')).toBe('99');
    releasePidLock(path.join(path.dirname(file), 'missing.lock'));
  });
});
