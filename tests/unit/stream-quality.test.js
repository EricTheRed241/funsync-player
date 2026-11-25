// The web remote's Quality setting: a transcode starts at `t`, and the
// <video> keeps the whole video's clock across seeks (remote-sync and the
// seek bar read currentTime / duration as for the original file).
import { describe, it, expect } from 'vitest';
import {
  getQuality, setQuality, streamSrc, playTranscoded,
} from '../../crates/funsync-server/web/remote/stream-quality.js';

function memoryStorage() {
  const m = new Map();
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => m.set(k, String(v)), removeItem: (k) => m.delete(k) };
}

describe('stream quality', () => {
  it('remembers a known quality and falls back to the original', () => {
    const s = memoryStorage();
    expect(getQuality(s)).toBe('original');
    setQuality('720', s);
    expect(getQuality(s)).toBe('720');
    setQuality('4k', s);
    expect(getQuality(s)).toBe('720');
    setQuality('original', s);
    expect(getQuality(s)).toBe('original');
  });

  it('asks the server for a transcode from a time, the original as is', () => {
    expect(streamSrc('/api/media/stream/ab', 'original', 30)).toBe('/api/media/stream/ab');
    expect(streamSrc('/api/media/stream/ab', '1080', 75.5)).toBe('/api/media/stream/ab?quality=1080&t=75.500');
    expect(streamSrc('/api/media/stream/ab?k=1', '720', -4)).toBe('/api/media/stream/ab?k=1&quality=720&t=0.000');
  });

  it('keeps the original clock through a seek and fires seeking / seeked', () => {
    const v = document.createElement('video');
    playTranscoded(v, { base: '/api/media/stream/ab', quality: '1080', duration: 600, start: 12 });
    expect(v.src).toContain('quality=1080&t=12.000');
    expect(v.currentTime).toBe(12);
    expect(v.duration).toBe(600);

    const events = [];
    v.addEventListener('seeking', () => events.push(['seeking', v.currentTime]));
    v.addEventListener('seeked', () => events.push(['seeked', v.currentTime]));
    v.currentTime = 300;
    expect(v.src).toContain('t=300.000');
    expect(v.currentTime).toBe(300);
    v.dispatchEvent(new Event('loadeddata'));
    expect(events).toEqual([['seeking', 300], ['seeked', 300]]);

    v.currentTime = 9999;
    expect(v.currentTime).toBe(600);
  });
});
