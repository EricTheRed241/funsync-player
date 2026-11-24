/**
 * @vitest-environment node
 */
// TCode device profiles: one device at a time, each keeping its own settings,
// and the FUNSR MINI settings commands.

import { describe, it, expect } from 'vitest';
import {
  activeProfileId,
  switchProfile,
  funsrCommands,
  funsrStrokeCm,
} from '../../renderer/js/tcode-profiles.js';

/** Apply a switch the way the connection panel does: fields + profiles into `tcode`. */
function apply(tcode, change) {
  return { ...tcode, ...change.fields, profiles: change.profiles };
}

const mirabotSettings = {
  transport: 'websocket',
  wsUrl: 'ws://mirabot.local:8888',
  precision: 4,
  updateRateHz: 60,
  defaultOffset: -120,
  autoConnect: true,
  axes: { L0: { enabled: true, min: 10, max: 90 }, R1: { enabled: true, inverted: true } },
};

describe('activeProfileId', () => {
  it('takes pre-profile MiraBot WebSocket settings as the MiraBot profile', () => {
    expect(activeProfileId(mirabotSettings)).toBe('mirabot');
  });

  it('takes any other pre-profile settings as Custom', () => {
    expect(activeProfileId({ transport: 'serial', port: '/dev/cu.usbserial-1' })).toBe('custom');
    expect(activeProfileId({ transport: 'websocket', wsUrl: 'ws://restim.local:12346' })).toBe('custom');
  });

  it('falls back from an unknown saved profile id', () => {
    expect(activeProfileId({ ...mirabotSettings, profiles: { active: 'gone' } })).toBe('mirabot');
  });
});

describe('switchProfile', () => {
  it('first switch to FUNSR MINI loads its safe defaults, not the MiraBot settings', () => {
    const change = switchProfile(mirabotSettings, 'funsr-mini');
    expect(change.profiles.active).toBe('funsr-mini');
    expect(change.fields.transport).toBe('serial');
    expect(change.fields.baudRate).toBe(115200);
    expect(change.fields.precision).toBe(4);
    expect(change.fields.wsUrl).toBe('');
    expect(change.fields.defaultOffset).toBeNull();
    // Stroke kept off the end stops; stroke + twist only.
    expect(change.fields.axes.L0).toMatchObject({ enabled: true, min: 5, max: 95 });
    expect(change.fields.axes.R0.enabled).toBe(true);
    expect(change.fields.axes.R1.enabled).toBe(false);
    expect(change.fields.axes.V0.enabled).toBe(false);
  });

  it('switching back restores every MiraBot setting, and FUNSR edits survive the round trip', () => {
    let tcode = { ...mirabotSettings };
    tcode = apply(tcode, switchProfile(tcode, 'funsr-mini'));
    // User picks the port, tunes the stroke and offset on the FUNSR MINI.
    tcode = {
      ...tcode,
      port: '/dev/cu.usbserial-110',
      autoConnect: true,
      defaultOffset: 40,
      funsr: { dag: 1.8 },
      axes: { ...tcode.axes, L0: { enabled: true, min: 8, max: 92 } },
    };
    tcode = apply(tcode, switchProfile(tcode, 'mirabot'));
    expect(tcode.transport).toBe('websocket');
    expect(tcode.wsUrl).toBe('ws://mirabot.local:8888');
    expect(tcode.defaultOffset).toBe(-120);
    expect(tcode.axes).toEqual(mirabotSettings.axes);
    expect(tcode.funsr).toBeNull();

    tcode = apply(tcode, switchProfile(tcode, 'funsr-mini'));
    expect(tcode.port).toBe('/dev/cu.usbserial-110');
    expect(tcode.defaultOffset).toBe(40);
    expect(tcode.funsr).toEqual({ dag: 1.8 });
    expect(tcode.axes.L0).toEqual({ enabled: true, min: 8, max: 92 });
  });

  it('does not share axis objects between the live settings and the saved profile', () => {
    const tcode = { ...mirabotSettings };
    const change = switchProfile(tcode, 'funsr-mini');
    tcode.axes.L0.min = 50;
    expect(change.profiles.saved.mirabot.axes.L0.min).toBe(10);
  });

  it('is a no-op for the active profile or an unknown one', () => {
    expect(switchProfile(mirabotSettings, 'mirabot')).toBeNull();
    expect(switchProfile(mirabotSettings, 'nope')).toBeNull();
  });
});

describe('funsrCommands', () => {
  it('sends only the settings the user set, in the firmware format', () => {
    expect(funsrCommands({ dag: 1.53, dkp: 1.2, dsp: 1 })).toEqual(['DAG1.53\n', 'DKP1.2\n', 'DSP1\n']);
    expect(funsrCommands({ dkp: 5 })).toEqual(['DKP5.0\n']);
    expect(funsrCommands(null)).toEqual([]);
    expect(funsrCommands({})).toEqual([]);
  });

  it('clamps out-of-range values to the manual limits and skips garbage', () => {
    expect(funsrCommands({ dag: 0.2, dkp: 9 })).toEqual(['DAG1.00\n', 'DKP5.0\n']);
    expect(funsrCommands({ dag: 'x', dkp: null, dsp: 4 })).toEqual([]);
  });
});

describe('funsrStrokeCm', () => {
  it('matches the manual table at its rows and interpolates between them', () => {
    expect(funsrStrokeCm(1.0)).toBeCloseTo(17.92, 2);
    expect(funsrStrokeCm(3.0)).toBeCloseTo(6.39, 2);
    expect(funsrStrokeCm(1.55)).toBeCloseTo((12.51 + 11.79) / 2, 2);
    expect(funsrStrokeCm(9)).toBeCloseTo(6.39, 2);
  });
});
