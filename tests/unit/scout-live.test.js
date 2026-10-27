/**
 * @vitest-environment node
 * No DOM required — skips jsdom construction. See notes/CLAUDE.md
 * "Test environments" before changing this or adding DOM here.
 */
// Live action in Discovery (scripts/scout-live.mjs): favourites always, anyone
// else one in every LIVE_QUOTA_EVERY downloads, counted across both scouts.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, it, expect } from 'vitest';
import { favouriteIn, liveAllowed, countDownload, readQuota, recordDownload } from '../../scripts/scout-live.mjs';
import { LIVE_QUOTA_EVERY } from '../../renderer/js/content-preferences.js';

describe('favouriteIn', () => {
  const favourites = ['Candy Love', 'NoFaceGirl', 'ScyllaHMV', 'Noodledude'];

  it('matches whole words in any text, spaced or run together', () => {
    expect(favouriteIn(['INSANE compilation + Candy Love GODLIKE'], favourites)).toBe('Candy Love');
    expect(favouriteIn(['Amateur Babe No Face Girl rides'], favourites)).toBe('NoFaceGirl');
    expect(favouriteIn(['The Hana Song Experience [ScyllaHMV]'], favourites)).toBe('ScyllaHMV');
    expect(favouriteIn(['Damn Right PMV', 'noodledude'], favourites)).toBe('Noodledude');
  });

  it('does not match inside other names', () => {
    expect(favouriteIn(['Candy Lovers in Paris', 'Love Candy'], favourites)).toBeNull();
    expect(favouriteIn([null, '', undefined], favourites)).toBeNull();
  });
});

describe('live quota', () => {
  it('allows one non-favourite live scene in every LIVE_QUOTA_EVERY downloads', () => {
    const q = { downloads: 0, live: 0 };
    const allowedAt = [];
    for (let n = 1; n <= 3 * LIVE_QUOTA_EVERY; n++) {
      const live = liveAllowed(q);
      if (live) allowedAt.push(n);
      countDownload(q, live);
    }
    expect(allowedAt).toEqual([LIVE_QUOTA_EVERY, 2 * LIVE_QUOTA_EVERY, 3 * LIVE_QUOTA_EVERY]);
    expect(q).toEqual({ downloads: 3 * LIVE_QUOTA_EVERY, live: 3 });
  });

  it('does not bank unused slots beyond what the downloads earned', () => {
    expect(liveAllowed({ downloads: 2 * LIVE_QUOTA_EVERY - 1, live: 1 })).toBe(true);
    expect(liveAllowed({ downloads: 2 * LIVE_QUOTA_EVERY - 1, live: 2 })).toBe(false);
  });
});

describe('files', () => {
  let dir;
  beforeEach(() => { dir = fs.mkdtempSync(path.join(os.tmpdir(), 'scout-live-')); });
  afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

  it('counts downloads from both scouts in one file', () => {
    recordDownload(dir, false);
    recordDownload(dir, true);
    expect(readQuota(dir)).toEqual({ downloads: 2, live: 1 });
  });
});
