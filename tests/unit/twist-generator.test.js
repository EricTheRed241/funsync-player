/**
 * @vitest-environment node
 * Pure generator — no DOM.
 */
// The generated twist: guided by a stroke model, styled by fitted knobs,
// randomness seeded per scene.

import { describe, it, expect } from 'vitest';
import {
  TWIST_MODEL,
  generateTwist,
  guidedTrack,
  strokeSeries,
  strokeTemplates,
} from '../../renderer/js/twist-generator.js';

const HZ = TWIST_MODEL.hz;

/** Strokes whose speed drifts between 0.6 and 1.6 per second, `seconds` long. */
function stroke(seconds) {
  const out = [];
  let phase = 0;
  for (let ms = 0; ms <= seconds * 1000; ms += 50) {
    const s = ms / 1000;
    phase += 2 * Math.PI * (1.1 + 0.5 * Math.sin(s / 20)) * 0.05;
    out.push({ at: ms, pos: Math.round(50 + 45 * Math.sin(phase)) });
  }
  return out;
}

function corr(a, b) {
  const ma = a.reduce((s, v) => s + v, 0) / a.length;
  const mb = b.reduce((s, v) => s + v, 0) / b.length;
  let ab = 0; let aa = 0; let bb = 0;
  for (let i = 0; i < a.length; i++) {
    ab += (a[i] - ma) * (b[i] - mb); aa += (a[i] - ma) ** 2; bb += (b[i] - mb) ** 2;
  }
  return ab / Math.sqrt(aa * bb);
}

const std = (x) => {
  const m = x.reduce((s, v) => s + v, 0) / x.length;
  return Math.sqrt(x.reduce((s, v) => s + (v - m) ** 2, 0) / x.length);
};

describe('stroke templates', () => {
  it('the quarter shape is the stroke shifted by 90° (sin → −cos)', () => {
    const n = 20 * HZ;
    const L = Float64Array.from({ length: n }, (_, i) => 50 + 40 * Math.sin(2 * Math.PI * i / HZ));
    const T = strokeTemplates(L);
    const mid = Array.from({ length: n / 2 }, (_, i) => i + n / 4);
    const negCos = mid.map((i) => -40 * Math.cos(2 * Math.PI * i / HZ));
    expect(corr(mid.map((i) => T.quarter[i]), negCos)).toBeGreaterThan(0.99);
    expect(std(mid.map((i) => T.quarter[i]))).toBeCloseTo(std(negCos), 0);
  });
});

describe('generateTwist', () => {
  const script = stroke(180);

  it('writes a playable script over the stroke span: 0–100, in order, a point every 2 s or sooner (to the sample)', () => {
    const { actions } = generateTwist(script, { vr: false, seed: 7 });
    expect(actions.length).toBeGreaterThan(50);
    expect(actions[0].at).toBe(0);
    expect(actions[actions.length - 1].at).toBeLessThanOrEqual(180_000);
    for (let i = 0; i < actions.length; i++) {
      expect(Number.isInteger(actions[i].pos)).toBe(true);
      expect(actions[i].pos).toBeGreaterThanOrEqual(0);
      expect(actions[i].pos).toBeLessThanOrEqual(100);
      if (i > 0) {
        expect(actions[i].at).toBeGreaterThanOrEqual(actions[i - 1].at);
        expect(actions[i].at - actions[i - 1].at).toBeLessThanOrEqual(2000 + Math.ceil(1000 / HZ));
      }
    }
  });

  it('the same seed gives the same twist, another seed another one', () => {
    const a = generateTwist(script, { seed: 11 }).actions;
    expect(generateTwist(script, { seed: 11 }).actions).toEqual(a);
    expect(generateTwist(script, { seed: 12 }).actions).not.toEqual(a);
  });

  it('VR twist stays gentler than flat twist', () => {
    const spread = (vr) => [1, 2, 3].map((seed) => std(generateTwist(script, { vr, seed }).actions.map((a) => a.pos)));
    const vr = spread(true);
    const flat = spread(false);
    expect(Math.max(...vr)).toBeLessThan(Math.min(...flat));
  });

  it('records which generator made it', () => {
    expect(generateTwist(script, { vr: true }).generator).toBe(TWIST_MODEL.groups.vr.generator);
    expect(generateTwist(script, { vr: false }).generator).toBe(TWIST_MODEL.groups.flat.generator);
  });

  it('returns nothing for a stroke without a span', () => {
    expect(generateTwist([{ at: 0, pos: 10 }]).actions).toEqual([]);
  });
});

describe('guided steering', () => {
  // One tree on the stroke itself: the prediction is +1 while the stroke is
  // above its mean, −1 below (or the reverse), so every stretch should take
  // the direction that moves with (or against) the stroke.
  const lag0 = TWIST_MODEL.features.indexOf('stroke_lag0');
  const { L } = strokeSeries(stroke(120));
  const T = strokeTemplates(L);
  const settings = {
    habits: false,
    knobs: { p_copy: 1, p_quarter: 0, p_alt: 0, p_rest: 0, seg: 3, gain: 10, drift: 0.1, tau: 10, centre: 0 },
    trust: 1,
    follow: 1,
  };
  const pred = (sign) => Float64Array.from(L, (v) => (v >= 50 ? sign : -sign));

  it('stretches follow the prediction\'s direction', () => {
    expect(lag0).toBeGreaterThanOrEqual(0);
    expect(corr(Array.from(guidedTrack(L, T, pred(1), settings, 5)), Array.from(T.copy))).toBeGreaterThan(0.8);
    expect(corr(Array.from(guidedTrack(L, T, pred(-1), settings, 5)), Array.from(T.copy))).toBeLessThan(-0.8);
  });

  it('with no trust the direction is a coin flip per stretch', () => {
    const r = corr(Array.from(guidedTrack(L, T, pred(1), { ...settings, trust: 0 }, 5)), Array.from(T.copy));
    expect(Math.abs(r)).toBeLessThan(0.6);
  });
});
