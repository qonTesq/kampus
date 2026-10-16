"""The OpenAI Realtime owl, plus the eye demo and audio/wake-model utilities.

Run on Raspberry Pi OS 64-bit with Python 3.11+.

Examples:
  owl-realtime --list-audio
  owl-realtime --demo
  owl-realtime --contact-sheet eyes.png
  owl-realtime --download-wake-model
  owl-realtime
"""

import argparse
import asyncio
import json
import logging
import os
import sys
import time
from pathlib import Path

from owl.brain.wake import download_model, phrase_matches
from owl.config import ENV_FILE, apply_env, default_config, load_config


def contact_sheet(path, pixel_size):
    from PIL import Image, ImageDraw

    from owl.hardware.eyes import render_eye

    emotions = [
        "normal",
        "happy",
        "sleeping",
        "sleepy",
        "surprised",
        "angry",
        "confused",
        "love",
        "money",
        "explaining",
    ]
    sheet = Image.new("RGB", (1000, 5 * 285), (20, 22, 28))
    draw = ImageDraw.Draw(sheet)
    for k, emotion in enumerate(emotions):
        x, y = (k % 2) * 500, (k // 2) * 285
        draw.text((x + 15, y + 8), emotion.upper(), fill="white")
        for eye in range(2):
            sheet.paste(render_eye(eye, emotion, pixel_size=pixel_size), (x + 8 + eye * 242, y + 34))
    sheet.save(path)
    print("Saved expression preview:", path)


async def run_voice(cfg, state, audio, eyes):
    import soxr
    from vosk import KaldiRecognizer, Model, SetLogLevel

    from owl.brain.realtime import VoiceSession

    SetLogLevel(-1)
    model_path = Path(cfg["wake"]["model_path"])
    if not model_path.is_dir():
        raise RuntimeError("Wake model missing. Run: owl-realtime --download-wake-model")
    model = Model(str(model_path))
    phrases = cfg["assistant"]["wake_phrases"]
    grammar = json.dumps(phrases + ["[unk]"])
    print("Sleeping. Say " + " or ".join(repr(p) for p in phrases) + ". Ctrl+C stops the owl.")
    while True:
        state.sleep()
        recognizer = KaldiRecognizer(model, 16000, grammar)
        resampler = soxr.ResampleStream(cfg["audio"]["input_rate"], 16000, 1, dtype="int16", quality="HQ")
        audio.drain_input()
        while True:
            if eyes.error:
                raise RuntimeError("Eye display stopped") from eyes.error
            item = await asyncio.to_thread(audio.read)
            if item is None:
                continue
            captured_at, samples = item
            if time.monotonic() - captured_at > 0.5 or audio.playing():
                continue
            pcm = resampler.resample_chunk(samples)
            if not pcm.size:
                continue
            final = recognizer.AcceptWaveform(pcm.astype("<i2").tobytes())
            result = json.loads(recognizer.Result() if final else recognizer.PartialResult())
            text = result.get("text", result.get("partial", ""))
            if phrase_matches(text, phrases):
                break
        state.wake()
        print("Wake phrase heard. Connecting to voice service...")
        try:
            await VoiceSession(cfg, state, audio, eyes).run()
            print("Conversation ended. Listening locally for the wake phrase.")
        except asyncio.CancelledError:
            raise
        except Exception as exc:
            logging.error("Voice session stopped: %s", exc)
            state.emotion("confused")
            state.phase("idle")
            # Do not repeatedly reconnect or spend API credit on a failure.
            await asyncio.sleep(3)
            print("Say the wake phrase to retry after fixing the problem.")


def main():
    parser = argparse.ArgumentParser(description="Talking owl for Raspberry Pi 5")
    parser.add_argument("--config", default=str(default_config()))
    parser.add_argument("--list-audio", action="store_true")
    parser.add_argument("--download-wake-model", action="store_true")
    parser.add_argument("--contact-sheet", metavar="PNG")
    parser.add_argument(
        "--demo", action="store_true", help="Eye-only demo: no microphone, camera, network or API key"
    )
    parser.add_argument("--no-display", action="store_true", help="Test voice without GPIO/screens")
    parser.add_argument(
        "--audio-test", action="store_true", help="Record three seconds locally, then play it back; no cloud"
    )
    args = parser.parse_args()
    cfg = load_config(args.config)
    from dotenv import load_dotenv

    load_dotenv(ENV_FILE)
    apply_env(cfg)
    if args.list_audio:
        import sounddevice as sd

        print(sd.query_devices())
        print("Default input/output IDs:", sd.default.device)
        return
    if args.download_wake_model:
        download_model(cfg)
        return
    if args.contact_sheet:
        contact_sheet(args.contact_sheet, cfg["display"]["pixel_size"])
        return
    if args.audio_test:
        import numpy as np
        import sounddevice as sd
        import soxr

        from owl.hardware.audio_io import device_value

        a = cfg["audio"]
        print("Recording three seconds locally. Say something now...")
        recording = sd.rec(
            round(a["input_rate"] * 3),
            samplerate=a["input_rate"],
            channels=a["input_channels"],
            dtype="int16",
            device=device_value(a["input_device"]),
        )
        sd.wait()
        mono = recording[:, a["mic_channel"]]
        playback = soxr.resample(mono, a["input_rate"], a["output_rate"])
        playback = (playback.astype(np.float32) * a["volume"]).astype("int16")
        playback = np.repeat(playback[:, None], a["output_channels"], axis=1)
        print("Playing through the configured speaker...")
        sd.play(playback, samplerate=a["output_rate"], device=device_value(a["output_device"]))
        sd.wait()
        return
    if not args.demo:
        if not cfg["assistant"]["allow_cloud_audio"]:
            raise RuntimeError("Online conversation is disabled. Eye-only mode: --demo")
        if not os.environ.get("OPENAI_API_KEY", "").strip():
            raise RuntimeError("Copy .env.example to .env and add your API key locally, or use --demo")
        if not Path(cfg["wake"]["model_path"]).is_dir():
            raise RuntimeError("Run owl-realtime --download-wake-model first")
    from owl.hardware.display import NullDisplays, PiDisplays
    from owl.hardware.eyes import EyeLoop
    from owl.state import EMOTIONS, OwlState

    state = OwlState(cfg["assistant"]["sleep_after_seconds"])
    sink = NullDisplays() if args.no_display or not cfg["display"]["enabled"] else PiDisplays(cfg["display"])
    eyes = EyeLoop(state, sink, cfg["display"]["fps"], cfg["display"]["pixel_size"])
    audio = None
    eyes.start()
    try:
        if args.demo:
            print("Offline eye demo. Ctrl+C stops.")
            while True:
                for emotion in ("sleeping",) + EMOTIONS:
                    if eyes.error:
                        raise RuntimeError("Eye display stopped") from eyes.error
                    if emotion == "sleeping":
                        state.sleep()
                    else:
                        state.wake()
                        state.phase("idle")
                        state.emotion(emotion)
                    print(emotion)
                    time.sleep(4)
        else:
            from owl.hardware.audio_io import AudioIO

            audio = AudioIO(cfg["audio"])
            audio.start()
            asyncio.run(run_voice(cfg, state, audio, eyes))
    finally:
        if audio:
            audio.close()
        eyes.close()


def run():
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")
    try:
        main()
    except KeyboardInterrupt:
        print("\nOwl stopped.")
    except Exception as exc:
        logging.error("%s", exc)
        sys.exit(1)


if __name__ == "__main__":
    run()
