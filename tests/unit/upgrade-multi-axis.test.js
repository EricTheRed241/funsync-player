/**
 * @vitest-environment node
 * Pure planner — no DOM. See notes/CLAUDE.md "Test environments".
 */
// Stroke-only → multi-axis companion upgrade. Writes the same motion-source
// values live playback already generates, as OFS-canonical companions the
// library scanner already round-trips. Original stroke file is never a write.

import { describe, it, expect } from 'vitest';
import { createNoise1D } from '../../renderer/js/motion-source.js';
import { detectCompanionFiles } from '../../renderer/js/multi-axis.js';
import { extractEmbeddedAxes } from '../../renderer/js/embedded-multi-axis.js';
import {
  planUpgrade,
  serialiseUpgradeWrites,
  existingSuffixesFromFiles,
  companionPathForScript,
  resolveUpgradeAxes,
  depthWeights,
  depthSchedule,
  REST_LIMIT_MS,
} from '../../renderer/js/upgrade-multi-axis.js';

const STROKE = Object.freeze([
  { at: 0, pos: 5 },
  { at: 400, pos: 95 },
  { at: 800, pos: 10 },
  { at: 1200, pos: 90 },
]);

const PRIMARY = '/lib/clip.funscript';

function plan(extra = {}) {
  return planUpgrade({
    primaryPath: PRIMARY,
    strokeActions: STROKE.map((a) => ({ ...a })),
    noise: createNoise1D(42),
    ...extra,
  });
}

describe('planUpgrade', () => {
  it('gives a stroke-only fixture twist, roll, and pitch companions', () => {
    const result = plan();
    expect(result.error).toBeNull();
    expect(result.writes.map((w) => w.suffix)).toEqual(['twist', 'roll', 'pitch']);
    expect(result.writes.map((w) => w.tcode)).toEqual(['R0', 'R1', 'R2']);
    expect(result.writes.map((w) => w.mode)).toEqual(['link', 'random', 'pattern']);
  });

  it('does not invent surge/sway/valve unless those axes are enabled', () => {
    expect(plan().writes.some((w) => w.suffix === 'surge')).toBe(false);
    const withSurge = plan({ enabledTcodes: new Set(['L1']) });
    expect(withSurge.writes.map((w) => w.suffix)).toEqual(['twist', 'roll', 'pitch', 'surge']);
  });

  it('leaves the stroke track unchanged', () => {
    const stroke = STROKE.map((a) => ({ ...a }));
    const snapshot = STROKE.map((a) => ({ ...a }));
    const result = plan({ strokeActions: stroke });
    expect(stroke).toEqual(snapshot);
    expect(result.writes.some((w) => w.path === PRIMARY)).toBe(false);
    expect(result.writes.every((w) => w.path !== PRIMARY)).toBe(true);
  });

  it('twist keeps stroke timestamps and a stepped depth', () => {
    const twist = plan().writes.find((w) => w.suffix === 'twist');
    expect(twist.actions.map((a) => a.at)).toEqual(STROKE.map((a) => a.at));
    expect(twist.segments.every((s) => [0, 25, 50, 75, 100].includes(s.depth))).toBe(true);
  });

  it('lays smooth noise on twist so it is not a pure scale of the stroke', () => {
    const twist = planUpgrade({
      primaryPath: PRIMARY,
      strokeActions: STROKE.map((a) => ({ ...a })),
      noise: () => 0.5,
    }).writes.find((w) => w.suffix === 'twist');
    const pure = STROKE.map((a) => 50 + (a.pos - 50) * (twist.segments[0].depth / 100));
    expect(twist.actions.map((a) => a.pos)).not.toEqual(pure);
  });

  it('does not clobber an existing twist file on a second run', () => {
    const first = plan();
    const names = [PRIMARY, ...first.writes.map((w) => w.path)];
    const existing = existingSuffixesFromFiles(PRIMARY, names);
    expect(existing.has('twist')).toBe(true);

    const second = plan({ existingSuffixes: new Set(['twist']) });
    expect(second.writes.map((w) => w.suffix)).toEqual(['roll', 'pitch']);
    expect(second.skipped).toEqual([
      expect.objectContaining({ suffix: 'twist', reason: 'exists', path: companionPathForScript(PRIMARY, 'twist') }),
    ]);
  });

  it('random track is deterministic for a fixed injected noise function', () => {
    const a = plan({ noise: createNoise1D(99) }).writes.find((w) => w.suffix === 'roll');
    const b = plan({ noise: createNoise1D(99) }).writes.find((w) => w.suffix === 'roll');
    expect(a.actions).toEqual(b.actions);
    const other = plan({ noise: createNoise1D(3) }).writes.find((w) => w.suffix === 'roll');
    expect(other.actions).not.toEqual(a.actions);
  });

  it('clamps every generated position to 0-100', () => {
    for (const w of plan().writes) {
      expect(w.actions.length).toBe(STROKE.length);
      for (const a of w.actions) {
        expect(a.pos).toBeGreaterThanOrEqual(0);
        expect(a.pos).toBeLessThanOrEqual(100);
      }
    }
  });

  it('roll and pitch are not copies of the stroke', () => {
    const result = plan();
    const strokePos = STROKE.map((a) => a.pos);
    expect(result.writes.find((w) => w.suffix === 'roll').actions.map((a) => a.pos)).not.toEqual(strokePos);
    expect(result.writes.find((w) => w.suffix === 'pitch').actions.map((a) => a.pos)).not.toEqual(strokePos);
  });

  it('refuses a missing path or a short stroke', () => {
    expect(planUpgrade({ strokeActions: STROKE }).error).toBe('no-path');
    expect(plan({ strokeActions: [{ at: 0, pos: 50 }] }).error).toBe('need-actions');
  });
});

describe('depth schedule', () => {
  const longStroke = Array.from({ length: 61 }, (_, i) => ({
    at: i * 1000,
    pos: i % 2 === 0 ? 0 : 100,
  }));

  it('prefers low steps early and high steps late', () => {
    const early = depthWeights(0);
    const late = depthWeights(1);
    expect(early[0]).toBeGreaterThan(early[4]);
    expect(late[4]).toBeGreaterThan(late[0]);
  });

  it('never holds 0% longer than 10s, even if the noise always asks for rest', () => {
    const segments = depthSchedule(longStroke, { ramp: 'up', noise: () => -1, phase: 0 });
    let rest = 0;
    for (const s of segments) {
      const dur = Math.min(s.until, 60_000) - s.from;
      if (s.depth === 0) rest += dur;
      else rest = 0;
      expect(rest).toBeLessThanOrEqual(REST_LIMIT_MS);
    }
    expect(segments.some((s) => s.depth === 0)).toBe(true);
    expect(segments.some((s) => s.depth > 0)).toBe(true);
  });
});

describe('scanner round-trip', () => {
  it('companions are detected next to the unchanged stroke file', () => {
    const result = plan();
    const files = serialiseUpgradeWrites(result.writes);
    expect(files.map((f) => f.suffix).sort()).toEqual(['pitch', 'roll', 'twist']);

    const dirListing = [
      'clip.funscript',
      ...result.writes.map((w) => w.path.split('/').pop()),
    ];
    const detected = detectCompanionFiles('clip.funscript', dirListing);
    expect(detected.map((d) => d.axis.suffix).sort()).toEqual(['pitch', 'roll', 'twist']);
    expect(dirListing).toContain('clip.generated.twist.funscript');
    expect(parseAxisFromListing(dirListing, 'clip.funscript')).toBeNull();

    // Original stroke is still stroke-only — no embedded extra axes.
    const original = { version: '1.0', actions: STROKE.map((a) => ({ ...a })) };
    expect(extractEmbeddedAxes(original).size).toBe(0);

    for (const f of files) {
      expect(f.content.version).toBe('1.0');
      expect(f.content.actions.every((a) => Number.isFinite(a.at) && Number.isFinite(a.pos))).toBe(true);
      expect(extractEmbeddedAxes(f.content).size).toBe(0);
    }
  });
});

describe('resolveUpgradeAxes / companionPathForScript', () => {
  it('stems off the primary even if handed a companion path', () => {
    expect(companionPathForScript('/v/clip.twist.funscript', 'roll'))
      .toBe('/v/clip.generated.roll.funscript');
  });

  it('optional axes stay off by default', () => {
    expect(resolveUpgradeAxes().map((a) => a.suffix)).toEqual(['twist', 'roll', 'pitch']);
  });
});

function parseAxisFromListing(listing, name) {
  const hit = listing.find((n) => n === name);
  if (!hit) return null;
  return detectCompanionFiles(name, listing).find((c) => c.path === name) || null;
}
