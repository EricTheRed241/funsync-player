// ActionGraph — read-only canvas renderer for a windowed funscript timeline
// (time × position). OFS (OpenFunscripter) style: centered playhead,
// speed-colored lines, dark purple bg. Mounted by the inline TL overlay.

export class ActionGraph {
  /**
   * @param {HTMLCanvasElement} canvas
   * @param {{actions: Array<{at:number,pos:number}>}} source — read on every draw
   */
  constructor(canvas, source) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d');
    this.script = source;

    // Viewport (visible time window)
    this._viewStartMs = 0;
    this._viewEndMs = 5000; // OFS default: 5s view
    this._videoDurationMs = 0;

    // Playback cursor
    this._cursorMs = 0;

    // Drawing constants
    this._padding = { top: 10, right: 20, bottom: 28, left: 40 };
    this._dotRadius = 5;

    // OFS Colors
    this._colors = {
      bg: 'rgb(60, 0, 60)',                    // OFS dark purple
      grid: 'rgba(255, 255, 255, 0.08)',
      gridLabel: 'rgba(255, 255, 255, 0.35)',
      dot: '#e94560',                           // red with dark border
      dotBorder: 'rgba(0, 0, 0, 0.8)',
      cursor: '#4caf50',
    };
  }

  // --- Viewport ---

  get viewStartMs() { return this._viewStartMs; }
  get viewEndMs() { return this._viewEndMs; }
  get viewDurationMs() { return this._viewEndMs - this._viewStartMs; }

  setVideoDuration(durationMs) {
    this._videoDurationMs = durationMs;
    if (this._viewEndMs > durationMs) {
      this._viewEndMs = durationMs;
    }
  }

  setViewport(startMs, endMs) {
    const minDuration = 1000;  // OFS: 1s minimum
    const maxDuration = 300000; // OFS: 300s maximum
    let duration = Math.max(minDuration, Math.min(maxDuration, endMs - startMs));
    this._viewStartMs = Math.max(0, startMs);
    this._viewEndMs = this._viewStartMs + duration;
    if (this._videoDurationMs > 0 && this._viewEndMs > this._videoDurationMs) {
      this._viewEndMs = this._videoDurationMs;
      this._viewStartMs = Math.max(0, this._viewEndMs - duration);
    }
  }

  centerOnTime(ms) {
    const half = this.viewDurationMs / 2;
    this.setViewport(ms - half, ms + half);
  }

  // --- Coordinate mapping ---

  /** Canvas drawing area (excludes padding). Cached — invalidated on resize. */
  _drawArea() {
    if (this._cachedArea) return this._cachedArea;
    const dpr = window.devicePixelRatio || 1;
    const w = this.canvas.width / dpr;
    const h = this.canvas.height / dpr;
    this._cachedArea = {
      x: this._padding.left,
      y: this._padding.top,
      w: w - this._padding.left - this._padding.right,
      h: h - this._padding.top - this._padding.bottom,
    };
    return this._cachedArea;
  }

  timeToX(ms) {
    const area = this._drawArea();
    return area.x + ((ms - this._viewStartMs) / this.viewDurationMs) * area.w;
  }

  xToTime(px) {
    const area = this._drawArea();
    return this._viewStartMs + ((px - area.x) / area.w) * this.viewDurationMs;
  }

  posToY(pos) {
    const area = this._drawArea();
    // Inverted: 100 at top, 0 at bottom
    return area.y + (1 - pos / 100) * area.h;
  }

  // --- Binary search utilities ---

  /**
   * Binary search for the leftmost action index where action.at >= targetMs.
   * Returns actions.length if all actions are before targetMs.
   */
  _lowerBound(actions, targetMs) {
    let lo = 0, hi = actions.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (actions[mid].at < targetMs) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  /**
   * Get the visible action index range for the current viewport.
   * Includes 1 extra action on each side for connection line continuity.
   * @returns {{ first: number, last: number }}
   */
  _visibleRange(actions) {
    if (actions.length === 0) return { first: 0, last: -1 };
    const first = Math.max(0, this._lowerBound(actions, this._viewStartMs) - 1);
    const lastRaw = this._lowerBound(actions, this._viewEndMs);
    const last = Math.min(actions.length - 1, lastRaw + 1);
    return { first, last };
  }

  // --- Cursor ---

  setCursorTime(ms) {
    this._cursorMs = ms;
  }

  // --- Resize ---

  resize() {
    const rect = this.canvas.parentElement.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    this.canvas.width = rect.width * dpr;
    this.canvas.height = rect.height * dpr;
    this.canvas.style.width = rect.width + 'px';
    this.canvas.style.height = rect.height + 'px';
    this._cachedArea = null; // invalidate draw area cache
    this.draw();
  }

  // --- Speed to color (OFS heatmap gradient) ---

  /**
   * Map action speed to OFS 6-stop heatmap color.
   * Speed is |deltaPos| / deltaTime (pos-units per millisecond).
   * @param {number} speed — pos/ms
   * @returns {string} CSS color
   */
  static speedToColor(speed) {
    // Use pre-computed LUT for O(1) lookup
    if (!ActionGraph._speedColorLUT) {
      ActionGraph._buildSpeedColorLUT();
    }
    const idx = Math.min(255, Math.floor(Math.min(1, speed / 0.3) * 255));
    return ActionGraph._speedColorLUT[idx];
  }

  static _buildSpeedColorLUT() {
    const stops = [
      [0, 0, 0], [0, 0, 255], [0, 255, 255],
      [0, 255, 0], [255, 255, 0], [255, 0, 0],
    ];
    const lut = new Array(256);
    for (let i = 0; i < 256; i++) {
      const t = i / 255;
      const idx = t * (stops.length - 1);
      const lo = Math.floor(idx);
      const hi = Math.min(lo + 1, stops.length - 1);
      const frac = idx - lo;
      const r = Math.round(stops[lo][0] + (stops[hi][0] - stops[lo][0]) * frac);
      const g = Math.round(stops[lo][1] + (stops[hi][1] - stops[lo][1]) * frac);
      const b = Math.round(stops[lo][2] + (stops[hi][2] - stops[lo][2]) * frac);
      lut[i] = `rgb(${r},${g},${b})`;
    }
    ActionGraph._speedColorLUT = lut;
  }

  // --- Drawing ---

  draw() {
    const dpr = window.devicePixelRatio || 1;
    const ctx = this.ctx;

    // Defensive auto-resize: if the canvas has zero pixel dimensions but
    // its parent is sized (e.g. the overlay was just un-hidden and resize()
    // hasn't fired yet), pull the parent's current rect and resync. Without
    // this, every position computed through posToY / timeToX lands at a
    // non-positive area, so all dots / lines / cursor render outside the
    // visible canvas.
    if (this.canvas.width === 0 || this.canvas.height === 0) {
      const parent = this.canvas.parentElement;
      const rect = parent ? parent.getBoundingClientRect() : null;
      if (rect && rect.width > 0 && rect.height > 0) {
        this.canvas.width = rect.width * dpr;
        this.canvas.height = rect.height * dpr;
        this.canvas.style.width = rect.width + 'px';
        this.canvas.style.height = rect.height + 'px';
        this._cachedArea = null;
      }
    }

    const w = this.canvas.width / dpr;
    const h = this.canvas.height / dpr;

    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    // 1. Background
    ctx.fillStyle = this._colors.bg;
    ctx.fillRect(0, 0, w, h);

    const area = this._drawArea();

    // 2. Grid lines
    this._drawGrid(ctx, area);

    // 3. Connection lines (speed-colored, OFS style)
    this._drawConnections(ctx);

    // 4. Action dots
    this._drawDots(ctx);

    // 5. Playback cursor (centered during playback)
    this._drawCursor(ctx, area);

    // 6. Heatmap bar at bottom
    this._drawHeatmapBar(ctx, area);

    ctx.restore();
  }

  _drawGrid(ctx, area) {
    ctx.strokeStyle = this._colors.grid;
    ctx.lineWidth = 1;
    ctx.font = '10px sans-serif';
    ctx.fillStyle = this._colors.gridLabel;

    // Horizontal grid (position: 0, 25, 50, 75, 100)
    for (const pos of [0, 25, 50, 75, 100]) {
      const y = this.posToY(pos);
      ctx.beginPath();
      ctx.moveTo(area.x, y);
      ctx.lineTo(area.x + area.w, y);
      ctx.stroke();
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(String(pos), area.x - 6, y);
    }

    // Vertical grid (time — adaptive intervals)
    const duration = this.viewDurationMs;
    let interval;
    if (duration > 60000) interval = 30000;
    else if (duration > 10000) interval = 5000;
    else if (duration > 2000) interval = 1000;
    else interval = 100;

    const firstTick = Math.ceil(this._viewStartMs / interval) * interval;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';

    for (let t = firstTick; t <= this._viewEndMs; t += interval) {
      const x = this.timeToX(t);
      ctx.beginPath();
      ctx.moveTo(x, area.y);
      ctx.lineTo(x, area.y + area.h);
      ctx.stroke();
      ctx.fillText(this._formatTime(t), x, area.y + area.h + 6);
    }
  }

  _drawConnections(ctx) {
    const actions = this.script.actions;
    if (actions.length < 2) return;

    // Viewport culling — only draw visible slice + 1 on each side
    const { first, last } = this._visibleRange(actions);
    if (first >= last) return;

    const sliceLen = last - first + 1;
    const xs = new Array(sliceLen);
    const ys = new Array(sliceLen);
    for (let i = first; i <= last; i++) {
      xs[i - first] = this.timeToX(actions[i].at);
      ys[i - first] = this.posToY(actions[i].pos);
    }

    const tangents = this._monotoneTangents(xs, ys);

    // Draw each visible segment
    for (let si = 0; si < sliceLen - 1; si++) {
      const i = first + si;
      const deltaTime = actions[i + 1].at - actions[i].at;
      const deltaPos = Math.abs(actions[i + 1].pos - actions[i].pos);
      const speed = deltaTime > 0 ? deltaPos / deltaTime : 0;
      const color = ActionGraph.speedToColor(speed);

      const x1 = xs[si], y1 = ys[si], x2 = xs[si + 1], y2 = ys[si + 1];

      // Cubic Hermite spline
      const path = new Path2D();
      path.moveTo(x1, y1);
      const dx = x2 - x1;
      const m1 = tangents[si] * dx;
      const m2 = tangents[si + 1] * dx;
      const steps = 16;
      for (let s = 1; s <= steps; s++) {
        const t = s / steps;
        const t2 = t * t;
        const t3 = t2 * t;
        const h00 = 2 * t3 - 3 * t2 + 1;
        const h10 = t3 - 2 * t2 + t;
        const h01 = -2 * t3 + 3 * t2;
        const h11 = t3 - t2;
        path.lineTo(x1 + dx * t, h00 * y1 + h10 * m1 + h01 * y2 + h11 * m2);
      }

      ctx.strokeStyle = 'rgba(0, 0, 0, 0.6)';
      ctx.lineWidth = 5;
      ctx.stroke(path);

      ctx.strokeStyle = color;
      ctx.lineWidth = 3;
      ctx.stroke(path);
    }
  }

  /**
   * Compute monotone cubic Hermite tangents using the Fritsch-Carlson method.
   * Guarantees the interpolation never overshoots between data points.
   * @returns {Float64Array} tangent (dy/dx) at each point
   */
  _monotoneTangents(xs, ys) {
    const n = xs.length;
    const tangents = new Float64Array(n);

    if (n < 2) return tangents;
    if (n === 2) {
      const slope = (ys[1] - ys[0]) / (xs[1] - xs[0] || 1);
      tangents[0] = slope;
      tangents[1] = slope;
      return tangents;
    }

    // Step 1: compute slopes between successive points
    const deltas = new Float64Array(n - 1);
    const slopes = new Float64Array(n - 1);
    for (let i = 0; i < n - 1; i++) {
      deltas[i] = xs[i + 1] - xs[i];
      slopes[i] = deltas[i] !== 0 ? (ys[i + 1] - ys[i]) / deltas[i] : 0;
    }

    // Step 2: initial tangents as average of adjacent slopes
    tangents[0] = slopes[0];
    for (let i = 1; i < n - 1; i++) {
      tangents[i] = (slopes[i - 1] + slopes[i]) / 2;
    }
    tangents[n - 1] = slopes[n - 2];

    // Step 3: Fritsch-Carlson monotonicity constraints
    for (let i = 0; i < n - 1; i++) {
      if (Math.abs(slopes[i]) < 1e-10) {
        // Flat segment — zero tangents at both ends
        tangents[i] = 0;
        tangents[i + 1] = 0;
      } else {
        const alpha = tangents[i] / slopes[i];
        const beta = tangents[i + 1] / slopes[i];

        // Clamp to ensure monotonicity (circle constraint)
        const mag = Math.sqrt(alpha * alpha + beta * beta);
        if (mag > 3) {
          const tau = 3 / mag;
          tangents[i] = tau * alpha * slopes[i];
          tangents[i + 1] = tau * beta * slopes[i];
        }
      }
    }

    return tangents;
  }

  _drawDots(ctx) {
    const actions = this.script.actions;

    // Viewport culling — only draw visible dots
    const { first, last } = this._visibleRange(actions);

    ctx.strokeStyle = this._colors.dotBorder;
    ctx.lineWidth = 1.5;
    ctx.fillStyle = this._colors.dot;
    for (let i = first; i <= last; i++) {
      const x = this.timeToX(actions[i].at);
      const y = this.posToY(actions[i].pos);
      ctx.beginPath();
      ctx.arc(x, y, this._dotRadius, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
    }
  }

  _drawCursor(ctx, area) {
    const x = this.timeToX(this._cursorMs);
    if (x < area.x - 1 || x > area.x + area.w + 1) return;

    ctx.strokeStyle = this._colors.cursor;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(x, area.y);
    ctx.lineTo(x, area.y + area.h);
    ctx.stroke();

    // Small triangle at top
    ctx.fillStyle = this._colors.cursor;
    ctx.beginPath();
    ctx.moveTo(x, area.y);
    ctx.lineTo(x - 5, area.y - 6);
    ctx.lineTo(x + 5, area.y - 6);
    ctx.closePath();
    ctx.fill();
  }

  /** Draw a thin heatmap intensity bar at the very bottom of the draw area. */
  _drawHeatmapBar(ctx, area) {
    const actions = this.script.actions;
    if (actions.length < 2) return;

    const barHeight = 6;
    const barY = area.y + area.h - barHeight;
    const barWidth = area.w;

    // Draw in pixel-width segments
    const segments = Math.min(barWidth, 200);
    const segWidth = barWidth / segments;

    // Binary search for starting action index, then scan forward
    const startTime = this.xToTime(area.x);
    let cursor = Math.max(0, this._lowerBound(actions, startTime) - 1);

    ctx.globalAlpha = 0.6;
    for (let s = 0; s < segments; s++) {
      const x = area.x + s * segWidth;
      const timeAtX = this.xToTime(x);

      // Advance cursor to the enclosing interval
      while (cursor < actions.length - 2 && actions[cursor + 1].at < timeAtX) {
        cursor++;
      }

      let speed = 0;
      if (cursor < actions.length - 1 && actions[cursor].at <= timeAtX && actions[cursor + 1].at >= timeAtX) {
        const dt = actions[cursor + 1].at - actions[cursor].at;
        const dp = Math.abs(actions[cursor + 1].pos - actions[cursor].pos);
        speed = dt > 0 ? dp / dt : 0;
      }

      ctx.fillStyle = ActionGraph.speedToColor(speed);
      ctx.fillRect(x, barY, segWidth + 0.5, barHeight);
    }
    ctx.globalAlpha = 1;
  }

  // --- Helpers ---

  _formatTime(ms) {
    const totalSec = Math.floor(ms / 1000);
    const min = Math.floor(totalSec / 60);
    const sec = totalSec % 60;
    const frac = ms % 1000;

    if (this.viewDurationMs <= 5000) {
      // Show milliseconds for tight zoom
      return `${min}:${String(sec).padStart(2, '0')}.${String(Math.floor(frac / 100))}`;
    }
    return `${min}:${String(sec).padStart(2, '0')}`;
  }
}
