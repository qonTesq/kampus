"""Original yellow-crescent owl renderer; no camera or network access here."""
import random
import threading
import time

import numpy as np
from PIL import Image

YELLOW = (255, 224, 24)
WHITE = (255, 255, 255)
MONEY = (0b00100, 0b01111, 0b10100, 0b10100, 0b01110,
         0b00101, 0b00101, 0b11110, 0b00100)

def render_eye(eye, emotion="normal", openness=1.0, gaze=(0, 0),
               energy=0.0, pixel_size=4):
    if pixel_size not in (1, 2, 3, 4, 5, 6, 8):
        raise ValueError("pixel_size must divide 240")
    n = 240 // pixel_size
    axis = (np.arange(n) + .5) * pixel_size - 120
    x, y = np.meshgrid(axis, axis)
    result = np.zeros((n, n, 3), dtype=np.uint8)
    if emotion == "sleeping":
        outer = (x / 78)**2 + ((y + 12) / 22)**2 <= 1
        inner = (x / 75)**2 + ((y + 17) / 22)**2 <= 1
        result[outer & ~inner & (y >= -2)] = YELLOW
    else:
        height = 62
        if emotion == "sleepy": height = 30
        if emotion == "surprised": height = 68
        if emotion == "confused": height = 66 if eye == 0 else 35
        if emotion == "explaining": height = 59 + round(3 * energy)
        height = max(2, height * max(0, min(1, openness)))
        sy = y * 62 / height
        inner = (x / 87)**2 + (sy / 53)**2 <= 1
        if openness > .04:
            if emotion == "happy":
                arc = (x / 80)**2 + (sy / 29)**2 <= 1
                cut = (x / 74)**2 + ((sy - 13) / 28)**2 <= 1
                result[arc & ~cut] = YELLOW
            elif emotion == "love":
                lobes = (((x + 27) / 27)**2 + ((sy + 10) / 33)**2 <= 1) | (((x - 27) / 27)**2 + ((sy + 10) / 33)**2 <= 1)
                tip = (np.abs(x) <= 55 * (43 - sy) / 53) & (sy >= -10) & (sy <= 43)
                result[inner & ((lobes & (sy < -10)) | tip)] = YELLOW
            elif emotion == "money":
                gy = np.floor((sy + 45) / 10).astype(int)
                gx = np.floor((x + 30) / 12).astype(int)
                mask = np.zeros((n, n), dtype=bool)
                for row, bits in enumerate(MONEY):
                    for col in range(5):
                        if bits & (1 << (4-col)):
                            mask |= (gy == row) & (gx == col)
                result[mask] = YELLOW
            else:
                visible = inner.copy()
                if emotion == "angry":
                    visible &= x <= 4 * (sy + 14) if eye == 0 else x >= -4 * (sy + 14)
                direction = 1 if eye == 0 else -1
                px, py = direction * 14 + gaze[0], -20 + gaze[1]
                pw, ph = 75, 59
                if emotion == "surprised":
                    px, py, pw, ph = gaze[0], gaze[1]-3, 57, 45
                pupil = ((x-px)/pw)**2 + ((sy-py)/ph)**2 <= 1
                result[visible & ~pupil] = YELLOW
                shine = ((x-(px-direction*12))/10)**2 + ((sy-(py-8))/12)**2 <= 1
                result[visible & pupil & shine] = WHITE
    return Image.fromarray(result).resize((240, 240), Image.Resampling.NEAREST)

class Animator:
    def __init__(self, seed=None):
        self.rng = random.Random(seed)
        self.emotion = "sleeping"
        self.target = "sleeping"
        self.blink_start = None
        self.next_blink = 0
        self.last_cue = 0.0
        self.next_gaze = 0
        self.gaze_start = 0
        self.gaze = np.zeros(2)
        self.from_gaze = self.gaze.copy()
        self.to_gaze = self.gaze.copy()

    def pose(self, snapshot, now):
        desired = "sleeping" if snapshot.phase == "sleeping" else snapshot.emotion
        if snapshot.phase in ("connecting", "thinking"):
            desired = "confused" if snapshot.phase == "connecting" else "explaining"
        if desired != self.target:
            self.target = desired
            self.blink_start = now
        if snapshot.cue != self.last_cue:      # "your turn": mic just reopened
            self.last_cue = snapshot.cue
            if self.blink_start is None and self.emotion != "sleeping":
                self.blink_start = now
        if self.blink_start is None and now >= self.next_blink and self.emotion != "sleeping":
            self.blink_start = now
        opening = 1.0
        if self.blink_start is not None:
            elapsed = now - self.blink_start
            if elapsed < .16: opening = 1 - elapsed / .16
            elif elapsed < .24: opening = 0
            elif elapsed < .48: opening = (elapsed-.24)/.24
            else:
                self.blink_start = None
                self.next_blink = now + self.rng.uniform(2.8, 5)
            if elapsed >= .16: self.emotion = self.target
            opening = opening * opening * (3 - 2 * opening)
        if now >= self.next_gaze and self.blink_start is None:
            self.from_gaze = self.gaze.copy()
            self.to_gaze = np.array([self.rng.uniform(-9, 9), self.rng.uniform(-5, 5)])
            if snapshot.phase == "listening": self.to_gaze *= .25
            self.gaze_start = now
            self.next_gaze = now + self.rng.uniform(1.5, 3.5)
        p = min(1, max(0, (now - self.gaze_start)/.45))
        self.gaze = self.from_gaze + (self.to_gaze-self.from_gaze) * p*p*(3-2*p)
        return self.emotion, opening, tuple(self.gaze), snapshot.level

class EyeLoop:
    def __init__(self, state, sink, fps=25, pixel_size=4):
        self.state, self.sink = state, sink
        self.period = 1 / fps
        self.pixel_size = pixel_size
        self.stop_event = threading.Event()
        self.error = None
        self.thread = threading.Thread(target=self._run, name="owl-eyes", daemon=True)

    def start(self): self.thread.start()

    def _run(self):
        animator = Animator()
        try:
            while not self.stop_event.is_set():
                start = time.monotonic()
                pose = animator.pose(self.state.snapshot(), start)
                frames = [render_eye(i, *pose, pixel_size=self.pixel_size) for i in range(2)]
                self.sink.show(frames)
                self.stop_event.wait(max(0, self.period-(time.monotonic()-start)))
        except Exception as exc:
            self.error = exc

    def close(self):
        self.stop_event.set()
        self.thread.join(timeout=5)
        self.sink.close()
