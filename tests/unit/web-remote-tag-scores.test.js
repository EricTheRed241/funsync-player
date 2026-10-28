/**
 * @vitest-environment jsdom
 * Web remote tag scores (backend/web-remote/tag-scores.js): the −/+ steps,
 * how a score reads, the server state, waiting for the desktop, and the
 * editor in the Tags view.
 */
import { describe, it, expect, vi } from 'vitest';
import {
  stepTagScore, formatTagScore, parseTagScores, fetchTagScores, setTagScore, requestRecalculation, pollTagScores,
  createTagScoreEditor, updateTagScoreEditor, formatRules, TAG_SCORE_MAX,
} from '../../backend/web-remote/tag-scores.js';

const jsonResponse = (data, status = 200) => ({ ok: status < 400, status, json: async () => data });

describe('stepping a tag score', () => {
  it('moves in 0.5 steps from 0 and stops at ±10', () => {
    expect([stepTagScore(0, 1), stepTagScore(0, -1), stepTagScore(1.5, 1), stepTagScore(-0.5, 1)]).toEqual([0.5, -0.5, 2, 0]);
    expect(Object.is(stepTagScore(-0.5, 1), 0)).toBe(true);
    expect([stepTagScore(10, 1), stepTagScore(-10, -1), stepTagScore(undefined, 1)]).toEqual([10, -10, 0.5]);
    expect(stepTagScore(1.2, 1)).toBe(1.5);
  });

  it('reads 0, +n and −n', () => {
    expect([0, 1.5, -2, undefined].map(formatTagScore)).toEqual(['0', '+1.5', '−2', '0']);
  });
});

describe('server state', () => {
  it('fills in defaults and keeps numbers (scores, built-in weights) and group names only', () => {
    const s = parseTagScores({
      scores: { riding: 2, bad: 'x' }, pending: ['riding'], recalculating: true,
      groups: { riding: 'Riding', cowgirl: 'Riding', odd: 3 }, rules: { vr: 3, hentaiOnly: -3, bad: 'x' },
    });
    expect(s.scores).toEqual({ riding: 2 });
    expect(s.groups).toEqual({ riding: 'Riding', cowgirl: 'Riding' });
    expect(s.rules).toEqual({ vr: 3, hentaiOnly: -3 });
    expect([...s.pending]).toEqual(['riding']);
    expect(s.recalculating).toBe(true);
    expect(parseTagScores(null)).toEqual({ scores: {}, groups: {}, rules: {}, pending: new Set(), recalculating: false });
  });

  it('GETs the scores and POSTs a score or a recalculation', async () => {
    const fetchImpl = vi.fn(async () => jsonResponse({ scores: { pmv: -1 }, pending: [], recalculating: false }));
    expect((await fetchTagScores(fetchImpl)).scores).toEqual({ pmv: -1 });
    expect(fetchImpl).toHaveBeenLastCalledWith('/api/remote/tag-scores');
    await setTagScore('pmv', 1.5, fetchImpl);
    expect(fetchImpl.mock.calls.at(-1)[0]).toBe('/api/remote/tag-score');
    expect(JSON.parse(fetchImpl.mock.calls.at(-1)[1].body)).toEqual({ tag: 'pmv', score: 1.5 });
    await requestRecalculation(fetchImpl);
    expect(fetchImpl.mock.calls.at(-1)[0]).toBe('/api/remote/recalculate-scores');
    await expect(fetchTagScores(async () => jsonResponse({}, 500))).rejects.toThrow(/500/);
  });

  it('lists the favourite-artist bonus right after the known-artist weight', () => {
    // No bundle loaded: t() returns the key, which shows the order.
    const line = formatRules({ likes: 1, favouriteArtist: 5, artistInLibrary: 2, vr: 0 });
    expect(line.split(' · ').map((s) => s.split('.').at(-1))).toEqual(['artistInLibrary', 'favouriteArtist', 'likes']);
  });

  it('polls until the desktop is done, past failed fetches, or gives up', async () => {
    const states = [new Error('down'), { pending: ['pmv'], recalculating: true }, { pending: [], recalculating: false }];
    const fetchImpl = vi.fn(async () => {
      const next = states.shift();
      if (next instanceof Error) throw next;
      return jsonResponse(next);
    });
    const sleep = vi.fn(async () => {});
    const done = await pollTagScores((s) => !s.pending.size && !s.recalculating, { fetchImpl, sleep, timeoutMs: 1e9 });
    expect(done.recalculating).toBe(false);
    expect(fetchImpl).toHaveBeenCalledTimes(3);

    let clock = 0;
    const never = await pollTagScores(() => false, {
      fetchImpl: async () => jsonResponse({ pending: ['x'] }), sleep: async () => { clock += 1000; }, now: () => clock, timeoutMs: 3000,
    });
    expect(never).toBeNull();
  });
});

describe('tag score editor', () => {
  it('steps on − / +, never reaches the row, and shows pending', () => {
    const onStep = vi.fn();
    const rowClick = vi.fn();
    const row = document.createElement('div');
    row.addEventListener('click', rowClick);
    const editor = createTagScoreEditor({ tag: 'riding', score: 1, onStep });
    row.appendChild(editor);
    const [down, up] = editor.querySelectorAll('button');
    expect(editor.querySelector('.tag-score__value').textContent).toBe('+1');
    up.click();
    down.click();
    expect(onStep.mock.calls).toEqual([[1.5], [0.5]]);
    expect(rowClick).not.toHaveBeenCalled();

    updateTagScoreEditor(editor, { score: -2, pending: true });
    expect(editor.querySelector('.tag-score__value').textContent).toBe('−2');
    expect(editor.classList.contains('tag-score--pending')).toBe(true);
    expect(editor.classList.contains('tag-score--negative')).toBe(true);
  });

  it('disables the step past the limit', () => {
    const onStep = vi.fn();
    const editor = createTagScoreEditor({ tag: 'pmv', score: TAG_SCORE_MAX, onStep });
    const up = editor.querySelector('.tag-score__btn--up');
    expect(up.disabled).toBe(true);
    up.click();
    expect(onStep).not.toHaveBeenCalled();
  });
});
