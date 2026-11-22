// twist-generator — the generated twist for `upgrade:axes` (user, 2026-11-21/22).
//
// Three parts, chosen in the twist experiments (33 VR / 111 flat scenes with
// hand-made twist, scored 0.5 · style match with the liked scenes + 0.5 ·
// timing against the hand-made twist, 5-fold):
//   preference  the twist is built from stretches of stroke-derived shapes
//               (copy, quarter phase, every other stroke, rest; flat also a
//               flick at stroke tops and more rest while the stroke is busy),
//               their mix, length, size and slow lean fitted so the result
//               looks like the twist of the scenes you like (favourite, 4★+,
//               top-quarter score)
//   guided      a stroke model (gradient-boosted trees over the stroke, its
//               shapes and speed) predicts where the twist goes; each stretch
//               takes the direction that agrees with it (probability `trust`)
//               and the slow lean follows its slow part (share `follow`)
//   random      everything else is drawn from a PRNG seeded per scene, so a
//               scene always gets the same twist and two scenes never share one
// VR and flat have their own settings (`twist-model.json`, exported by the
// axes lab's export_twist_model.py). Scores: VR 71.5, flat 67.0, against 38.3
// and 45.7 for the earlier random twist.
//
// Pure: no I/O. Signal maths follows NumPy / SciPy (interp, uniform_filter1d,
// hilbert, unwrap, gradient) so the exported model sees the features it was
// trained on.

import TWIST_MODEL from './twist-model.json' with { type: 'json' };
import { mulberry32 } from './motion-source.js';

export { TWIST_MODEL };

const HZ = TWIST_MODEL.hz;

// ---------------------------------------------------------------- signal maths

function mean(x) {
  let s = 0;
  for (let i = 0; i < x.length; i++) s += x[i];
  return x.length ? s / x.length : 0;
}

function std(x) {
  const m = mean(x);
  let s = 0;
  for (let i = 0; i < x.length; i++) s += (x[i] - m) ** 2;
  return x.length ? Math.sqrt(s / x.length) : 0;
}

/** Linear percentile (NumPy's default). */
function percentile(x, q) {
  const v = Float64Array.from(x).sort();
  const pos = (q / 100) * (v.length - 1);
  const lo = Math.floor(pos);
  return lo + 1 < v.length ? v[lo] + (v[lo + 1] - v[lo]) * (pos - lo) : v[lo];
}

/** SciPy uniform_filter1d: window [i − ⌊size/2⌋, i + size − ⌊size/2⌋ − 1],
 *  edges 'reflect' (d c b a | a b c d | d c b a) or 'nearest' (a a | a b c d | d d). */
export function uniformFilter(x, size, mode = 'reflect') {
  const n = x.length;
  const out = new Float64Array(n);
  if (n === 0) return out;
  const at = mode === 'nearest'
    ? (k) => x[k < 0 ? 0 : k >= n ? n - 1 : k]
    : (k) => {
      const m = ((k % (2 * n)) + 2 * n) % (2 * n);
      return x[m < n ? m : 2 * n - 1 - m];
    };
  const left = Math.floor(size / 2);
  let s = 0;
  for (let k = -left; k < size - left; k++) s += at(k);
  for (let i = 0; i < n; i++) {
    out[i] = s / size;
    s += at(i + size - left) - at(i - left);
  }
  return out;
}

function fftPow2(re, im, inverse) {
  const n = re.length;
  for (let i = 1, j = 0; i < n; i++) {
    let bit = n >> 1;
    for (; j & bit; bit >>= 1) j ^= bit;
    j ^= bit;
    if (i < j) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }
  for (let len = 2; len <= n; len <<= 1) {
    const ang = (inverse ? 2 : -2) * Math.PI / len;
    for (let i = 0; i < n; i += len) {
      for (let k = 0; k < len / 2; k++) {
        const wr = Math.cos(ang * k);
        const wi = Math.sin(ang * k);
        const a = i + k;
        const b = a + len / 2;
        const xr = re[b] * wr - im[b] * wi;
        const xi = re[b] * wi + im[b] * wr;
        re[b] = re[a] - xr; im[b] = im[a] - xi;
        re[a] += xr; im[a] += xi;
      }
    }
  }
  if (inverse) {
    for (let i = 0; i < n; i++) { re[i] /= n; im[i] /= n; }
  }
}

/** Exact DFT of any length (Bluestein's chirp-z for lengths that aren't a power of 2). */
function dft(re, im, inverse = false) {
  const n = re.length;
  if ((n & (n - 1)) === 0) {
    fftPow2(re, im, inverse);
    return;
  }
  let m = 1;
  while (m < 2 * n - 1) m <<= 1;
  const sign = inverse ? 1 : -1;
  const wr = new Float64Array(n);
  const wi = new Float64Array(n);
  for (let k = 0; k < n; k++) {
    const ang = sign * Math.PI * ((k * k) % (2 * n)) / n;
    wr[k] = Math.cos(ang); wi[k] = Math.sin(ang);
  }
  const ar = new Float64Array(m); const ai = new Float64Array(m);
  const br = new Float64Array(m); const bi = new Float64Array(m);
  for (let k = 0; k < n; k++) {
    ar[k] = re[k] * wr[k] - im[k] * wi[k];
    ai[k] = re[k] * wi[k] + im[k] * wr[k];
  }
  br[0] = wr[0]; bi[0] = -wi[0];
  for (let k = 1; k < n; k++) {
    br[k] = br[m - k] = wr[k];
    bi[k] = bi[m - k] = -wi[k];
  }
  fftPow2(ar, ai, false);
  fftPow2(br, bi, false);
  for (let k = 0; k < m; k++) {
    const r = ar[k] * br[k] - ai[k] * bi[k];
    ai[k] = ar[k] * bi[k] + ai[k] * br[k];
    ar[k] = r;
  }
  fftPow2(ar, ai, true);
  for (let k = 0; k < n; k++) {
    const r = ar[k] * wr[k] - ai[k] * wi[k];
    const i = ar[k] * wi[k] + ai[k] * wr[k];
    re[k] = inverse ? r / n : r;
    im[k] = inverse ? i / n : i;
  }
}

/** Analytic signal (SciPy hilbert): real part = x, imaginary part = x shifted by 90°. */
export function hilbert(x) {
  const n = x.length;
  const re = Float64Array.from(x);
  const im = new Float64Array(n);
  dft(re, im);
  for (let k = 1; k < n; k++) {
    const h = (n % 2 === 0 && k === n / 2) ? 1 : k < n / 2 ? 2 : 0;
    re[k] *= h; im[k] *= h;
  }
  dft(re, im, true);
  return { re, im };
}

function unwrap(p) {
  const out = Float64Array.from(p);
  let corr = 0;
  for (let i = 1; i < p.length; i++) {
    const dd = p[i] - p[i - 1];
    if (Math.abs(dd) >= Math.PI) {
      let mod = ((dd + Math.PI) % (2 * Math.PI) + 2 * Math.PI) % (2 * Math.PI) - Math.PI;
      if (mod === -Math.PI && dd > 0) mod = Math.PI;
      corr += mod - dd;
    }
    out[i] = p[i] + corr;
  }
  return out;
}

// ---------------------------------------------------------------- the scene

/** Stroke positions sampled at HZ over the scripted span (times in seconds). */
export function strokeSeries(actions) {
  const a = (actions || [])
    .filter((x) => x && Number.isFinite(x.at) && Number.isFinite(x.pos))
    .sort((p, q) => p.at - q.at);
  if (a.length < 2) return { t: new Float64Array(0), L: new Float64Array(0) };
  const start = a[0].at / 1000;
  const n = Math.max(0, Math.ceil((a[a.length - 1].at / 1000 - start) * HZ));
  const t = new Float64Array(n);
  const L = new Float64Array(n);
  let j = 0;
  for (let i = 0; i < n; i++) {
    const s = start + i / HZ;
    t[i] = s;
    while (j < a.length - 2 && a[j + 1].at / 1000 <= s) j++;
    const x0 = a[j].at / 1000; const x1 = a[j + 1].at / 1000;
    L[i] = x1 > x0 ? a[j].pos + (a[j + 1].pos - a[j].pos) * Math.min(1, Math.max(0, (s - x0) / (x1 - x0))) : a[j + 1].pos;
  }
  return { t, L };
}

/** The stroke's shapes a scripter builds twist from. */
export function strokeTemplates(L) {
  const n = L.length;
  const local = uniformFilter(L, 4 * HZ, 'nearest');
  const copy = new Float64Array(n);
  for (let i = 0; i < n; i++) copy[i] = L[i] - local[i];
  const a = hilbert(copy);
  const amp = new Float64Array(n);
  const ang = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    amp[i] = Math.hypot(a.re[i], a.im[i]);
    ang[i] = Math.atan2(a.im[i], a.re[i]);
  }
  const ph = unwrap(ang);
  const env = uniformFilter(amp, 2 * HZ, 'nearest');
  const slowL = uniformFilter(L, 8 * HZ, 'nearest');
  const mL = mean(L); const mEnv = mean(env);
  const alternate = new Float64Array(n); const alternateQ = new Float64Array(n);
  const slow = new Float64Array(n); const envelope = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    alternate[i] = env[i] * Math.cos(ph[i] / 2);
    alternateQ[i] = env[i] * Math.sin(ph[i] / 2);
    slow[i] = slowL[i] - mL;
    envelope[i] = env[i] - mEnv;
  }
  return { copy, quarter: a.im, alternate, alternate_q: alternateQ, slow, envelope };
}

/** Model inputs per sample, each standardised over the scene (order = TWIST_MODEL.features). */
export function twistFeatures(L, T) {
  const n = L.length;
  const c = Float64Array.from(L, (v) => v - 50);
  const cols = [];
  for (let k = -60; k <= 60; k += 15) {
    cols.push(Float64Array.from({ length: n }, (_, i) => c[(((i + k) % n) + n) % n]));
  }
  const speed = new Float64Array(n);
  for (let i = 0; i < n; i++) {
    const d = n < 2 ? 0 : i === 0 ? c[1] - c[0] : i === n - 1 ? c[n - 1] - c[n - 2] : (c[i + 1] - c[i - 1]) / 2;
    speed[i] = d * HZ;
  }
  cols.push(T.quarter, T.alternate, T.alternate_q, T.slow, T.envelope, speed);
  return cols.map((col) => {
    const m = mean(col); const s = std(col) + 1e-9;
    return Float64Array.from(col, (v) => (v - m) / s);
  });
}

/** Gradient-boosted trees: baseline + Σ leaves; node [f, threshold, left, right] or leaf [value]. */
export function predictTrees(model, cols) {
  const n = cols.length ? cols[0].length : 0;
  const y = new Float64Array(n).fill(model.baseline);
  for (const nodes of model.trees) {
    for (let r = 0; r < n; r++) {
      let i = 0;
      while (nodes[i].length === 4) {
        const [f, thr, l, rr] = nodes[i];
        i = cols[f][r] <= thr ? l : rr;
      }
      y[r] += nodes[i][0];
    }
  }
  return y;
}

// ---------------------------------------------------------------- generator

function randomSource(seed) {
  const u = mulberry32(seed);
  return {
    uniform: u,
    exponential: (scale) => -scale * Math.log(1 - u()),
    normal: () => Math.sqrt(-2 * Math.log(1 - u())) * Math.cos(2 * Math.PI * u()),
    pick: (w) => {
      let x = u() * w.reduce((s, v) => s + v, 0);
      for (let i = 0; i < w.length; i++) {
        x -= w[i];
        if (x < 0) return i;
      }
      return w.length - 1;
    },
    coin: () => (u() < 0.5 ? -1 : 1),
  };
}

const unit = (x) => {
  const s = std(x) + 1e-9;
  return Float64Array.from(x, (v) => v / s);
};

/**
 * Guided twist track at HZ (0–100) for stroke samples L.
 * @param {Float64Array} L
 * @param {object} T  strokeTemplates(L)
 * @param {Float64Array} pred  the stroke model's prediction
 * @param {{habits:boolean, knobs:object, trust:number, follow:number}} g
 * @param {number} seed
 */
export function guidedTrack(L, T, pred, g, seed) {
  const n = L.length;
  const kn = g.knobs;
  const rng = randomSource(seed);
  const zeros = new Float64Array(n);
  const shapes = [unit(T.copy), unit(T.quarter), unit(T.alternate)];
  let base = [kn.p_copy, kn.p_quarter, kn.p_alt];
  let act = null;
  if (g.habits) {
    // Flicks at stroke tops, and more rest while the stroke is busy (kn.quiet).
    shapes.push(unit(Float64Array.from(shapes[0], (v) => Math.max(v, 0) ** 2)));
    base.push(kn.p_flick);
    const lo = percentile(T.envelope, 10); const hi = percentile(T.envelope, 90);
    act = Float64Array.from(T.envelope, (v) => Math.min(1, Math.max(0, (v - lo) / (hi - lo + 1e-9))));
  }
  shapes.push(zeros);
  base.push(kn.p_rest);
  base = base.map((w) => w + 1e-6);
  const minLen = g.habits ? HZ / 2 : HZ;

  const y = new Float64Array(n);
  for (let i = 0; i < n;) {
    const len = Math.max(minLen, Math.floor(rng.exponential(kn.seg) * HZ));
    const w = base.slice();
    if (act) w[w.length - 1] *= Math.exp(kn.quiet * (act[i] - 0.5) * 2);
    const j = rng.pick(w);
    const end = Math.min(n, i + len);
    let pm = 0;
    for (let k = i; k < end; k++) pm += pred[k];
    pm /= end - i;
    let dot = 0;
    for (let k = i; k < end; k++) dot += shapes[j][k] * (pred[k] - pm);
    const agree = dot < 0 ? -1 : 1;
    const sgn = rng.uniform() < g.trust ? agree : rng.coin();
    for (let k = i; k < end; k++) y[k] = sgn * kn.gain * shapes[j][k];
    i = end;
  }
  const ys = uniformFilter(y, Math.max(1, Math.floor(HZ / 6)));
  const tau = Math.max(1, Math.floor(kn.tau * HZ));
  const raw = Float64Array.from({ length: n + 4 * tau }, () => rng.normal());
  const d = unit(uniformFilter(raw, tau).subarray(2 * tau, 2 * tau + n));
  const ps = uniformFilter(pred, tau);
  const pm = mean(ps); const psd = std(ps) + 1e-9;
  const lean = Float64Array.from(d, (v, i) => g.follow * (ps[i] - pm) / psd + (1 - g.follow) * v);
  const leanSd = std(lean) + 1e-9;
  return Float64Array.from(ys, (v, i) => Math.min(100, Math.max(0, 50 + kn.centre + v + lean[i] / leanSd * kn.drift)));
}

/** HZ track → funscript points at its turns (a swing of ≥ 2) and at least every 2 s. */
export function trackToActions(t, y) {
  const n = y.length;
  if (n < 2) return [];
  const keep = [0];
  let ext = 0; let dir = 0;
  for (let i = 1; i < n; i++) {
    if ((dir >= 0 && y[i] >= y[ext]) || (dir <= 0 && y[i] <= y[ext])) {
      if (dir === 0 && Math.abs(y[i] - y[0]) >= 2) dir = y[i] > y[0] ? 1 : -1;
      if (dir !== 0) ext = i;
    } else if (Math.abs(y[i] - y[ext]) >= 2) {
      keep.push(ext); dir = -dir; ext = i;
    }
  }
  keep.push(n - 1);
  const out = [];
  let last = null;
  for (const i of keep) {
    while (last != null && t[i] - last > 2) {
      last += 2;
      let lo = 0; let hi = n - 1;                      // first sample at or after `last`
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if (t[mid] < last) lo = mid + 1; else hi = mid;
      }
      const j = lo;
      out.push([t[j], y[j]]);
    }
    out.push([t[i], y[i]]);
    last = t[i];
  }
  return out.map(([s, p]) => ({ at: Math.round(s * 1000), pos: Math.round(p) }));
}

/**
 * The generated twist for a stroke script.
 * @param {Array<{at:number, pos:number}>} strokeActions
 * @param {{vr?: boolean, seed?: number, model?: object}} [opts]
 * @returns {{actions: Array<{at:number, pos:number}>, generator: string}}
 */
export function generateTwist(strokeActions, { vr = false, seed = 1337, model = TWIST_MODEL } = {}) {
  const g = model.groups[vr ? 'vr' : 'flat'];
  const { t, L } = strokeSeries(strokeActions);
  if (L.length < 2) return { actions: [], generator: g.generator };
  const T = strokeTemplates(L);
  const pred = predictTrees(g.model, twistFeatures(L, T));
  return { actions: trackToActions(t, guidedTrack(L, T, pred, g, seed)), generator: g.generator };
}
