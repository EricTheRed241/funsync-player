// TCode device profiles — one TCode device active at a time (MiraBot over
// Wi-Fi, FUNSR MINI over USB, or the user's own settings), each with its own
// connection, precision, update rate, axis settings, offset and auto-connect.
//
// The active profile's values live in the plain `tcode.*` settings every
// other part of the app already reads; `tcode.profiles` keeps the others
// (`{ active, saved: { <id>: <snapshot> } }`). Switching saves the current
// `tcode.*` values under the old profile and loads the new one's saved
// values, or its defaults the first time. Pure: no settings or DOM access.

/** Every `tcode.*` setting that belongs to a device rather than the app. */
export const PROFILE_FIELDS = [
  'transport', 'port', 'baudRate', 'udpHost', 'udpPort', 'wsUrl', 'remoteBaudRate',
  'precision', 'updateRateHz', 'axes', 'defaultOffset', 'defaultOffsetSource',
  'autoConnect', 'funsr',
];

// The store's defaults (electron/store.js) plus the fields it leaves unset:
// what a profile starts from before its own defaults apply.
const BASE = {
  transport: 'serial',
  port: '',
  baudRate: 115200,
  udpHost: '',
  udpPort: 0,
  wsUrl: '',
  remoteBaudRate: 115200,
  precision: 3,
  updateRateHz: 60,
  axes: {},
  defaultOffset: null,
  defaultOffsetSource: null,
  autoConnect: null,
  funsr: null,
};

const ALL_AXES = ['L0', 'L1', 'L2', 'R0', 'R1', 'R2', 'V0', 'V1', 'A0', 'A1', 'A2'];

/** Axis settings with only `on` enabled, plus per-axis overrides. */
function axesOnly(on, overrides = {}) {
  const axes = {};
  for (const a of ALL_AXES) axes[a] = { enabled: on.includes(a), ...(overrides[a] || {}) };
  return axes;
}

/**
 * Built-in profiles, in menu order. `custom` has no defaults: it holds
 * whatever was set before profiles existed, or any other TCode device.
 */
export const TCODE_PROFILES = [
  { id: 'custom', nameKey: 'connection.tcode.profileCustom', defaults: {} },
  {
    id: 'mirabot',
    name: 'MiraBot',
    // 4-axis MiraBot Lite over its TCode WebSocket (mDNS name: DHCP moves
    // the IP), TCode 0.3 at 60 Hz.
    defaults: {
      transport: 'websocket',
      wsUrl: 'ws://mirabot.local:8888',
      precision: 4,
      updateRateHz: 60,
      axes: axesOnly(['L0', 'R0', 'R1', 'R2']),
    },
  },
  {
    id: 'funsr-mini',
    name: 'FUNSR MINI',
    // USB serial at 115200, stroke + twist module. The stroke keeps 5 %
    // off each end: driving the arms into the limit stops stalls the
    // motor, which heats up and damages it (FUNSR MINI manual).
    defaults: {
      transport: 'serial',
      baudRate: 115200,
      remoteBaudRate: 115200,
      precision: 4,
      updateRateHz: 60,
      axes: axesOnly(['L0', 'R0'], { L0: { min: 5, max: 95 } }),
    },
  },
];

export const DEFAULT_PROFILE_ID = 'custom';

export function getProfile(id) {
  return TCODE_PROFILES.find((p) => p.id === id) || null;
}

const clone = (v) => (v == null ? v : JSON.parse(JSON.stringify(v)));

/** The profile fields of a `tcode` settings object, missing ones filled from BASE. */
export function snapshotProfile(tcode = {}) {
  const snap = {};
  for (const f of PROFILE_FIELDS) snap[f] = clone(tcode[f] !== undefined ? tcode[f] : BASE[f]);
  return snap;
}

/** A profile's first-use values: BASE overlaid with its defaults. */
export function profileDefaults(id) {
  const defaults = getProfile(id)?.defaults || {};
  return snapshotProfile({ ...BASE, ...clone(defaults) });
}

/**
 * The active profile id from `tcode.profiles`. Before profiles existed the
 * settings were the user's own: a MiraBot WebSocket URL makes them the
 * MiraBot profile, anything else Custom.
 */
export function activeProfileId(tcode = {}) {
  const active = tcode.profiles?.active;
  if (getProfile(active)) return active;
  if (tcode.transport === 'websocket' && /mirabot/i.test(tcode.wsUrl || '')) return 'mirabot';
  return DEFAULT_PROFILE_ID;
}

/**
 * Switch from the active profile to `targetId`.
 * @param {object} tcode — the current `tcode` settings object
 * @returns {{ fields: object, profiles: object } | null} — `fields` are the
 *   new values for every PROFILE_FIELDS key, `profiles` the new
 *   `tcode.profiles`; null when the target is unknown or already active.
 */
export function switchProfile(tcode = {}, targetId) {
  if (!getProfile(targetId)) return null;
  const current = activeProfileId(tcode);
  if (current === targetId) return null;
  const saved = { ...(tcode.profiles?.saved || {}) };
  saved[current] = snapshotProfile(tcode);
  const fields = saved[targetId] ? snapshotProfile(saved[targetId]) : profileDefaults(targetId);
  return { fields, profiles: { active: targetId, saved } };
}

// --- FUNSR MINI device settings ---------------------------------------
//
// The firmware takes three settings commands on its TCode line:
//   DAG<x>  stroke length (global gain), 1.0 = longest … 3.0 = shortest
//   DKP<x>  speed: higher is faster and louder, 0.5 … 5.0
//   DSP<n>  where L0 sits at power-on: 1 bottom, 2 middle, 3 top
// The manual doesn't say whether the device keeps them across a power
// cycle, so the app sends the ones the user set at every connect.

export const FUNSR_LIMITS = {
  dag: { min: 1.0, max: 3.0, step: 0.05, default: 1.53 },
  dkp: { min: 0.5, max: 5.0, step: 0.1, default: 1.2 },
  dsp: { values: [1, 2, 3], default: 1 },
};

// Stroke length in cm per DAG value, from the manual's reference table.
const DAG_STROKE_CM = [
  [1.0, 17.92], [1.1, 16.48], [1.2, 15.27], [1.3, 14.23], [1.4, 13.31],
  [1.5, 12.51], [1.6, 11.79], [1.7, 11.13], [1.8, 10.55], [1.9, 10.01],
  [2.0, 9.53], [2.1, 9.08], [2.2, 8.67], [2.3, 8.30], [2.4, 7.96],
  [2.5, 7.64], [2.6, 7.35], [2.7, 7.08], [2.8, 6.83], [2.9, 6.60],
  [3.0, 6.39],
];

/** Approximate stroke length (cm) for a DAG value, interpolated from the manual's table. */
export function funsrStrokeCm(dag) {
  const x = Math.min(FUNSR_LIMITS.dag.max, Math.max(FUNSR_LIMITS.dag.min, Number(dag)));
  if (!Number.isFinite(x)) return null;
  for (let i = 1; i < DAG_STROKE_CM.length; i++) {
    const [x1, y1] = DAG_STROKE_CM[i];
    if (x <= x1 + 1e-9) {
      const [x0, y0] = DAG_STROKE_CM[i - 1];
      return y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
    }
  }
  return DAG_STROKE_CM[DAG_STROKE_CM.length - 1][1];
}

const clampTo = (v, { min, max }) => Math.min(max, Math.max(min, v));

/**
 * The command lines for the settings the user set (unset ones are left to
 * the device). Out-of-range values are clamped; garbage is skipped.
 * @param {{dag?: number, dkp?: number, dsp?: number} | null} funsr
 * @returns {string[]} e.g. ['DAG1.53\n', 'DKP1.2\n', 'DSP1\n']
 */
export function funsrCommands(funsr) {
  if (!funsr || typeof funsr !== 'object') return [];
  const lines = [];
  const dag = Number(funsr.dag);
  if (funsr.dag != null && Number.isFinite(dag)) lines.push(`DAG${clampTo(dag, FUNSR_LIMITS.dag).toFixed(2)}\n`);
  const dkp = Number(funsr.dkp);
  if (funsr.dkp != null && Number.isFinite(dkp)) lines.push(`DKP${clampTo(dkp, FUNSR_LIMITS.dkp).toFixed(1)}\n`);
  const dsp = Number(funsr.dsp);
  if (FUNSR_LIMITS.dsp.values.includes(dsp)) lines.push(`DSP${dsp}\n`);
  return lines;
}
