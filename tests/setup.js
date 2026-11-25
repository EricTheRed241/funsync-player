// Global test setup: browser globals some shared modules expect.
//
// This file runs for EVERY test file, including the ones that opt into the
// `node` environment (see the `@vitest-environment` docblock convention), so
// it must not assume a DOM exists at module scope. `globalThis` is used
// instead of `window`: under jsdom they are the same object. Genuinely
// DOM-only patches (HTMLCanvasElement) are explicitly guarded.
import { vi } from 'vitest';

// --- localStorage on Node 25+ ---
// Node 25+ defines its own global `localStorage`, which is undefined unless
// the process starts with --localstorage-file. Vitest does not replace an
// existing global with jsdom's, so jsdom files saw `localStorage` as
// undefined (CI on Node 22 never did). Point it back at jsdom's storage.
if (globalThis.jsdom && globalThis.localStorage === undefined) {
  Object.defineProperty(globalThis, 'localStorage', {
    value: globalThis.jsdom.window.localStorage,
    configurable: true,
    writable: true,
  });
}

// --- Canvas 2D context mock ---
const mockCtx = {
  save: vi.fn(),
  restore: vi.fn(),
  setTransform: vi.fn(),
  scale: vi.fn(),
  clearRect: vi.fn(),
  fillRect: vi.fn(),
  strokeRect: vi.fn(),
  beginPath: vi.fn(),
  closePath: vi.fn(),
  moveTo: vi.fn(),
  lineTo: vi.fn(),
  arc: vi.fn(),
  fill: vi.fn(),
  stroke: vi.fn(),
  drawImage: vi.fn(),
  measureText: vi.fn().mockReturnValue({ width: 50 }),
  roundRect: vi.fn(),
  fillText: vi.fn(),
  fillStyle: '',
  strokeStyle: '',
  lineWidth: 1,
  font: '',
  textAlign: '',
  textBaseline: '',
  globalAlpha: 1,
  canvas: { width: 800, height: 400 },
};

// DOM-only: absent in the `node` environment, where nothing renders a canvas.
if (typeof HTMLCanvasElement !== 'undefined') {
  const _origGetContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (type) {
    if (type === '2d') return mockCtx;
    return _origGetContext.call(this, type);
  };
}

// --- requestAnimationFrame / cancelAnimationFrame ---
let _rafId = 0;
if (!globalThis.requestAnimationFrame) {
  globalThis.requestAnimationFrame = (cb) => ++_rafId;
}
if (!globalThis.cancelAnimationFrame) {
  globalThis.cancelAnimationFrame = () => {};
}

// --- ResizeObserver ---
if (!globalThis.ResizeObserver) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// --- URL.createObjectURL / revokeObjectURL ---
if (!URL.createObjectURL) {
  URL.createObjectURL = vi.fn().mockReturnValue('blob:mock-url');
}
if (!URL.revokeObjectURL) {
  URL.revokeObjectURL = vi.fn();
}

// --- Path2D ---
if (!globalThis.Path2D) {
  globalThis.Path2D = class {
    constructor() {}
    moveTo() {}
    lineTo() {}
    closePath() {}
    arc() {}
    rect() {}
  };
}

// --- devicePixelRatio ---
if (!globalThis.devicePixelRatio) {
  globalThis.devicePixelRatio = 1;
}

// --- crypto.randomUUID ---
if (!crypto.randomUUID) {
  crypto.randomUUID = () => {
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
      const r = (Math.random() * 16) | 0;
      return (c === 'x' ? r : (r & 0x3) | 0x8).toString(16);
    });
  };
}
