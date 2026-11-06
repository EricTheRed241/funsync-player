/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Python writes every log line to stderr; main.log should show uvicorn's and
// the backend's INFO lines as info, not as errors, and still flag a crash.
import { describe, it, expect } from 'vitest';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const log = require('../../electron/logger.js');
log.transports.file.level = false;
log.transports.console.level = false;
const { stderrLevel } = require('../../electron/python-bridge.js');

describe('stderrLevel', () => {
  it('maps the LEVEL: prefix', () => {
    expect(stderrLevel('INFO:     127.0.0.1:5 - "GET /x HTTP/1.1" 200 OK\n')).toBe('info');
    expect(stderrLevel('INFO:  [funsync.stereo_detect] stereo check queued 3 video(s)\n')).toBe('info');
    expect(stderrLevel('WARNING:  [funsync.vr] Edit dropped: full\n')).toBe('warn');
    expect(stderrLevel('ERROR:    Exception in ASGI application\n')).toBe('error');
  });

  it('treats unprefixed output (a traceback, a crash) as an error', () => {
    expect(stderrLevel('Traceback (most recent call last):\n  File "x.py"')).toBe('error');
    expect(stderrLevel('Segmentation fault')).toBe('error');
  });
});
