#!/usr/bin/env python3
"""Run on Raspberry Pi OS 64-bit with Python 3.11+.

Examples:
  python main.py --list-audio
  python main.py --demo
  python main.py --contact-sheet eyes.png
  python main.py --download-wake-model
  python main.py
"""
import argparse
import asyncio
import json
import logging
import os
from pathlib import Path
import re
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parent

def load_config(path):
    with open(path, "rb") as handle: cfg = tomllib.load(handle)
    if cfg["assistant"]["sleep_after_seconds"] <= 0:
        raise ValueError("sleep_after_seconds must be positive")
    if not cfg["assistant"]["wake_phrases"]:
        raise ValueError("At least one wake phrase is required")
    if not 1 <= cfg["display"]["fps"] <= 60:
        raise ValueError("display fps must be between 1 and 60")
    if cfg["display"]["pixel_size"] not in (1, 2, 3, 4, 5, 6, 8):
        raise ValueError("pixel_size must be 1, 2, 3, 4, 5, 6 or 8")
    if not 0 < cfg["audio"]["volume"] <= 1:
        raise ValueError("audio volume must be between 0 and 1")
    if cfg["audio"]["output_channels"] not in (1, 2):
        raise ValueError("Use one or two output channels")
    if not 0 <= cfg["audio"]["mic_channel"] < cfg["audio"]["input_channels"]:
        raise ValueError("mic_channel must exist in input_channels")
    pins = [cfg["display"][p] for p in ("left_dc_gpio", "right_dc_gpio", "left_reset_gpio", "right_reset_gpio")]
    if len(set(pins)) != 4 or any(p in (7, 8, 9, 10, 11) for p in pins):
        raise ValueError("DC/reset pins must be distinct and not use the SPI0 pins")
    if cfg["display"]["left_ce"] == cfg["display"]["right_ce"]:
        raise ValueError("Screens need different chip-select lines")
    for name in ("left_rotation", "right_rotation"):
        if cfg["display"][name] not in (0, 90, 180, 270):
            raise ValueError("Display rotation must be 0, 90, 180 or 270")
    model = Path(cfg["wake"]["model_path"])
    cfg["wake"]["model_path"] = str(model if model.is_absolute() else ROOT / model)
    return cfg

def phrase_matches(text, phrases):
    words = " " + " ".join(re.findall(r"[\w']+", text.casefold())) + " "
    return any(" " + " ".join(re.findall(r"[\w']+", p.casefold())) + " " in words for p in phrases)

def download_model(cfg):
    from urllib.request import urlretrieve
    import tempfile
    import zipfile
    destination = Path(cfg["wake"]["model_path"])
    expected = "vosk-model-small-en-us-0.15"
    if destination.name != expected:
        raise ValueError("This helper downloads only the default English model; see README for other languages")
    if destination.exists():
        print("Wake model already exists:", destination)
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    print("Downloading the official Vosk small English model (about 40 MB)...")
    with tempfile.TemporaryDirectory() as temp:
        archive = Path(temp) / "model.zip"
        urlretrieve("https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip", archive)
        with zipfile.ZipFile(archive) as package:
            root = destination.parent.resolve()
            for member in package.infolist():
                target = (root / member.filename).resolve()
                if not target.is_relative_to(root) or member.filename.split('/')[0] != expected:
                    raise ValueError("Unexpected model archive path")
                if ((member.external_attr >> 16) & 0o170000) == 0o120000:
                    raise ValueError("Model archive must not contain symbolic links")
            package.extractall(root)
    print("Installed wake model:", destination)

def contact_sheet(path, pixel_size):
    from PIL import Image, ImageDraw
    from eyes import render_eye
    emotions = ["normal", "happy", "sleeping", "sleepy", "surprised", "angry", "confused", "love", "money", "explaining"]
    sheet = Image.new("RGB", (1000, 5*285), (20, 22, 28))
    draw = ImageDraw.Draw(sheet)
    for k, emotion in enumerate(emotions):
        x, y = (k % 2)*500, (k // 2)*285
        draw.text((x+15, y+8), emotion.upper(), fill="white")
        for eye in range(2):
            sheet.paste(render_eye(eye, emotion, pixel_size=pixel_size), (x+8+eye*242, y+34))
    sheet.save(path)
    print("Saved expression preview:", path)

async def run_voice(cfg, state, audio, eyes):
    import soxr
    from vosk import Model, KaldiRecognizer, SetLogLevel
    from realtime_voice import VoiceSession
    SetLogLevel(-1)
    model_path = Path(cfg["wake"]["model_path"])
    if not model_path.is_dir():
        raise RuntimeError("Wake model missing. Run: python main.py --download-wake-model")
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
            if eyes.error: raise RuntimeError("Eye display stopped") from eyes.error
            item = await asyncio.to_thread(audio.read)
            if item is None: continue
            captured_at, samples = item
            if time.monotonic()-captured_at > .5 or audio.playing(): continue
            pcm = resampler.resample_chunk(samples)
            if not pcm.size: continue
            final = recognizer.AcceptWaveform(pcm.astype("<i2").tobytes())
            result = json.loads(recognizer.Result() if final else recognizer.PartialResult())
            text = result.get("text", result.get("partial", ""))
            if phrase_matches(text, phrases): break
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
    parser.add_argument("--config", default=str(ROOT / "config.toml"))
    parser.add_argument("--list-audio", action="store_true")
    parser.add_argument("--download-wake-model", action="store_true")
    parser.add_argument("--contact-sheet", metavar="PNG")
    parser.add_argument("--demo", action="store_true", help="Eye-only demo: no microphone, camera, network or API key")
    parser.add_argument("--no-display", action="store_true", help="Test voice without GPIO/screens")
    parser.add_argument("--audio-test", action="store_true", help="Record three seconds locally, then play it back; no cloud")
    args = parser.parse_args()
    cfg = load_config(args.config)
    if args.list_audio:
        import sounddevice as sd
        print(sd.query_devices())
        print("Default input/output IDs:", sd.default.device)
        return
    if args.download_wake_model:
        download_model(cfg); return
    if args.contact_sheet:
        contact_sheet(args.contact_sheet, cfg["display"]["pixel_size"]); return
    if args.audio_test:
        import sounddevice as sd
        import soxr
        import numpy as np
        from audio_io import device_value
        a = cfg["audio"]
        print("Recording three seconds locally. Say something now...")
        recording = sd.rec(round(a["input_rate"]*3), samplerate=a["input_rate"], channels=a["input_channels"],
                           dtype="int16", device=device_value(a["input_device"]))
        sd.wait()
        mono = recording[:, a["mic_channel"]]
        playback = soxr.resample(mono, a["input_rate"], a["output_rate"])
        playback = (playback.astype(np.float32)*a["volume"]).astype("int16")
        playback = np.repeat(playback[:, None], a["output_channels"], axis=1)
        print("Playing through the configured speaker...")
        sd.play(playback, samplerate=a["output_rate"], device=device_value(a["output_device"]))
        sd.wait(); return
    if not args.demo:
        from dotenv import load_dotenv
        load_dotenv(ROOT / ".env")
        if not cfg["assistant"]["allow_cloud_audio"]:
            raise RuntimeError("Online conversation is disabled. Eye-only mode: --demo")
        if not os.environ.get("OPENAI_API_KEY", "").strip():
            raise RuntimeError("Copy .env.example to .env and add your API key locally, or use --demo")
        if not Path(cfg["wake"]["model_path"]).is_dir():
            raise RuntimeError("Run python main.py --download-wake-model first")
    from state import OwlState, EMOTIONS
    from eyes import EyeLoop
    from display import PiDisplays, NullDisplays
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
                    if eyes.error: raise RuntimeError("Eye display stopped") from eyes.error
                    if emotion == "sleeping": state.sleep()
                    else:
                        state.wake(); state.phase("idle"); state.emotion(emotion)
                    print(emotion)
                    time.sleep(4)
        else:
            from audio_io import AudioIO
            audio = AudioIO(cfg["audio"])
            audio.start()
            asyncio.run(run_voice(cfg, state, audio, eyes))
    finally:
        if audio: audio.close()
        eyes.close()

if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")
    try: main()
    except KeyboardInterrupt: print("\nOwl stopped.")
    except Exception as exc:
        logging.error("%s", exc)
        sys.exit(1)
