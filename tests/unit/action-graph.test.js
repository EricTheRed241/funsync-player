// Unit tests for ActionGraph — imports from real source
import { describe, it, expect, beforeEach } from 'vitest';
import { ActionGraph } from '../../renderer/js/action-graph.js';

describe('ActionGraph', () => {
  let graph, canvas;

  beforeEach(() => {
    // Create a canvas and set dimensions (jsdom canvas)
    canvas = document.createElement('canvas');
    canvas.width = 800;
    canvas.height = 400;
    // Mock parentElement for resize operations
    const parent = document.createElement('div');
    parent.appendChild(canvas);
    parent.getBoundingClientRect = () => ({ width: 800, height: 400 });
    document.body.appendChild(parent);

    graph = new ActionGraph(canvas, { actions: [] });
  });

  // --- Viewport ---

  describe('setViewport', () => {
    it('sets start and end times', () => {
      graph.setViewport(1000, 5000);
      expect(graph.viewStartMs).toBe(1000);
      expect(graph.viewEndMs).toBe(5000);
      expect(graph.viewDurationMs).toBe(4000);
    });

    it('enforces minimum 1s duration', () => {
      graph.setViewport(1000, 1500);
      expect(graph.viewDurationMs).toBeGreaterThanOrEqual(1000);
    });

    it('enforces maximum 300s duration', () => {
      graph.setViewport(0, 500000);
      expect(graph.viewDurationMs).toBeLessThanOrEqual(300000);
    });

    it('clamps start to >= 0', () => {
      graph.setViewport(-1000, 3000);
      expect(graph.viewStartMs).toBe(0);
    });

    it('respects video duration', () => {
      graph.setVideoDuration(10000);
      graph.setViewport(8000, 20000);
      expect(graph.viewEndMs).toBeLessThanOrEqual(10000);
    });
  });

  describe('centerOnTime', () => {
    it('centers viewport on given time', () => {
      graph.setViewport(0, 4000);
      graph.centerOnTime(5000);
      // Should center the 4000ms window around 5000ms
      expect(graph.viewStartMs).toBe(3000);
      expect(graph.viewEndMs).toBe(7000);
    });
  });

  // --- Coordinate mapping ---

  describe('timeToX / xToTime round-trip', () => {
    it('round-trips correctly', () => {
      graph.setViewport(0, 10000);
      const time = 5000;
      const x = graph.timeToX(time);
      const recoveredTime = graph.xToTime(x);
      expect(Math.abs(recoveredTime - time)).toBeLessThan(1);
    });

    it('maps viewport start to left edge', () => {
      graph.setViewport(1000, 5000);
      const x = graph.timeToX(1000);
      expect(x).toBe(graph._padding.left);
    });
  });

  describe('posToY', () => {
    it('pos 100 maps to top (y close to padding.top)', () => {
      const y100 = graph.posToY(100);
      const y0 = graph.posToY(0);
      expect(y100).toBeLessThan(y0);
    });
  });

  // --- Static speedToColor ---

  describe('ActionGraph.speedToColor', () => {
    it('returns black for zero speed', () => {
      expect(ActionGraph.speedToColor(0)).toBe('rgb(0,0,0)');
    });

    it('returns red for high speed', () => {
      expect(ActionGraph.speedToColor(0.3)).toBe('rgb(255,0,0)');
    });

    it('clamps above max speed', () => {
      expect(ActionGraph.speedToColor(1.0)).toBe('rgb(255,0,0)');
    });

    it('returns blue-ish for low speed', () => {
      const color = ActionGraph.speedToColor(0.06);
      expect(color).toMatch(/^rgb\(/);
      // At normalized=0.2, should be pure blue
      expect(ActionGraph.speedToColor(0.06)).toBe('rgb(0,0,255)');
    });

    it('returns green for medium speed', () => {
      // normalized = 0.6 → green (idx 3)
      const speed = 0.3 * 0.6;
      const color = ActionGraph.speedToColor(speed);
      expect(color).toBe('rgb(0,255,0)');
    });
  });

  // --- _monotoneTangents ---

  describe('_monotoneTangents', () => {
    it('returns constant slope for 2 points', () => {
      const xs = [0, 100];
      const ys = [0, 100];
      const tangents = graph._monotoneTangents(xs, ys);
      expect(tangents[0]).toBeCloseTo(1);
      expect(tangents[1]).toBeCloseTo(1);
    });

    it('returns zero tangents for flat segment', () => {
      const xs = [0, 100, 200];
      const ys = [50, 50, 50];
      const tangents = graph._monotoneTangents(xs, ys);
      expect(tangents[0]).toBe(0);
      expect(tangents[1]).toBe(0);
      expect(tangents[2]).toBe(0);
    });

    it('returns empty for empty input', () => {
      const tangents = graph._monotoneTangents([], []);
      expect(tangents.length).toBe(0);
    });

    it('produces valid tangents for monotone data', () => {
      const xs = [0, 100, 200, 300];
      const ys = [0, 30, 60, 100];
      const tangents = graph._monotoneTangents(xs, ys);
      expect(tangents.length).toBe(4);
      // All tangents should be positive for monotonically increasing data
      for (const t of tangents) {
        expect(t).toBeGreaterThanOrEqual(0);
      }
    });
  });
});
