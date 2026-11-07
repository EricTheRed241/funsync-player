// Regression test: UTF-8 BOM on funscript files must not break parsing.
// Community report 2026-06-01 — RyzaMerged.funscript loaded fine in
// another player but errored here at JSON.parse because the file carried
// a leading EF BB BF BOM (U+FEFF). Fixed at the IPC boundary AND in
// FunscriptEngine.loadContent for drag-drop coverage.

import { describe, it, expect } from 'vitest';
import { FunscriptEngine, stripBOM } from '../../renderer/js/funscript-engine.js';

const SAMPLE = JSON.stringify({
  actions: [
    { at: 0, pos: 0 },
    { at: 500, pos: 100 },
    { at: 1000, pos: 50 },
  ],
});

describe('stripBOM', () => {
  it('removes a leading U+FEFF BOM character', () => {
    const withBom = '﻿' + SAMPLE;
    expect(stripBOM(withBom)).toBe(SAMPLE);
  });

  it('passes through content without a BOM unchanged', () => {
    expect(stripBOM(SAMPLE)).toBe(SAMPLE);
  });

  it('does not strip a BOM that appears mid-string', () => {
    const midBom = SAMPLE + '﻿';
    expect(stripBOM(midBom)).toBe(midBom);
  });

  it('handles empty string', () => {
    expect(stripBOM('')).toBe('');
  });

  it('handles non-string input defensively', () => {
    expect(stripBOM(null)).toBe(null);
    expect(stripBOM(undefined)).toBe(undefined);
    expect(stripBOM(42)).toBe(42);
  });
});

describe('FunscriptEngine.loadContent — BOM tolerance', () => {
  it('parses content with a leading UTF-8 BOM', async () => {
    const engine = new FunscriptEngine();
    const withBom = '﻿' + SAMPLE;
    await expect(engine.loadContent(withBom, 'bommed.funscript')).resolves.toBeTruthy();
    const actions = engine.getActions();
    expect(actions.length).toBe(3);
    expect(actions[0]).toEqual({ at: 0, pos: 0 });
  });

  it('still parses content without a BOM', async () => {
    const engine = new FunscriptEngine();
    await expect(engine.loadContent(SAMPLE, 'clean.funscript')).resolves.toBeTruthy();
    expect(engine.getActions().length).toBe(3);
  });

  it('preserves chapters / bookmarks through BOM-prefixed parse', async () => {
    const engine = new FunscriptEngine();
    const withMetadata = JSON.stringify({
      actions: [{ at: 0, pos: 0 }, { at: 100, pos: 50 }],
      metadata: {
        bookmarks: [{ at: 50, name: 'mid' }],
        chapters: [{ name: 'S1', startTime: 0, endTime: 100 }],
      },
    });
    await engine.loadContent('﻿' + withMetadata, 'meta.funscript');
    expect(engine.getBookmarks()).toEqual([{ at: 50, name: 'mid' }]);
    expect(engine.getChapters().length).toBe(1);
  });
});
