"""Phase 5 soak test: eyes + mic + speaker running together, no cloud.

The eyes cycle expressions and turn "surprised" while the mic hears a loud
sound; a short chime plays on the speaker every 20 s. Every 30 s it prints
eye fps, mic level, audio glitch counters, CPU temperature and throttling.

  owl-soak-test                    # 10 minutes
  owl-soak-test --minutes 2
"""
import argparse
import subprocess
import time
import tomllib

import numpy as np

from owl.config import default_config
from owl.hardware.audio_io import AudioIO
from owl.hardware.display import PiDisplays
from owl.hardware.eyes import EyeLoop
from owl.state import OwlState

EXPRESSIONS = ("normal", "happy", "explaining", "love", "sleepy", "confused")
LOUD = 3000          # int16 RMS that counts as "the mic heard something"


def vcgencmd(*args):
    out = subprocess.run(["vcgencmd", *args], check=False, capture_output=True, text=True).stdout
    return out.strip().split("=", 1)[-1]


def chime():
    t = np.arange(int(24000 * 0.4)) / 24000
    tone = np.sin(2 * np.pi * 880 * t) * np.minimum(1, (0.4 - t) * 10) * 8000
    return tone.astype("<i2").tobytes()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--minutes", type=float, default=10)
    args = parser.parse_args()
    with open(default_config(), "rb") as f:
        cfg = tomllib.load(f)

    state = OwlState(timeout=10**6)
    state.wake()
    state.phase("idle")
    sink = PiDisplays(cfg["display"])
    frames = [0]
    show = sink.show
    sink.show = lambda f: (frames.__setitem__(0, frames[0] + 1), show(f))
    eyes = EyeLoop(state, sink, cfg["display"]["fps"], cfg["display"]["pixel_size"])
    audio = AudioIO(cfg["audio"])
    eyes.start()
    audio.start()

    start = last_report = last_chime = last_expr = time.monotonic()
    expr = 0
    peak = 0.0
    loud_events = 0
    worst_temp = 0.0
    print(f"Soak test for {args.minutes:g} min. Talk or clap near the webcam now and then.")
    try:
        while time.monotonic() - start < args.minutes * 60:
            if eyes.error:
                raise RuntimeError("Eye loop crashed") from eyes.error
            now = time.monotonic()
            item = audio.read(timeout=0.1)
            if item is not None:
                rms = float(np.sqrt(np.mean(item[1].astype(np.float32) ** 2)))
                peak = max(peak, rms)
                if rms > LOUD and not audio.playing():
                    loud_events += 1
                    state.emotion("surprised")
                    last_expr = now
            if now - last_expr >= 4:
                expr = (expr + 1) % len(EXPRESSIONS)
                state.emotion(EXPRESSIONS[expr])
                last_expr = now
            if now - last_chime >= 20:
                audio.feed_pcm24(chime())
                last_chime = now
            if now - last_report >= 30:
                temp = float(vcgencmd("measure_temp").rstrip("'C"))
                worst_temp = max(worst_temp, temp)
                print(f"{(now - start) / 60:4.1f} min | eyes {frames[0] / (now - last_report):4.1f} fps"
                      f" | mic peak {peak:6.0f} | loud {loud_events:3d}"
                      f" | mic drops {audio.input_drops} in-glitch {audio.input_status_count}"
                      f" out-glitch {audio.output_status_count}"
                      f" | {temp:.1f}C throttled={vcgencmd('get_throttled')}", flush=True)
                frames[0] = 0
                peak = 0.0
                last_report = now
    finally:
        audio.close()
        eyes.close()
    print(f"PASS criteria: eyes ~{cfg['display']['fps']} fps, mic drops 0, throttled=0x0."
          f" Worst temp {worst_temp:.1f}C.")


if __name__ == "__main__":
    main()
