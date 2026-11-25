/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// scripts/disk-space.mjs: the scouts keep MIN_FREE_GB free on the library drive after every download.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, it, expect } from 'vitest';
import { freeBytes, roomFor } from '../../scripts/disk-space.mjs';
import { MIN_FREE_GB } from '../../shared/content-preferences.js';
import shared from '../../shared/content-preferences.json' with { type: 'json' };

const GB = 1e9;

describe('roomFor', () => {
  it('fits when free − in flight − the file keeps the minimum', () => {
    expect(roomFor({ free: 80 * GB, bytes: 20 * GB, inFlight: 10 * GB, minFree: 50 * GB })).toBeNull();
    expect(roomFor({ free: 80 * GB, bytes: 20 * GB, inFlight: 0, minFree: 50 * GB })).toBeNull();
  });

  it('gives the reason when it would leave less', () => {
    expect(roomFor({ free: 80 * GB, bytes: 20 * GB, inFlight: 10.5 * GB, minFree: 50 * GB }))
      .toBe('not enough disk space (80 GB free, 80.5 GB needed to keep 50 GB free)');
  });

  it('checks the downloads in flight alone when the size is unknown', () => {
    expect(roomFor({ free: 60 * GB, bytes: null, inFlight: 10 * GB, minFree: 50 * GB })).toBeNull();
    expect(roomFor({ free: 60 * GB, bytes: 0, inFlight: 11 * GB, minFree: 50 * GB }))
      .toBe('not enough disk space (60 GB free, 61 GB needed to keep 50 GB free)');
    expect(roomFor({ free: 49 * GB, minFree: 50 * GB })).toMatch(/^not enough disk space \(49 GB free/);
  });

  it('measures the drive of the folder (or its nearest parent) when not given', () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'disk-space-'));
    try {
      const free = freeBytes(tmp);
      expect(free).toBeGreaterThan(0);
      expect(freeBytes(path.join(tmp, 'not', 'made', 'yet'))).toBeGreaterThan(0);
      expect(roomFor({ dir: tmp, bytes: 0, minFree: 0 })).toBeNull();
      expect(roomFor({ dir: tmp, bytes: free * 4, minFree: 0 })).toMatch(/^not enough disk space/);
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

describe('MIN_FREE_GB', () => {
  it('is the shared swipeDeck.minFreeGb', () => {
    expect(MIN_FREE_GB).toBe(shared.swipeDeck.minFreeGb);
  });
});
