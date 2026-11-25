// Local device dashboard. Polls GET /api/device/telemetry once a second
// (each poll also keeps the desktop's sample stream alive) and draws the
// device link, the sync engine, the playback source and what was sent.

const POLL_MS = 1000;
const RATE_WINDOW_MS = 15 * 60 * 1000;
const AXIS_WINDOW_MS = 30 * 1000;
const RATE_AVERAGE_MS = 5000;
const EVENT_KEEP = 500;
const AXIS_COLORS = {
  L0: '#5aa9ff', L1: '#a78bfa', L2: '#f472b6', R0: '#3ccf7a', R1: '#f0b429', R2: '#f97316',
  V0: '#22d3ee', V1: '#94a3b8', V2: '#e879f9', A0: '#facc15', A1: '#84cc16', A2: '#fb7185',
};

const state = {
  seq: 0, cmdSince: 0, eventSince: 0,
  points: [], commands: [], events: [],
  latest: null, latestAgeS: null, streaming: false, status: null,
  now: Date.now(), error: null, backendStartedAt: null,
};

// --- formatting --------------------------------------------------------------

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

function dur(ms) {
  if (ms == null || !Number.isFinite(ms)) return '—';
  const s = Math.abs(ms) / 1000;
  if (s < 1) return `${Math.round(Math.abs(ms))} ms`;
  if (s < 60) return `${s.toFixed(s < 10 ? 1 : 0)} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${String(Math.floor(s % 60)).padStart(2, '0')}s`;
  return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, '0')}m`;
}

function mmss(ms) {
  if (ms == null) return '—';
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const mm = String(Math.floor((s % 3600) / 60)).padStart(h ? 2 : 1, '0');
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

function clock(isoOrMs) {
  if (isoOrMs == null) return '—';
  const d = new Date(isoOrMs);
  return Number.isNaN(d.getTime()) ? '—' : d.toLocaleTimeString([], { hour12: false });
}

const since = (iso) => (iso ? state.now - Date.parse(iso) : null);

function bytes(n) {
  if (n == null) return '—';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function merge(body) {
  // The backend restarted (the app did): its cursors start over, so do ours.
  // This reply was cut with the old cursors; the next poll gets everything.
  const started = body.status?.backend?.startedAt ?? null;
  const restarted = state.backendStartedAt && started !== state.backendStartedAt;
  state.backendStartedAt = started;
  if (restarted) {
    Object.assign(state, { seq: 0, cmdSince: 0, eventSince: 0, points: [], commands: [], events: [] });
    return;
  }
  state.now = body.now;
  state.points.push(...body.points);
  state.points = state.points.filter((p) => p.t >= state.now - RATE_WINDOW_MS);
  state.seq = body.seq;
  state.commands.push(...body.commands);
  state.commands = state.commands.filter((c) => c.t >= state.now - AXIS_WINDOW_MS);
  if (state.commands.length) state.cmdSince = state.commands[state.commands.length - 1].t;
  state.events.push(...body.events);
  if (state.events.length > EVENT_KEEP) state.events = state.events.slice(-EVENT_KEEP);
  if (state.events.length) state.eventSince = state.events[state.events.length - 1].seq;
  state.latest = body.latest;
  state.latestAgeS = body.latestAgeS;
  state.streaming = body.streaming;
  state.status = body.status;
}

/** Commands per second between consecutive samples, from main's counter. */
function rateSeries() {
  const out = [];
  for (let i = 1; i < state.points.length; i++) {
    const a = state.points[i - 1];
    const b = state.points[i];
    const dt = (b.t - a.t) / 1000;
    if (dt <= 0 || dt > 10) { out.push([b.t, null]); continue; }
    const key = b.sent != null && a.sent != null ? 'sent' : 'handed';
    const d = (b[key] ?? 0) - (a[key] ?? 0);
    // A new connection restarts main's counter.
    out.push([b.t, d >= 0 ? d / dt : (b[key] ?? 0) / dt]);
  }
  return out;
}

function linkDownBands() {
  const bands = [];
  let start = null;
  for (const p of state.points) {
    const down = p.linkState !== 'connected';
    if (down && start == null) start = p.t;
    if (!down && start != null) { bands.push([start, p.t]); start = null; }
  }
  if (start != null) bands.push([start, state.now]);
  return bands;
}

/** TCode command → [{axis, value 0-100, ms}]. Value width = precision. */
function parseCommand(cmd) {
  const out = [];
  for (const m of cmd.matchAll(/\b([LRVA]\d)(\d+)(?:I(\d+))?/g)) {
    const max = 10 ** m[2].length - 1;
    out.push({ axis: m[1], value: (Number(m[2]) / max) * 100, ms: m[3] ? Number(m[3]) : 0 });
  }
  return out;
}

/** Each axis's targets, plotted where the device should reach them. */
function axisSeries() {
  const byAxis = new Map();
  for (const c of state.commands) {
    for (const { axis, value, ms } of parseCommand(c.cmd)) {
      if (!byAxis.has(axis)) byAxis.set(axis, []);
      byAxis.get(axis).push([c.t + ms, value]);
    }
  }
  for (const pts of byAxis.values()) pts.sort((a, b) => a[0] - b[0]);
  return byAxis;
}

function recentRate() {
  const from = state.now - RATE_AVERAGE_MS;
  return state.commands.filter((c) => c.t > from).length / (RATE_AVERAGE_MS / 1000);
}

function mediaAdvancing() {
  const pts = state.points.filter((p) => p.mediaMs != null);
  if (pts.length < 3) return null;
  const a = pts[pts.length - 3];
  const b = pts[pts.length - 1];
  return b.mediaMs !== a.mediaMs;
}

// --- checks ------------------------------------------------------------------

function checks() {
  const out = [];
  const add = (label, level, detail) => out.push({ label, level, detail });
  if (state.error) {
    add('Backend', 'bad', `can't reach the backend: ${state.error}`);
    return out;
  }
  const s = state.status || {};
  const desktop = s.desktop || {};
  if (!desktop.connected) {
    add('Desktop app', 'bad', 'not connected to the backend: the app is closed, starting, or its window is stuck');
    return out;
  }
  add('Desktop app', 'ok', `connected since ${clock(desktop.connectedSince)}`);

  const L = state.latest;
  if (!state.streaming || !L) {
    add('Live data', 'bad', L
      ? `the last sample is ${dur(state.latestAgeS * 1000)} old: the app's window may be frozen or busy`
      : "the desktop app sends none. An app started before the dashboard existed doesn't: restart it");
    return out;
  }
  add('Live data', 'ok', 'one sample a second');

  const link = L.tcode?.link;
  if (!link || link.state === 'stopped') {
    add('TCode link', 'bad', 'no TCode device: connect it in the app (Devices → TCode)');
  } else if (link.state === 'connected') {
    const up = link.connectedSince ? `, up ${dur(since(link.connectedSince))}` : '';
    add('TCode link', 'ok', `${link.url || link.transport}${link.ip ? ` (${link.ip})` : ''}${up}`);
  } else {
    const err = link.lastError?.text ? `: ${link.lastError.text}` : '';
    const next = link.nextRetryAt ? `, next try in ${dur(Date.parse(link.nextRetryAt) - state.now)}` : '';
    add('TCode link', 'bad', `${link.state}${err} (${link.attempts ?? 0} attempts${next})`);
  }

  const src = L.source;
  const vr = L.vr || {};
  if (src === 'heresphere' || src === 'deovr' || src === 'vr') {
    if (!vr.connected) add('Playback source', 'bad', `${src} bridge isn't connected${vr.lastError ? `: ${vr.lastError}` : ''}`);
    else if (vr.linkState === 'waiting') add('Playback source', 'warn', `${src} at ${vr.host}: no timestamp packets for ${dur(vr.lastPacketAgeMs)}`);
    else add('Playback source', 'ok', `${src} at ${vr.host}${vr.video ? `, ${vr.video}` : ''}`);
  } else if (src === 'phone') {
    add('Playback source', s.phone?.connected ? 'ok' : 'warn', s.phone?.connected ? `web remote on ${s.phone.ip}` : 'web remote, but no phone is connected');
  } else if (src === 'player-window') {
    add('Playback source', 'ok', 'pop-out player window');
  } else if (src === 'desktop') {
    if (s.player?.video) add('Playback source', 'ok', `desktop player: ${s.player.video}`);
    else {
      const headset = s.headset?.lastRequestAgeS != null && s.headset.lastRequestAgeS < 900;
      add('Playback source', 'warn', headset
        ? `the desktop player has no video, and the headset bridge isn't connected${vr.lastError ? ` (${vr.lastError})` : ''}: in HereSphere, check the timestamp server is on and the headset is awake`
        : 'the desktop player has no video and no headset or phone is driving the devices');
    }
  } else {
    add('Playback source', 'bad', 'the sync engine follows no player');
  }

  if (!L.script) add('Script', 'bad', 'no script loaded for this video');
  else {
    const axes = (L.sync?.axes || []).map((a) => a.axis).join(' ') || 'none';
    add('Script', 'ok', `${L.script.name}: ${L.script.actions} actions, ${mmss(L.script.durationMs)}; axes ${axes}`);
  }

  const sync = L.sync;
  if (!sync) {
    add('Sync engine', 'bad', 'no TCode sync engine');
    return out;
  }
  if (sync.mediaTimeMs == null) add('Playback', 'idle', 'no playback time');
  else if (sync.paused) add('Playback', 'warn', `paused at ${mmss(sync.mediaTimeMs)}`);
  else {
    const moving = mediaAdvancing();
    if (moving === false) add('Playback', 'bad', `says playing at ${mmss(sync.mediaTimeMs)}, but the time hasn't moved for 2 s: the source stopped sending time`);
    else add('Playback', 'ok', `playing at ${mmss(sync.mediaTimeMs)}`);
  }

  if (!sync.active) add('Sync engine', 'bad', 'stopped: it starts when a device is connected and a video with a script plays');
  else if (!sync.running) add('Sync engine', 'idle', 'started, waiting for playback');
  else add('Sync engine', 'ok', `running at ${sync.tickHz} Hz, offset ${sync.offsetMs} ms`);

  if (sync.nextKeyframeInMs == null) add('Script motion now', L.script ? 'warn' : 'idle', L.script ? 'past the end of the script' : '—');
  else if (sync.nextKeyframeInMs > sync.maxGapMs) add('Script motion now', 'warn', `next keyframe in ${dur(sync.nextKeyframeInMs)}: the device holds still through gaps over ${dur(sync.maxGapMs)}`);
  else add('Script motion now', 'ok', `next keyframe in ${dur(sync.nextKeyframeInMs)}`);

  const rate = recentRate();
  if (rate > 0) add('Commands', 'ok', `${rate.toFixed(1)} a second (last ${RATE_AVERAGE_MS / 1000} s)`);
  else add('Commands', sync.running ? 'bad' : 'idle', sync.running ? `none in the last ${RATE_AVERAGE_MS / 1000} s although the engine runs` : 'none (nothing to send)');

  const recent = state.points.filter((p) => p.t >= state.now - 10_000);
  const first = recent[0];
  const last = recent[recent.length - 1];
  const failed = first && last ? (last.failed ?? 0) - (first.failed ?? 0) : 0;
  const handed = first && last ? (last.handed ?? 0) - (first.handed ?? 0) : 0;
  if (failed > 0) add('Device accepts', 'bad', `${failed} of ${handed} sends failed in the last 10 s: the socket isn't open`);
  else if (link?.sent) add('Device accepts', link.sent.lastSendAgoMs == null ? 'idle' : 'ok', `${link.sent.commands} commands (${bytes(link.sent.bytes)}) this connection, last ${link.sent.lastSendAgoMs == null ? 'never' : `${dur(link.sent.lastSendAgoMs)} ago`}${link.sent.errors ? `, ${link.sent.errors} errors` : ''}`);
  else add('Device accepts', 'idle', '—');
  return out;
}

// --- rendering ---------------------------------------------------------------

function kv(el, rows) {
  el.innerHTML = rows.filter(Boolean).map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join('');
}

function renderChecks(list) {
  document.getElementById('checks').innerHTML = list.map((c) =>
    `<li><span class="dot ${c.level}"></span><span class="label">${esc(c.label)}</span><span class="detail">${esc(c.detail)}</span></li>`).join('');
  const verdict = document.getElementById('verdict');
  const problem = list.find((c) => c.level === 'bad') || list.find((c) => c.level === 'warn');
  verdict.className = `verdict ${problem ? 'bad' : 'ok'}`;
  verdict.textContent = problem ? `${problem.label}: ${problem.detail}` : 'Commands are flowing to the device.';
  const pill = document.getElementById('stream');
  pill.className = `pill ${state.streaming ? 'ok' : 'bad'}`;
  pill.textContent = state.error ? 'backend unreachable' : state.streaming ? 'live' : 'no live data';
}

function renderLink() {
  const L = state.latest;
  const link = L?.tcode?.link;
  const out = L?.tcode?.output;
  if (!link) { kv(document.getElementById('link'), [['State', 'no data']]); return; }
  kv(document.getElementById('link'), [
    ['State', link.state],
    ['Device', link.url || link.transport],
    link.ip && ['IP', link.ip],
    link.connectedSince && ['Connected', `${clock(link.connectedSince)} (${dur(since(link.connectedSince))})`],
    link.downSince && ['Down since', `${clock(link.downSince)} (${dur(since(link.downSince))})`],
    link.attempts ? ['Attempts', `${link.attempts}${link.retryEveryMs ? `, every ${dur(link.retryEveryMs)}` : ''}`] : null,
    link.lastError && ['Last error', `${link.lastError.text} (${clock(link.lastError.at)})`],
    link.lastDisconnect && ['Last drop', `${link.lastDisconnect.close} (${clock(link.lastDisconnect.at)})`],
    link.sent && ['Sent', `${link.sent.commands} commands, ${bytes(link.sent.bytes)}${link.sent.errors ? `, ${link.sent.errors} errors` : ''}`],
    link.sent && ['Last send', link.sent.lastSendAgoMs == null ? 'never' : `${dur(link.sent.lastSendAgoMs)} ago`],
    L.tcode.precision && ['Precision', `${L.tcode.precision} digits`],
    out && ['App run', `${out.commands} commands handed to the link, ${out.failed} failed`],
  ]);
}

function renderSync() {
  const L = state.latest;
  const sync = L?.sync;
  const table = document.getElementById('axes');
  if (!sync) { kv(document.getElementById('sync'), [['State', 'no data']]); table.innerHTML = ''; return; }
  kv(document.getElementById('sync'), [
    ['Engine', sync.active ? (sync.running ? 'running' : 'started, idle') : 'stopped'],
    ['Follows', L.source || 'nothing'],
    ['Media time', `${mmss(sync.mediaTimeMs)}${sync.paused ? ' (paused)' : ''}`],
    ['Next keyframe', sync.nextKeyframeInMs == null ? '—' : dur(sync.nextKeyframeInMs)],
    ['Update rate', `${sync.tickHz} Hz`],
    ['Offset', `${sync.offsetMs} ms`],
    ['Script', L.script ? `${L.script.name} (${L.script.actions} actions)` : 'none'],
  ]);
  table.innerHTML = '<tr><th>Axis</th><th>Source</th><th>Actions</th><th>Last sent</th><th></th></tr>' +
    sync.axes.map((a) => `<tr><td>${esc(a.axis)}</td><td>${esc(a.enabled ? a.kind : `${a.kind} (off)`)}</td>` +
      `<td>${a.actions ?? '—'}</td><td>${a.last == null ? '—' : a.last.toFixed(1)}</td>` +
      `<td><div class="bar"><span style="width:${a.last ?? 0}%;background:${AXIS_COLORS[a.axis] || '#5aa9ff'}"></span></div></td></tr>`).join('');
}

function renderSource() {
  const L = state.latest;
  const s = state.status || {};
  const vr = L?.vr || s.vrBridge || {};
  kv(document.getElementById('source'), [
    ['Follows', L?.source || '—'],
    ['VR bridge', vr.connected ? `${vr.linkState} (${vr.playerType} at ${vr.host})` : 'disconnected'],
    vr.connected && ['Headset video', vr.video || '—'],
    vr.lastPacketAgeMs != null && ['Last packet', `${dur(vr.lastPacketAgeMs)} ago`],
    vr.jitterMs != null && ['Jitter', `${Math.round(vr.jitterMs)} ms`],
    vr.lastError && ['Bridge error', vr.lastError],
    s.headset?.lastRequestAt && ['Headset request', `${dur(s.headset.lastRequestAgeS * 1000)} ago from ${s.headset.clientIp}${s.headset.video ? ` (${s.headset.video})` : ''}`],
    ['Phone', s.phone?.connected ? `${s.phone.ip}${s.phone.video ? `, ${s.phone.video}` : ''}` : 'not connected'],
    ['Desktop player', s.player?.video ? `${s.player.video}${s.player.playing ? ' (playing)' : ''}` : 'no video'],
  ]);
}

function renderCommands() {
  const el = document.getElementById('commands');
  const last = state.commands.slice(-15).reverse();
  el.innerHTML = last.length
    ? last.map((c) => `<li><span class="ago">-${dur(state.now - c.t)}</span><span>${esc(c.cmd)}</span></li>`).join('')
    : '<li class="empty">nothing sent in the last 30 s</li>';
}

function renderIssues() {
  const issues = state.status?.issues || [];
  document.getElementById('issues').innerHTML = issues.length
    ? issues.map((i) => `<li>${esc(i)}</li>`).join('')
    : '<li class="none">none</li>';
}

function renderEvents() {
  const el = document.getElementById('events');
  el.innerHTML = state.events.length
    ? state.events.slice().reverse().map((e) =>
      `<li class="${e.level}"><time>${clock(e.at)}</time><span class="kind">${esc(e.kind)}</span><span class="text">${esc(e.text)}</span></li>`).join('')
    : '<li class="empty">nothing yet</li>';
}

// --- charts ------------------------------------------------------------------

function prepare(canvas) {
  if (!canvas.dataset.h) canvas.dataset.h = canvas.getAttribute('height');
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = Number(canvas.dataset.h);
  canvas.style.height = `${h}px`;
  canvas.width = Math.round(w * dpr);
  canvas.height = Math.round(h * dpr);
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  return { ctx, w, h };
}

function drawChart(canvas, { xMin, xMax, yMax, series, bands = [], xLabels, yFormat }) {
  const { ctx, w, h } = prepare(canvas);
  const pad = { l: 44, r: 8, t: 6, b: 18 };
  const pw = w - pad.l - pad.r;
  const ph = h - pad.t - pad.b;
  const x = (v) => pad.l + ((v - xMin) / (xMax - xMin)) * pw;
  const y = (v) => pad.t + ph - (Math.min(v, yMax) / yMax) * ph;

  ctx.fillStyle = 'rgba(240, 82, 82, 0.18)';
  for (const [a, b] of bands) {
    const x0 = Math.max(pad.l, x(a));
    const x1 = Math.min(pad.l + pw, x(b));
    if (x1 > x0) ctx.fillRect(x0, pad.t, x1 - x0, ph);
  }

  ctx.strokeStyle = '#2a313b';
  ctx.fillStyle = '#8a94a3';
  ctx.font = '11px -apple-system, sans-serif';
  ctx.lineWidth = 1;
  for (let i = 0; i <= 2; i++) {
    const v = (yMax * i) / 2;
    ctx.beginPath(); ctx.moveTo(pad.l, y(v)); ctx.lineTo(pad.l + pw, y(v)); ctx.stroke();
    ctx.fillText(yFormat(v), 4, y(v) + 4);
  }
  for (const [v, label] of xLabels) {
    const px = x(v);
    ctx.fillText(label, Math.min(px - 10, w - 30), h - 4);
  }

  ctx.lineWidth = 1.5;
  for (const s of series) {
    ctx.strokeStyle = s.color;
    ctx.beginPath();
    let open = false;
    for (const [px, py] of s.pts) {
      if (py == null || px < xMin) { open = false; continue; }
      if (open) ctx.lineTo(x(px), y(py)); else ctx.moveTo(x(px), y(py));
      open = true;
    }
    ctx.stroke();
  }
}

function minuteLabels(xMax, span) {
  const out = [];
  for (let m = span / 60000; m >= 0; m -= 5) out.push([xMax - m * 60000, m ? `-${m}m` : 'now']);
  return out;
}

function renderCharts() {
  const now = state.now;
  const rates = rateSeries();
  const maxRate = Math.max(10, ...rates.map((r) => r[1] ?? 0));
  drawChart(document.getElementById('rate-chart'), {
    xMin: now - RATE_WINDOW_MS, xMax: now, yMax: Math.ceil(maxRate / 10) * 10,
    series: [{ color: '#5aa9ff', pts: rates }],
    bands: linkDownBands(),
    xLabels: minuteLabels(now, RATE_WINDOW_MS),
    yFormat: (v) => `${Math.round(v)}/s`,
  });

  const axes = axisSeries();
  document.getElementById('axis-legend').innerHTML = [...axes.keys()].sort()
    .map((a) => `<span><i style="background:${AXIS_COLORS[a] || '#ccc'}"></i>${esc(a)}</span>`).join('') || '<span>no commands</span>';
  drawChart(document.getElementById('axis-chart'), {
    xMin: now - AXIS_WINDOW_MS, xMax: now, yMax: 100,
    series: [...axes].map(([a, pts]) => ({ color: AXIS_COLORS[a] || '#ccc', pts })),
    xLabels: [[now - 30000, '-30s'], [now - 20000, '-20s'], [now - 10000, '-10s'], [now, 'now']],
    yFormat: (v) => `${Math.round(v)}`,
  });

  const vrPts = state.points.map((p) => [p.t, p.vrPacketAgeMs]);
  const maxAge = Math.max(1000, ...vrPts.map((p) => p[1] ?? 0));
  drawChart(document.getElementById('vr-chart'), {
    xMin: now - RATE_WINDOW_MS, xMax: now, yMax: Math.min(10000, Math.ceil(maxAge / 1000) * 1000),
    series: [{ color: '#3ccf7a', pts: vrPts }],
    xLabels: minuteLabels(now, RATE_WINDOW_MS),
    yFormat: (v) => `${(v / 1000).toFixed(1)}s`,
  });
}

function render() {
  renderChecks(checks());
  renderLink();
  renderSync();
  renderSource();
  renderCommands();
  renderIssues();
  renderEvents();
  renderCharts();
}

async function poll() {
  try {
    const q = new URLSearchParams({ since: state.seq, cmdSince: state.cmdSince, eventSince: state.eventSince });
    const res = await fetch(`/api/device/telemetry?${q}`, { cache: 'no-store' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    merge(await res.json());
    state.error = null;
  } catch (err) {
    state.error = err.message;
  }
  try { render(); } catch (err) { console.error(err); }
  setTimeout(poll, POLL_MS);
}

window.addEventListener('resize', () => renderCharts());
poll();
