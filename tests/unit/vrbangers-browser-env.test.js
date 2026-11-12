/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// The VRBangers browser in a pod (scripts/vrbangers-browser.mjs browserOptions):
// Debian's Chromium, its own Linux profile, --no-sandbox as root, and the
// account file from a Secret. Explicit options win over the environment,
// which wins over the Mac defaults. No browser is started.
import { describe, it, expect } from 'vitest';
import path from 'node:path';
import {
  CHROME_PATH, DEFAULT_CREDENTIALS, PROFILE_DIR, browserOptions, chromeArgs,
} from '../../scripts/vrbangers-browser.mjs';

const POD = {
  FUNSYNC_CHROME_PATH: '/usr/bin/chromium',
  FUNSYNC_CHROME_PROFILE: 'vrbangers-chromium-linux',
  FUNSYNC_CHROME_NO_SANDBOX: '1',
  FUNSYNC_VRB_CREDENTIALS: '/secrets/vrbangers/credentials.txt',
};

describe('browserOptions', () => {
  it('defaults to the Mac Chrome, its profile and the repo account file', () => {
    expect(browserOptions({}, {})).toEqual({
      chromePath: CHROME_PATH, profileDir: PROFILE_DIR, noSandbox: false, credentialsFile: DEFAULT_CREDENTIALS,
    });
    expect(PROFILE_DIR).toBe('vrbangers-chrome');
  });

  it('takes each setting from the environment', () => {
    expect(browserOptions({}, POD)).toEqual({
      chromePath: '/usr/bin/chromium',
      profileDir: 'vrbangers-chromium-linux',
      noSandbox: true,
      credentialsFile: '/secrets/vrbangers/credentials.txt',
    });
    expect(browserOptions({}, { FUNSYNC_CHROME_NO_SANDBOX: '0' }).noSandbox).toBe(false);
    expect(browserOptions({}, { FUNSYNC_VRB_CREDENTIALS: 'rel/creds.txt' }).credentialsFile).toBe(path.resolve('rel/creds.txt'));
  });

  it('lets explicit options win over the environment', () => {
    expect(browserOptions({
      chromePath: '/opt/chrome', profileDir: 'other', noSandbox: false, credentialsFile: '/c.txt',
    }, POD)).toEqual({ chromePath: '/opt/chrome', profileDir: 'other', noSandbox: false, credentialsFile: '/c.txt' });
  });

  it('takes a profile folder name only, never a path', () => {
    for (const bad of ['../vrbangers-chrome', '/tmp/profile', 'a/b', '..', '.']) {
      expect(() => browserOptions({}, { FUNSYNC_CHROME_PROFILE: bad }), bad).toThrow(/folder name/);
    }
  });
});

describe('chromeArgs', () => {
  it('adds --no-sandbox only when asked', () => {
    const base = { profile: '/ud/vrbangers-chromium-linux', headless: true };
    expect(chromeArgs({ ...base, noSandbox: false })).not.toContain('--no-sandbox');
    const args = chromeArgs({ ...base, noSandbox: true });
    expect(args).toContain('--no-sandbox');
    expect(args).toContain('--user-data-dir=/ud/vrbangers-chromium-linux');
    expect(args).toContain('--headless=new');
    expect(args.at(-1)).toBe('about:blank');
  });
});
