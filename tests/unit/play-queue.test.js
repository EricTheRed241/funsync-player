// The shared play queue (play-queue.js): auto-fill candidates and order, the
// played map, queue changes, and the queue edits from the phone and the
// headset (library-edits.js).
import { describe, it, expect, vi } from 'vitest';
import {
  autoFillQueue, stampPlayed, applyQueueChange, reviewPlaylistPaths, DAY_MS, PLAYED_CAP,
} from '../../renderer/js/play-queue.js';
import { applyLibraryEdit, LibraryEditApplier } from '../../renderer/js/library-edits.js';
import { AGE_SCREEN, DELETE_AT_OR_BELOW, QUEUE_AUTOFILL } from '../../renderer/js/content-preferences.js';

const NOW = 1_800_000_000_000;
const v = (path, extra = {}) => ({ path, hasFunscript: true, dateAdded: 0, ...extra });

describe('autoFillQueue — candidates', () => {
  it('skips videos without a script, queued ones, the current one, the review list and rejects', () => {
    const videos = [
      v('/ok.mp4'),
      v('/noscript.mp4', { hasFunscript: false }),
      v('/manual.mp4'),
      v('/current.mp4'),
      v('/review.mp4'),
      v('/reject.mp4'),
      v('/reject-fav.mp4'),
      v('/above.mp4'),
    ];
    const ratings = {
      '/reject.mp4': { rating: DELETE_AT_OR_BELOW },
      '/reject-fav.mp4': { rating: DELETE_AT_OR_BELOW, favorite: true },
      '/above.mp4': { rating: DELETE_AT_OR_BELOW + 0.5 },
    };
    const out = autoFillQueue({
      videos, manual: ['/manual.mp4'], ratings, currentPath: '/current.mp4', reviewPaths: ['/review.mp4'], now: NOW,
    });
    expect(out.sort()).toEqual(['/above.mp4', '/ok.mp4', '/reject-fav.mp4']);
  });

  it('lists a path once even when the library has it twice', () => {
    expect(autoFillQueue({ videos: [v('/a.mp4'), v('/a.mp4')], now: NOW })).toEqual(['/a.mp4']);
  });
});

describe('autoFillQueue — order', () => {
  it('not played lately, then unrated, then score, then newest, then path', () => {
    const videos = [
      // Played lately: last, whatever else it has going for it.
      v('/played.mp4', { dateAdded: 900 }),
      // Rated: after every unrated one, even with the top score.
      v('/rated-top.mp4', { dateAdded: 900 }),
      v('/rated-low.mp4', { dateAdded: 900 }),
      // Unrated, by score; equal scores by date added, newest first; then path.
      v('/b-old.mp4', { dateAdded: 100 }),
      v('/b-new.mp4', { dateAdded: 500 }),
      v('/a-new.mp4', { dateAdded: 500 }),
      v('/hi.mp4', { dateAdded: 1 }),
      v('/noscore.mp4', { dateAdded: 1000 }),
      v('/negative.mp4', { dateAdded: 1000 }),
    ];
    const ratings = { '/rated-top.mp4': { rating: 5 }, '/rated-low.mp4': { rating: 3 } };
    const scores = {
      '/played.mp4': 99, '/rated-top.mp4': 50, '/rated-low.mp4': 1,
      '/b-old.mp4': 2, '/b-new.mp4': 2, '/a-new.mp4': 2, '/hi.mp4': 10, '/negative.mp4': -1,
    };
    const played = { '/played.mp4': NOW - DAY_MS };
    expect(autoFillQueue({ videos, ratings, scores, played, now: NOW, size: 20 })).toEqual([
      '/hi.mp4', '/a-new.mp4', '/b-new.mp4', '/b-old.mp4', '/noscore.mp4', '/negative.mp4',
      '/rated-top.mp4', '/rated-low.mp4',
      '/played.mp4',
    ]);
  });

  it('a favourite without stars counts as unrated', () => {
    const ratings = { '/fav.mp4': { favorite: true }, '/rated.mp4': { rating: 4 } };
    expect(autoFillQueue({ videos: [v('/rated.mp4'), v('/fav.mp4')], ratings, now: NOW })).toEqual(['/fav.mp4', '/rated.mp4']);
  });

  it('a play older than the replay window no longer holds a video back', () => {
    const window = QUEUE_AUTOFILL.replayAfterDays * DAY_MS;
    const videos = [v('/recent.mp4', { dateAdded: 3 }), v('/stale.mp4', { dateAdded: 2 }), v('/never.mp4', { dateAdded: 1 })];
    const played = { '/recent.mp4': NOW - window + 1, '/stale.mp4': NOW - window };
    expect(autoFillQueue({ videos, played, now: NOW })).toEqual(['/stale.mp4', '/never.mp4', '/recent.mp4']);
    expect(autoFillQueue({ videos, played, now: NOW, replayAfterMs: 0 })).toEqual(['/recent.mp4', '/stale.mp4', '/never.mp4']);
  });

  it('when only played videos are left, the one played longest ago comes first, rated or not', () => {
    const videos = [v('/just-now.mp4'), v('/yesterday.mp4'), v('/last-week.mp4')];
    const ratings = { '/last-week.mp4': { rating: 5 } };
    const played = { '/just-now.mp4': NOW - 1000, '/yesterday.mp4': NOW - DAY_MS, '/last-week.mp4': NOW - 7 * DAY_MS };
    expect(autoFillQueue({ videos, ratings, scores: { '/just-now.mp4': 9 }, played, now: NOW }))
      .toEqual(['/last-week.mp4', '/yesterday.mp4', '/just-now.mp4']);
  });
});

describe('autoFillQueue — size', () => {
  const videos = Array.from({ length: 30 }, (_, i) => v(`/v${String(i).padStart(2, '0')}.mp4`));

  it('fills up to the queue size by default', () => {
    expect(autoFillQueue({ videos, now: NOW })).toHaveLength(QUEUE_AUTOFILL.size);
  });

  it('counts the user queue towards the size', () => {
    const out = autoFillQueue({ videos, manual: ['/x.mp4', '/y.mp4', '/z.mp4'], size: 10, now: NOW });
    expect(out).toHaveLength(7);
    expect(autoFillQueue({ videos, manual: videos.slice(0, 12).map((x) => x.path), size: 10, now: NOW })).toEqual([]);
  });

  it('returns what there is when the library is small', () => {
    expect(autoFillQueue({ videos: videos.slice(0, 3), size: 10, now: NOW })).toHaveLength(3);
    expect(autoFillQueue({ videos: [], now: NOW })).toEqual([]);
  });
});

describe('stampPlayed', () => {
  it('stamps a copy and drops entries outside the window', () => {
    const before = { '/old.mp4': NOW - 40 * DAY_MS, '/edge.mp4': NOW - 30 * DAY_MS, '/keep.mp4': NOW - DAY_MS };
    const after = stampPlayed(before, '/new.mp4', NOW, { windowMs: 30 * DAY_MS });
    expect(after).toEqual({ '/keep.mp4': NOW - DAY_MS, '/new.mp4': NOW });
    expect(before['/old.mp4']).toBeDefined();
  });

  it('keeps at most `cap` entries, newest first', () => {
    const played = Object.fromEntries(Array.from({ length: 5 }, (_, i) => [`/v${i}.mp4`, NOW - (i + 1) * 1000]));
    expect(Object.keys(stampPlayed(played, '/now.mp4', NOW, { cap: 3 })).sort()).toEqual(['/now.mp4', '/v0.mp4', '/v1.mp4']);
  });

  it('caps at PLAYED_CAP by default', () => {
    const played = Object.fromEntries(Array.from({ length: PLAYED_CAP + 50 }, (_, i) => [`/v${i}.mp4`, NOW - i - 1]));
    const out = stampPlayed(played, '/now.mp4', NOW);
    expect(Object.keys(out)).toHaveLength(PLAYED_CAP);
    expect(out['/now.mp4']).toBe(NOW);
    expect(out[`/v${PLAYED_CAP + 49}.mp4`]).toBeUndefined();
  });
});

describe('reviewPlaylistPaths', () => {
  it('finds the age screener playlist by name, any case', () => {
    const playlists = [
      { name: 'Evening', videoPaths: ['/a.mp4'] },
      { name: ` ${AGE_SCREEN.playlist.toUpperCase()} `, videoPaths: ['/r.mp4'] },
    ];
    expect(reviewPlaylistPaths(playlists)).toEqual(['/r.mp4']);
    expect(reviewPlaylistPaths([])).toEqual([]);
    expect(reviewPlaylistPaths(undefined)).toEqual([]);
  });
});

describe('applyQueueChange', () => {
  const state = { manual: ['/a.mp4', '/b.mp4'], played: {}, autoFill: true };

  it('add: end appends, next goes first; queued: next moves it to the front, end leaves it', () => {
    expect(applyQueueChange(state, { kind: 'add', path: '/c.mp4', position: 'end' }).manual).toEqual(['/a.mp4', '/b.mp4', '/c.mp4']);
    expect(applyQueueChange(state, { kind: 'add', path: '/c.mp4', position: 'next' }).manual).toEqual(['/c.mp4', '/a.mp4', '/b.mp4']);
    expect(applyQueueChange(state, { kind: 'add', path: '/b.mp4', position: 'next' }).manual).toEqual(['/b.mp4', '/a.mp4']);
    expect(applyQueueChange(state, { kind: 'add', path: '/b.mp4', position: 'end' }).manual).toBe(state.manual);
    expect(applyQueueChange(state, { kind: 'add', path: '/a.mp4', position: 'next' }).manual).toBe(state.manual);
  });

  it('remove: drops a queued video; anything else is stamped as skipped', () => {
    const removed = applyQueueChange(state, { kind: 'remove', path: '/a.mp4' }, { now: NOW });
    expect(removed.manual).toEqual(['/b.mp4']);
    expect(removed.played).toBe(state.played);
    const skipped = applyQueueChange(state, { kind: 'remove', path: '/auto.mp4' }, { now: NOW });
    expect(skipped.manual).toBe(state.manual);
    expect(skipped.played).toEqual({ '/auto.mp4': NOW });
  });

  it('a skipped auto-filled video makes room for the next candidate', () => {
    const videos = [v('/x.mp4', { dateAdded: 3 }), v('/y.mp4', { dateAdded: 2 }), v('/z.mp4', { dateAdded: 1 })];
    const before = { manual: [], played: {}, autoFill: true };
    expect(autoFillQueue({ videos, ...before, size: 2, now: NOW })).toEqual(['/x.mp4', '/y.mp4']);
    const after = applyQueueChange(before, { kind: 'remove', path: '/x.mp4' }, { now: NOW });
    expect(autoFillQueue({ videos, ...after, size: 2, now: NOW })).toEqual(['/y.mp4', '/z.mp4']);
  });

  it('played: leaves the user queue and is stamped', () => {
    const out = applyQueueChange(state, { kind: 'played', path: '/b.mp4' }, { now: NOW });
    expect(out.manual).toEqual(['/a.mp4']);
    expect(out.played).toEqual({ '/b.mp4': NOW });
    expect(applyQueueChange(state, { kind: 'played', path: '/elsewhere.mp4' }, { now: NOW }).manual).toBe(state.manual);
  });

  it('clear empties the user queue only; auto-fill sets the flag', () => {
    const played = { '/p.mp4': NOW };
    const cleared = applyQueueChange({ ...state, played }, { kind: 'clear' });
    expect(cleared.manual).toEqual([]);
    expect(cleared.played).toBe(played);
    expect(applyQueueChange(state, { kind: 'auto-fill', enabled: false }).autoFill).toBe(false);
    expect(applyQueueChange({ ...state, autoFill: false }, { kind: 'auto-fill', enabled: true }).autoFill).toBe(true);
  });

  it('rejects an unknown change', () => {
    expect(() => applyQueueChange(state, { kind: 'shuffle' })).toThrow(/unknown queue change/);
  });
});

describe('queue edits (library-edits.js)', () => {
  const hooks = () => ({
    add: vi.fn(), remove: vi.fn(), played: vi.fn(), clear: vi.fn(), setAutoFill: vi.fn(),
  });

  it('calls the matching hook with the edit values', async () => {
    const queue = hooks();
    await applyLibraryEdit({ kind: 'queue-add', path: '/a.mp4', position: 'next', source: 'web-remote' }, {}, { queue });
    await applyLibraryEdit({ kind: 'queue-add', path: '/b.mp4' }, {}, { queue });
    await applyLibraryEdit({ kind: 'queue-remove', path: '/c.mp4' }, {}, { queue });
    await applyLibraryEdit({ kind: 'queue-played', path: '/d.mp4', source: 'heresphere' }, {}, { queue });
    await applyLibraryEdit({ kind: 'queue-clear' }, {}, { queue });
    await applyLibraryEdit({ kind: 'queue-auto-fill', enabled: false }, {}, { queue });
    expect(queue.add.mock.calls).toEqual([['/a.mp4', 'next'], ['/b.mp4', 'end']]);
    expect(queue.remove).toHaveBeenCalledWith('/c.mp4');
    expect(queue.played).toHaveBeenCalledWith('/d.mp4');
    expect(queue.clear).toHaveBeenCalledTimes(1);
    expect(queue.setAutoFill).toHaveBeenCalledWith(false);
  });

  it('rejects queue edits without the queue hooks', async () => {
    for (const edit of [
      { kind: 'queue-add', path: '/a.mp4' },
      { kind: 'queue-remove', path: '/a.mp4' },
      { kind: 'queue-played', path: '/a.mp4' },
      { kind: 'queue-clear' },
      { kind: 'queue-auto-fill', enabled: true },
    ]) {
      await expect(applyLibraryEdit(edit, {})).rejects.toThrow(/no play queue/);
      await expect(applyLibraryEdit(edit, {}, { queue: {} })).rejects.toThrow(/no play queue/);
    }
  });

  it('rejects malformed queue edits without touching the queue', async () => {
    const queue = hooks();
    await expect(applyLibraryEdit({ kind: 'queue-add', position: 'end' }, {}, { queue })).rejects.toThrow(/path/);
    await expect(applyLibraryEdit({ kind: 'queue-add', path: '/a.mp4', position: 'top' }, {}, { queue })).rejects.toThrow(/position/);
    await expect(applyLibraryEdit({ kind: 'queue-auto-fill', enabled: 'yes' }, {}, { queue })).rejects.toThrow(/enabled/);
    expect(queue.add).not.toHaveBeenCalled();
    expect(queue.setAutoFill).not.toHaveBeenCalled();
  });

  it('the applier hands the queue hooks through, then publishes and acks', async () => {
    const log = [];
    const applier = new LibraryEditApplier({
      data: {},
      publish: async () => { log.push('publish'); },
      ack: (seq, error) => log.push(error ? `ack ${seq} ${error}` : `ack ${seq}`),
      queue: { ...hooks(), add: async (path, position) => { log.push(`add ${path} ${position}`); } },
    });
    applier.apply({ seq: 1, kind: 'queue-add', path: '/a.mp4', position: 'end' });
    await applier.apply({ seq: 2, kind: 'queue-auto-fill' });
    expect(log).toEqual([
      'add /a.mp4 end', 'publish', 'ack 1',
      'publish', 'ack 2 queue-auto-fill: enabled must be true or false',
    ]);
  });
});
