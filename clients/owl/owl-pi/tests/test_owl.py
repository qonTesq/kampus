import json
from pathlib import Path
import sys
import unittest
from unittest.mock import Mock
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from state import OwlState, EMOTIONS
from eyes import render_eye, Animator
from display import INIT, rgb565, PiDisplays
from main import phrase_matches, load_config
from realtime_voice import session_settings

class Clock:
    def __init__(self): self.now = 0
    def __call__(self): return self.now

class StateTests(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.state = OwlState(300, self.clock)
        self.state.wake(); self.state.phase("idle")

    def test_five_minutes_exactly(self):
        self.clock.now = 299.9
        self.assertFalse(self.state.should_sleep())
        self.clock.now = 300
        self.assertTrue(self.state.should_sleep())

    def test_speech_restarts_timer_but_emotion_does_not(self):
        self.clock.now = 250
        self.state.speech(True); self.state.speech(False)
        self.state.phase("idle")
        self.clock.now = 400
        self.state.emotion("money")
        self.assertFalse(self.state.should_sleep())
        self.clock.now = 550
        self.assertTrue(self.state.should_sleep())

    def test_never_sleep_during_reply_or_human_speech(self):
        self.clock.now = 500
        self.assertFalse(self.state.should_sleep(busy=True))
        self.state.speech(True)
        self.clock.now = 900
        self.assertFalse(self.state.should_sleep())

    def test_sleep_and_wake(self):
        self.state.sleep()
        self.assertEqual(self.state.snapshot().phase, "sleeping")
        self.assertFalse(self.state.should_sleep())
        self.state.wake()
        self.assertEqual(self.state.snapshot().phase, "connecting")

    def test_expression_allowlist(self):
        for emotion in EMOTIONS: self.state.emotion(emotion)
        with self.assertRaises(ValueError): self.state.emotion("run_shell")

class RenderTests(unittest.TestCase):
    def test_all_expressions_and_blinks(self):
        for emotion in EMOTIONS + ("sleeping",):
            for eye in (0, 1):
                for opening in (0, .3, 1):
                    image = render_eye(eye, emotion, opening)
                    self.assertEqual(image.size, (240, 240))
                    self.assertEqual(image.mode, "RGB")
                    arr = np.asarray(image)
                    self.assertEqual(tuple(arr[0, 0]), (0, 0, 0))
            if emotion != "sleeping":
                self.assertEqual(np.asarray(render_eye(0, emotion, 0)).max(), 0)

    def test_crescents_are_mirrored(self):
        left = np.asarray(render_eye(0))
        right = np.asarray(render_eye(1))
        np.testing.assert_array_equal(left[:, ::-1], right)

    def test_money_and_sleep_differ(self):
        self.assertFalse(np.array_equal(render_eye(0, "money"), render_eye(0, "sleeping")))
        # No lettering in the old Zzz region.
        self.assertEqual(np.asarray(render_eye(0, "sleeping"))[198:].max(), 0)

    def test_animation_reaches_requested_expression(self):
        state = OwlState()
        state.wake(); state.phase("idle"); state.emotion("love")
        animator = Animator(seed=1)
        animator.pose(state.snapshot(), 0)
        self.assertEqual(animator.pose(state.snapshot(), .2)[0], "love")

    def test_rgb565_byte_order(self):
        from PIL import Image
        self.assertEqual(rgb565(Image.new("RGB", (1, 1), "red")).tobytes(), b"\xf8\x00")

class ProtocolTests(unittest.TestCase):
    def test_each_screen_uses_its_own_dc_pin(self):
        driver = PiDisplays.__new__(PiDisplays)
        driver.dcs = [Mock(), Mock()]
        driver.spies = [Mock(), Mock()]
        driver.command(1, 0x36, b"\x48")
        driver.dcs[0].off.assert_not_called()
        driver.dcs[0].on.assert_not_called()
        driver.spies[0].writebytes.assert_not_called()
        driver.dcs[1].off.assert_called_once()
        driver.dcs[1].on.assert_called_once()
        driver.spies[1].writebytes.assert_called_once_with([0x36])
        driver.spies[1].writebytes2.assert_called_once_with(b"\x48")

    def test_init_sequence_is_well_formed(self):
        pos = 0
        while pos < len(INIT):
            count = INIT[pos+1]; pos += 2 + (count & 127)
            if count & 128: pos += 1
            self.assertLessEqual(pos, len(INIT))
        self.assertEqual(pos, len(INIT))

    def test_wake_phrase_has_word_boundaries(self):
        self.assertTrue(phrase_matches("Hey Owl!", ["hey owl"]))
        self.assertFalse(phrase_matches("hey owlet", ["hey owl"]))
        self.assertFalse(phrase_matches("hello world", ["hey owl"]))

    def test_config_and_session_schema(self):
        cfg = load_config(Path(__file__).resolve().parents[1] / "config.toml")
        self.assertEqual(cfg["assistant"]["sleep_after_seconds"], 300)
        self.assertEqual(cfg["display"]["left_dc_gpio"], 25)
        self.assertEqual(cfg["display"]["right_dc_gpio"], 24)
        session = session_settings(cfg)
        json.dumps(session)
        self.assertEqual(session["audio"]["input"]["format"]["rate"], 24000)
        self.assertFalse(session["audio"]["input"]["turn_detection"]["create_response"])
        self.assertEqual(session["tools"][0]["name"], "prepare_reply")

if __name__ == "__main__": unittest.main()
