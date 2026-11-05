/**
 * @vitest-environment jsdom
 * Web remote shared play queue (backend/web-remote/queue.js): what plays
 * next, the request bodies and endpoints, and the per-phone autoplay
 * preference.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  nextUp, normalizeQueue, fetchQueue, queueAddBody, queueAdd, queueRemove, queuePlayed, queueClear,
  queueSetAutoFill, queueFromResult, getAutoplay, setAutoplay, AUTOPLAY_STORAGE_KEY,
} from '../../backend/web-remote/queue.js';

const shared = (manual, auto = [], autoFill = true) => ({
  items: [...manual.map((videoId) => ({ videoId, auto: false })), ...auto.map((videoId) => ({ videoId, auto: true }))],
  autoFill,
});

function memoryStorage() {
  const map = new Map();
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => { map.set(k, String(v)); },
  };
}

describe('nextUp precedence', () => {
  const playAll = { ids: ['p1', 'p2', 'p3'], index: 0 };

  it('manual queue items come first, before Play All and auto-filled', () => {
    expect(nextUp({ currentId: 'p1', shared: shared(['m1', 'm2'], ['a1']), phoneQueue: playAll }))
      .toEqual({ id: 'm1', source: 'manual' });
  });

  it('then the next item of the Play All list', () => {
    expect(nextUp({ currentId: 'p1', shared: shared([], ['a1']), phoneQueue: playAll }))
      .toEqual({ id: 'p2', source: 'playAll' });
  });

  it('then auto-filled items, also after Play All has run out', () => {
    expect(nextUp({ currentId: 'x', shared: shared([], ['a1', 'a2']) })).toEqual({ id: 'a1', source: 'auto' });
    expect(nextUp({ currentId: 'p3', shared: shared([], ['a1']), phoneQueue: { ids: ['p1', 'p2', 'p3'], index: 2 } }))
      .toEqual({ id: 'a1', source: 'auto' });
  });

  it('Play All continues after a manual item played in between', () => {
    // Next took the phone from p1 to manual item m1; Play All still sits at p1.
    expect(nextUp({ currentId: 'm1', shared: shared([]), phoneQueue: playAll })).toEqual({ id: 'p2', source: 'playAll' });
  });
});

describe('nextUp skips the video being played', () => {
  it('in the manual queue', () => {
    expect(nextUp({ currentId: 'm1', shared: shared(['m1', 'm2']) })).toEqual({ id: 'm2', source: 'manual' });
  });

  it('in the Play All list', () => {
    expect(nextUp({ currentId: 'p2', shared: shared([]), phoneQueue: { ids: ['p1', 'p2', 'p2', 'p3'], index: 1 } }))
      .toEqual({ id: 'p3', source: 'playAll' });
  });

  it('in the auto-filled items', () => {
    expect(nextUp({ currentId: 'a1', shared: shared([], ['a1', 'a2']) })).toEqual({ id: 'a2', source: 'auto' });
  });

  it('falls through when the current video is the only manual item', () => {
    expect(nextUp({ currentId: 'm1', shared: shared(['m1'], ['a1']) })).toEqual({ id: 'a1', source: 'auto' });
  });
});

describe('nextUp with nothing up next', () => {
  it('returns null for no queue, an empty queue, or only the current video', () => {
    expect(nextUp({ currentId: 'v' })).toBeNull();
    expect(nextUp({ currentId: 'v', shared: shared([]) })).toBeNull();
    expect(nextUp({ currentId: 'v', shared: shared(['v'], ['v']), phoneQueue: { ids: ['v'], index: 0 } })).toBeNull();
  });

  it('auto-fill off leaves manual and Play All', () => {
    expect(nextUp({ currentId: 'v', shared: shared([], [], false), phoneQueue: { ids: ['v', 'p2'], index: 0 } }))
      .toEqual({ id: 'p2', source: 'playAll' });
  });
});

describe('queue state from the backend', () => {
  it('normalises the GET body and drops malformed items', () => {
    expect(normalizeQueue({ items: [{ videoId: 'a' }, { videoId: 'b', auto: 1 }, { auto: true }, null], autoFill: false }))
      .toEqual({ items: [{ videoId: 'a', auto: false }, { videoId: 'b', auto: true }], autoFill: false });
    expect(normalizeQueue({})).toEqual({ items: [], autoFill: true });
  });

  it('reads GET /api/remote/queue and gives null when it fails', async () => {
    const ok = vi.fn(async () => ({ ok: true, json: async () => shared(['m1'], ['a1']) }));
    expect(await fetchQueue(ok)).toEqual(shared(['m1'], ['a1']));
    expect(ok).toHaveBeenCalledWith('/api/remote/queue');
    expect(await fetchQueue(async () => ({ ok: false, status: 404 }))).toBeNull();
    expect(await fetchQueue(async () => { throw new Error('offline'); })).toBeNull();
  });

  it('takes the queue a write answered with', () => {
    expect(queueFromResult({ ok: true, data: { ok: true, delivered: true, queue: shared(['m1']) } })).toEqual(shared(['m1']));
    expect(queueFromResult({ ok: false, status: 404, data: { detail: 'unknown video' } })).toBeNull();
    expect(queueFromResult({ ok: true, data: { ok: true } })).toBeNull();
  });
});

describe('request bodies + endpoints', () => {
  it('add defaults to the end; only end / next are sent', () => {
    expect(queueAddBody('v1')).toEqual({ videoId: 'v1', position: 'end' });
    expect(queueAddBody('v1', 'next')).toEqual({ videoId: 'v1', position: 'next' });
    expect(queueAddBody('v1', 'front')).toEqual({ videoId: 'v1', position: 'end' });
  });

  it('posts to the right endpoints', async () => {
    const fetchImpl = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ ok: true, delivered: true }) }));
    await queueAdd('v1', undefined, fetchImpl);
    await queueAdd('v2', 'next', fetchImpl);
    await queueRemove('v1', fetchImpl);
    await queuePlayed('v2', fetchImpl);
    await queueClear(fetchImpl);
    await queueSetAutoFill(0, fetchImpl);
    expect(fetchImpl.mock.calls.map(([url, init]) => [url, init.method, JSON.parse(init.body)])).toEqual([
      ['/api/remote/queue/add', 'POST', { videoId: 'v1', position: 'end' }],
      ['/api/remote/queue/add', 'POST', { videoId: 'v2', position: 'next' }],
      ['/api/remote/queue/remove', 'POST', { videoId: 'v1' }],
      ['/api/remote/queue/played', 'POST', { videoId: 'v2' }],
      ['/api/remote/queue/clear', 'POST', {}],
      ['/api/remote/queue/auto-fill', 'POST', { enabled: false }],
    ]);
  });
});

describe('autoplay preference', () => {
  it('is on by default', () => {
    expect(getAutoplay(memoryStorage())).toBe(true);
  });

  it('remembers off and on', () => {
    const storage = memoryStorage();
    setAutoplay(false, storage);
    expect(storage.getItem(AUTOPLAY_STORAGE_KEY)).toBe('0');
    expect(getAutoplay(storage)).toBe(false);
    setAutoplay(true, storage);
    expect(getAutoplay(storage)).toBe(true);
  });

  it('stays on when storage is unavailable', () => {
    const broken = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } };
    expect(() => setAutoplay(false, broken)).not.toThrow();
    expect(getAutoplay(broken)).toBe(true);
  });
});
