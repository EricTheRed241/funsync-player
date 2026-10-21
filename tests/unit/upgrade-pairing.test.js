import { describe, it, expect } from 'vitest';
import { pairUpgradeJobs } from '../../scripts/upgrade-pairing.mjs';

const jobOf = (r, script) => r.jobs.find((j) => j.script === script);

describe('pairUpgradeJobs', () => {
  it("names companions after the video for an exact match, after the script for a variant", () => {
    const r = pairUpgradeJobs(
      ['/lib/T/Title.mp4'],
      ['/lib/T/Title.funscript', '/lib/T/Title_hardcore.funscript', '/lib/T/Title (Soft).funscript'],
    );
    expect(jobOf(r, '/lib/T/Title.funscript').video).toBe('/lib/T/Title.mp4');
    expect(jobOf(r, '/lib/T/Title_hardcore.funscript').video).toBeNull();
    expect(jobOf(r, '/lib/T/Title (Soft).funscript').video).toBeNull();
    expect(r.orphanScripts).toEqual([]);
  });

  it('skips a walked script with no video, including the bare copy beside per-resolution videos', () => {
    const r = pairUpgradeJobs(
      ['/lib/C/City_4k.mp4', '/lib/C/City_6k.mp4'],
      ['/lib/C/City.funscript', '/lib/C/City_4k.funscript', '/lib/C/City_6k.funscript',
        '/lib/Lost/Lost Scene.funscript'],
    );
    expect(r.orphanScripts.sort()).toEqual(['/lib/C/City.funscript', '/lib/Lost/Lost Scene.funscript']);
    expect(r.jobs.map((j) => j.script).sort()).toEqual(['/lib/C/City_4k.funscript', '/lib/C/City_6k.funscript']);
  });

  it('does not treat a script as a variant of a video in another folder', () => {
    const r = pairUpgradeJobs(['/lib/A/Title.mp4', '/lib/A/Other.mp4'], ['/lib/B/Title_hard.funscript']);
    expect(r.orphanScripts).toEqual(['/lib/B/Title_hard.funscript']);
  });

  it('always upgrades a script named on the command line', () => {
    const r = pairUpgradeJobs([], ['/x/Lone.funscript', '/x/Other.funscript'], new Set(['/x/Lone.funscript']));
    expect(r.jobs).toEqual([{ video: null, script: '/x/Lone.funscript' }]);
    expect(r.orphanScripts).toEqual(['/x/Other.funscript']);
  });
});
