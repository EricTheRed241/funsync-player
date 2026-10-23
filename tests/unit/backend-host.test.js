/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Local-only mode: the backend must bind loopback whenever the user asked for
// it, and every interface otherwise (the phone remote and headsets need LAN).
import { describe, it, expect } from 'vitest';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const log = require('../../electron/logger.js');
log.transports.file.level = false;
log.transports.console.level = false;
const { backendHost } = require('../../electron/python-bridge.js');

describe('backendHost', () => {
  it('binds every interface by default', () => {
    expect(backendHost(['electron', '.'], {})).toBe('0.0.0.0');
  });

  it('binds loopback with --local-only', () => {
    expect(backendHost(['electron', '.', '--local-only'], {})).toBe('127.0.0.1');
  });

  it('binds loopback when FUNSYNC_LOCAL_ONLY is set to a truthy value', () => {
    for (const v of ['1', 'true', 'yes', 'on', 'TRUE']) {
      expect(backendHost([], { FUNSYNC_LOCAL_ONLY: v })).toBe('127.0.0.1');
    }
  });

  it('treats 0 / false / off / empty as LAN access allowed', () => {
    for (const v of ['0', 'false', 'off', 'no', '']) {
      expect(backendHost([], { FUNSYNC_LOCAL_ONLY: v })).toBe('0.0.0.0');
    }
  });
});
