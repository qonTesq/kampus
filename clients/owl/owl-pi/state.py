"""Small, independently testable conversation state machine."""
from dataclasses import dataclass, replace
from threading import Lock
import time

EMOTIONS = ("normal", "happy", "sleepy", "surprised", "angry", "confused", "love", "money", "explaining")

@dataclass
class Snapshot:
    phase: str = "sleeping"
    emotion: str = "normal"
    last_human: float = 0.0
    user_speaking: bool = False
    level: float = 0.0

class OwlState:
    def __init__(self, timeout=300, clock=time.monotonic):
        self.clock = clock
        self.timeout = timeout
        self.lock = Lock()
        self.data = Snapshot(last_human=clock())

    def snapshot(self):
        with self.lock:
            return replace(self.data)

    def wake(self):
        with self.lock:
            self.data = Snapshot(phase="connecting", last_human=self.clock())

    def sleep(self):
        with self.lock:
            self.data = Snapshot(phase="sleeping", last_human=self.clock())

    def phase(self, value):
        with self.lock:
            self.data.phase = value

    def speech(self, active):
        with self.lock:
            self.data.last_human = self.clock()
            self.data.user_speaking = active
            self.data.phase = "listening" if active else "thinking"
            if active:
                self.data.emotion = "normal"

    def emotion(self, value):
        if value not in EMOTIONS:
            raise ValueError("Unsupported expression")
        with self.lock:
            self.data.emotion = value

    def level(self, value):
        with self.lock:
            self.data.level = max(0.0, min(1.0, float(value)))

    def should_sleep(self, busy=False):
        s = self.snapshot()
        return (s.phase != "sleeping" and not busy and not s.user_speaking
                and self.clock() - s.last_human >= self.timeout)
