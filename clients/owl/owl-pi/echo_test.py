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

SILENCE_END = 0.8     # seconds of quiet that count as "done speaking"
MAX_UTTERANCE = 10    # seconds; longer speech is cut off here


def rms(samples):
    return float(np.sqrt(np.mean(samples.astype(np.float32) ** 2)))


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
        noise = []
        end = time.monotonic() + 2
        while time.monotonic() < end:
            item = audio.read()
            if item is not None:
                noise.append(rms(item[1]))
        floor = float(np.median(noise)) if noise else 200.0
        threshold = max(1200.0, floor * 4)
        print(f"Noise floor {floor:.0f}, speech threshold {threshold:.0f}")

        for turn in range(1, args.turns + 1):
            print(f"\n[{turn}/{args.turns}] Speak now...", flush=True)
            state.emotion("normal")
            audio.drain_input()
            chunks, started, last_voice = [], None, None
            while True:
                item = audio.read()
                if item is None:
                    continue
                captured_at, samples = item
                loud = rms(samples) > threshold
                if started is None:
                    if not loud:
                        chunks = chunks[-15:] + [samples]   # keep 300 ms pre-roll
                        continue
                    started = captured_at
                    state.emotion("surprised")
                    print("  hearing you...", flush=True)
                chunks.append(samples)
                if loud:
                    last_voice = captured_at + len(samples) / rate
                if captured_at - last_voice >= SILENCE_END or captured_at - started > MAX_UTTERANCE:
                    break

            speech_end = last_voice
            spoken = np.concatenate(chunks)[: int((speech_end - started + 0.3) * rate)]
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
