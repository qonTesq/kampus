#!/usr/bin/env python3
"""Voice echo test: speak to the mic, hear it back 3 s after you stop. No cloud.

Checks the whole local turn-taking path the owl relies on: detect speech start,
detect the end of speech, wait, then play on the speaker. The eyes show the
phase: normal = waiting, surprised = hearing you, explaining = waiting to reply,
happy = playing back.

  python echo_test.py                  # 5 turns, 3 s reply delay
  python echo_test.py --turns 3 --delay 3 --no-display
"""
import argparse
import time
import tomllib

import numpy as np
import soxr

from audio_io import AudioIO
from main import ROOT
from state import OwlState
from turns import calibrate, listen, rms

TARGET_RMS = 6000     # replay loudness (int16 RMS) before the config volume


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--turns", type=int, default=5)
    parser.add_argument("--delay", type=float, default=3.0,
                        help="seconds from end of speech to start of playback")
    parser.add_argument("--bt-latency", type=float, default=0.25,
                        help="estimated Bluetooth speaker delay, subtracted from --delay")
    parser.add_argument("--no-display", action="store_true")
    args = parser.parse_args()
    with open(ROOT / "config.toml", "rb") as f:
        cfg = tomllib.load(f)
    rate = cfg["audio"]["input_rate"]

    state = OwlState(timeout=10**6)
    state.wake()
    state.phase("idle")
    eyes = None
    if not args.no_display:
        from display import PiDisplays
        from eyes import EyeLoop
        eyes = EyeLoop(state, PiDisplays(cfg["display"]), cfg["display"]["fps"],
                       cfg["display"]["pixel_size"])
        eyes.start()
    state.emotion("normal")
    audio = AudioIO(cfg["audio"])
    audio.start()

    try:
        print("Calibrating background noise, stay quiet for 2 s...")
        floor, threshold = calibrate(audio)
        print(f"Noise floor {floor:.0f}, speech threshold {threshold:.0f}")

        def heard():
            state.emotion("surprised")
            print("  hearing you...", flush=True)

        for turn in range(1, args.turns + 1):
            print(f"\n[{turn}/{args.turns}] Speak now...", flush=True)
            state.emotion("normal")
            said = listen(audio, rate, cfg["audio"]["block_ms"], threshold, on_start=heard)
            started, speech_end = said.started, said.ended
            spoken = said.samples.astype(np.float32)
            # Level the replay: quiet or distant speech is boosted (up to 4x)
            # without clipping, so it plays at a steady volume.
            gain = min(4.0, TARGET_RMS / max(rms(spoken), 1.0),
                       30000 / max(float(np.abs(spoken).max()), 1.0))
            spoken = (spoken * gain).astype(np.int16)
            state.emotion("explaining")
            print(f"  heard {speech_end - started:.1f} s of speech; replying in {args.delay:g} s", flush=True)
            reply = soxr.resample(spoken, rate, 24000).astype("<i2").tobytes()
            wait = speech_end + args.delay - args.bt_latency - time.monotonic()
            if wait > 0:
                time.sleep(wait)
            queued = time.monotonic() - speech_end
            state.emotion("happy")
            audio.feed_pcm24(reply)
            print(f"  playback queued {queued:.2f} s after you stopped"
                  f" (~{queued + args.bt_latency:.2f} s at the speaker)", flush=True)
            while audio.playing():
                time.sleep(0.05)
    finally:
        audio.close()
        if eyes:
            eyes.close()
    print("\nDone. Pass if each reply started about 3 s after you stopped and sounded clear.")


if __name__ == "__main__":
    main()
