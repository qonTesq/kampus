"""Local turn detection: find where someone starts and stops speaking.

Shared by hardware/echo_test.py and brain/pipeline.py. Energy-based (no cloud):
speech starts once MIN_SPEECH of loud audio lands within ONSET_WINDOW, so taps,
clicks and echo tails are ignored; it ends after SILENCE_END of quiet.
"""

import time
from dataclasses import dataclass

import numpy as np

SILENCE_END = 1.2  # seconds of quiet that count as "done speaking" (natural pauses are ~1 s)
MAX_UTTERANCE = 10  # seconds; longer speech is cut off here
MIN_SPEECH = 0.3  # seconds of loud audio needed before it counts as speech
ONSET_WINDOW = 0.5  # seconds the MIN_SPEECH has to fall within
MAX_THRESHOLD = 4500  # a noisy calibration (fan at boot, people talking) can't make it deaf


def rms(samples):
    return float(np.sqrt(np.mean(samples.astype(np.float32) ** 2)))


@dataclass
class Utterance:
    samples: np.ndarray  # int16 mono at the input rate, 0.3 s pre-roll, 0.2 s tail
    started: float  # time.monotonic() when speech began
    ended: float  # time.monotonic() of the last loud audio


def calibrate(audio, seconds=2.0):
    """Measure the room's background noise; return a speech threshold."""
    noise = []
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        item = audio.read()
        if item is not None:
            noise.append(rms(item[1]))
    floor = float(np.median(noise)) if noise else 200.0
    return floor, min(MAX_THRESHOLD, max(1200.0, floor * 4))


def listen(audio, rate, block_ms, threshold, on_start=None, timeout=None, on_audio=None):
    """Block until one utterance is heard; return it, or None after timeout
    seconds without speech. Mic audio is ignored while the speaker plays.

    on_audio(samples), if given, receives the speech as it is recorded (pre-roll
    first), so a cloud brain can be listening before the user has finished."""
    onset_chunks = max(1, round(ONSET_WINDOW * 1000 / block_ms))
    deadline = None if timeout is None else time.monotonic() + timeout
    audio.drain_input()
    chunks, recent, started, last_voice = [], [], None, None
    while True:
        if started is None and deadline is not None and time.monotonic() > deadline:
            return None
        item = audio.read()
        if item is None:
            continue
        captured_at, samples = item
        if started is None and audio.playing():
            chunks, recent = [], []
            continue
        loud = rms(samples) > threshold
        if started is None:
            chunks = chunks[-(onset_chunks + 15) :] + [samples]  # + 300 ms pre-roll
            recent = recent[-(onset_chunks - 1) :] + [(captured_at, loud)]
            if sum(len(samples) for _, l in recent if l) / rate < MIN_SPEECH:
                continue
            started = next(t for t, l in recent if l)
            last_voice = captured_at + len(samples) / rate
            if on_start:
                on_start()
            if on_audio:
                for chunk in chunks:
                    on_audio(chunk)
            continue
        chunks.append(samples)
        if on_audio:
            on_audio(samples)
        if loud:
            last_voice = captured_at + len(samples) / rate
        if captured_at - last_voice >= SILENCE_END or captured_at - started > MAX_UTTERANCE:
            break

    # Keep 0.3 s before the onset and 0.2 s after the last loud chunk.
    audio_end = captured_at + len(samples) / rate
    recorded = np.concatenate(chunks)
    first = max(0, len(recorded) - int((audio_end - started + 0.3) * rate))
    last = len(recorded) - max(0, int((audio_end - last_voice - 0.2) * rate))
    return Utterance(recorded[first:last], started, last_voice)
