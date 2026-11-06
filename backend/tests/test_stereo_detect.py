"""Side-by-side stereo detection from frames (services.stereo_detect) and how
the VR decision uses it (routes.vr_api)."""
import random

import pytest

from routes import vr_api
from services import stereo_detect
from services.stereo_detect import SIZE, frame_score, layout_from_scores


@pytest.fixture(autouse=True)
def _clean():
    stereo_detect._reset_for_tests()
    yield
    stereo_detect._reset_for_tests()


def _texture(w, h, seed):
    """Blobs of random width and value in 4-row bands: detail that survives
    the 128 px scale like a render's, with no grid that repeats across the
    frame (a regular grid would look 'stereo' to any shift test)."""
    rnd = random.Random(seed)
    rows = []
    for _ in range(0, h, 4):
        row = []
        while len(row) < w:
            row.extend([rnd.randrange(256)] * rnd.randint(2, 7))
        rows.extend([row[:w]] * 4)
    return rows[:h]


def _frame(rows):
    return bytes(v for row in rows for v in row)


def _sbs(seed, parallax=2):
    half = SIZE // 2
    eye = _texture(half + 8, SIZE, seed)
    return _frame([row[:half] + row[parallax:half + parallax] for row in eye])


def _flat(seed):
    return _frame(_texture(SIZE, SIZE, seed))


def _pillarboxed(seed):
    bar = SIZE // 4
    pic = _texture(SIZE - 2 * bar, SIZE, seed)
    return _frame([[0] * bar + row + [0] * bar for row in pic])


class TestFrames:
    def test_two_eyes_with_parallax_score_high(self):
        assert frame_score(_sbs(1)) > stereo_detect.STEREO_MIN

    def test_one_picture_scores_low(self):
        assert frame_score(_flat(2)) < stereo_detect.MONO_MAX

    def test_black_frame_says_nothing(self):
        assert frame_score(bytes(SIZE * SIZE)) is None

    def test_picture_between_wide_side_bars_is_mono(self):
        assert frame_score(_pillarboxed(3)) == 'mono'


class TestLayout:
    def test_second_lowest_frame_decides(self):
        # One odd frame (title card) can't make a stereo video flat ...
        assert layout_from_scores([0.05, 0.6, 0.7, 0.8]) == 'sbs'
        # ... nor one stereo-looking frame make a flat one VR.
        assert layout_from_scores([0.05, 0.1, 0.12, 0.9]) == 'mono'

    def test_in_between_or_too_few_frames_is_undecided(self):
        assert layout_from_scores([0.25, 0.26, 0.27]) is None
        assert layout_from_scores([0.9, None, None]) is None
        assert layout_from_scores([]) is None

    def test_side_bars_in_most_frames_are_mono(self):
        assert layout_from_scores(['mono', 'mono', 0.9, 0.9]) == 'mono'
        assert layout_from_scores(['mono', 0.9, 0.8, 0.85]) == 'sbs'


class TestDecision:
    def _video(self, name, path='/lib/v.mp4', **extra):
        return {'name': name, 'path': path, **extra}

    def test_side_by_side_frames_make_an_untagged_video_vr(self):
        stereo_detect._set_layout_for_tests('/lib/v.mp4', 'sbs')
        # 16:9 frame, eyes 8:9 each: 180° VR.
        assert vr_api.detect_vr_format_for_video(self._video('Plain.mp4'), dims=(7680, 4320)) == ('dome', 'sbs', True)

    def test_wide_eyes_are_flat_3d(self):
        stereo_detect._set_layout_for_tests('/lib/v.mp4', 'sbs')
        assert vr_api.detect_vr_format_for_video(self._video('Fap Hero.mp4'), dims=(3840, 1080)) == ('flat', 'sbs', True)

    def test_one_picture_beats_a_vr_shaped_frame(self):
        stereo_detect._set_layout_for_tests('/lib/v.mp4', 'mono')
        _, _, is_3d = vr_api.detect_vr_format_for_video(self._video('Collage PMV.mp4'), dims=(4320, 2160))
        assert is_3d is False

    def test_undecided_frames_fall_back_to_the_frame_shape(self):
        _, _, is_3d = vr_api.detect_vr_format_for_video(self._video('Collage PMV.mp4'), dims=(4320, 2160))
        assert is_3d is True

    def test_filename_and_manual_toggle_still_win(self):
        stereo_detect._set_layout_for_tests('/lib/v.mp4', 'mono')
        assert vr_api.detect_vr_format_for_video(self._video('Scene_180_LR.mp4'))[2] is True
        stereo_detect._set_layout_for_tests('/lib/v.mp4', 'sbs')
        assert vr_api.detect_vr_format_for_video(self._video('Plain.mp4', manualVRType='flat'))[2] is False

    def test_a_decided_layout_answers_without_dimensions(self):
        stereo_detect._set_layout_for_tests('/lib/v.mp4', 'mono')
        assert vr_api.vr_decision('unknown-id', self._video('Plain.mp4')) is False


class TestCache:
    def test_results_survive_a_restart_and_follow_the_file(self, tmp_path, monkeypatch):
        video = tmp_path / 'v.mp4'
        video.write_bytes(b'x' * 10)
        calls = []
        monkeypatch.setattr(stereo_detect, 'analyze', lambda p, d=0: calls.append(p) or 'sbs')
        stereo_detect.set_cache_dir(str(tmp_path))
        stereo_detect.queue_checks([(str(video), 10)])
        _wait()
        assert stereo_detect.cached_layout(str(video)) == 'sbs' and len(calls) == 1

        stereo_detect._reset_for_tests()
        stereo_detect.set_cache_dir(str(tmp_path))
        assert stereo_detect.queue_checks([(str(video), 10)]) == 0  # answered from the file
        assert stereo_detect.cached_layout(str(video)) == 'sbs' and len(calls) == 1

        video.write_bytes(b'y' * 20)  # replaced: looked at again
        assert stereo_detect.queue_checks([(str(video), 10)]) == 1
        _wait()
        assert len(calls) == 2


def _wait():
    import time
    for _ in range(200):
        if not stereo_detect._worker_running:
            return
        time.sleep(0.01)
    raise AssertionError('stereo worker did not finish')
