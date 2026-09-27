#!/usr/bin/env python3
"""Talking owl with a ChatGPT brain and an ElevenLabs voice.

One conversation turn:
  webcam mic -> local turn detection (turns.py) -> OpenAI speech-to-text
  -> ChatGPT reply + eye expression -> ElevenLabs voice -> JBL speaker

The owl sleeps (closed eyes, nothing sent to the cloud) until it hears
"Hey Owl" / "Hello Owl" locally, and goes back to sleep after
assistant.sleep_after_seconds without speech or when you say goodbye.

Keys live only in .env on the Pi:
  OPENAI_API_KEY=...        ELEVENLABS_API_KEY=...        ELEVENLABS_VOICE_ID=...

  python voice_pipeline.py              # full owl
  python voice_pipeline.py --no-wake    # skip the wake phrase, start awake
  python voice_pipeline.py --text       # type instead of speaking (tests ChatGPT + voice)
  python voice_pipeline.py --no-display # without the eye screens
"""
import argparse
import io
import json
import os
import time
import wave

import numpy as np
import requests

from main import ROOT, load_config, phrase_matches
from state import EMOTIONS, OwlState
from turns import calibrate, listen

OPENAI = "https://api.openai.com/v1"
ELEVENLABS = "https://api.elevenlabs.io/v1"

REPLY_SCHEMA = {
    "type": "object",
    "properties": {
        "say": {"type": "string"},
        "emotion": {"type": "string", "enum": list(EMOTIONS)},
        "sleep": {"type": "boolean"},
    },
    "required": ["say", "emotion", "sleep"],
    "additionalProperties": False,
}


def instructions(cfg):
    a, p = cfg["assistant"], cfg["pipeline"]
    return f"""You are {a['name']}, a friendly owl-shaped robot assistant with expressive eyes.
Speak {a['language']} unless the user asks for another language. Your words are read aloud,
so answer in at most {p['max_reply_words']} words, in plain spoken sentences: no lists,
markdown, emoji or URLs. Expand only when asked. Do not claim to be human or to have real
feelings, and do not claim to see, search or do anything you cannot.
Pick the eye expression that fits your reply: explaining for neutral explanations; happy for
encouragement, thanks or good news; surprised for unexpected information; confused when you
need clarification (then ask ONE short question); love or money only for playful affection or
money topics; angry only as a playful character expression; normal or sleepy otherwise.
Set sleep to true only when the user says goodbye or asks you to sleep."""


class Brain:
    """OpenAI: speech-to-text plus the ChatGPT reply, with a short memory."""

    def __init__(self, cfg, key):
        self.cfg = cfg
        self.http = requests.Session()
        self.http.headers["Authorization"] = f"Bearer {key}"
        self.history = []

    def reset(self):
        self.history = []

    def transcribe(self, samples, rate):
        buf = io.BytesIO()
        with wave.open(buf, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(rate)
            w.writeframes(samples.astype("<i2").tobytes())
        r = self.http.post(f"{OPENAI}/audio/transcriptions", timeout=30,
                           files={"file": ("speech.wav", buf.getvalue(), "audio/wav")},
                           data={"model": self.cfg["pipeline"]["transcribe_model"]})
        r.raise_for_status()
        return r.json().get("text", "").strip()

    def reply(self, text):
        self.history.append({"role": "user", "content": text})
        r = self.http.post(f"{OPENAI}/responses", timeout=30, json={
            "model": self.cfg["pipeline"]["chat_model"],
            "instructions": instructions(self.cfg),
            "input": self.history,
            "max_output_tokens": self.cfg["assistant"]["max_response_tokens"],
            "text": {"format": {"type": "json_schema", "name": "owl_reply",
                                "schema": REPLY_SCHEMA, "strict": True}},
        })
        r.raise_for_status()
        out = "".join(c["text"] for item in r.json()["output"] if item["type"] == "message"
                      for c in item["content"] if c["type"] == "output_text")
        answer = json.loads(out)
        self.history.append({"role": "assistant", "content": answer["say"]})
        self.history = self.history[-2 * self.cfg["pipeline"]["history_turns"]:]
        return answer


class Voice:
    """ElevenLabs text-to-speech, streamed straight into the speaker."""

    def __init__(self, cfg, key, voice_id):
        self.cfg = cfg
        self.voice_id = voice_id
        self.http = requests.Session()
        self.http.headers["xi-api-key"] = key
        self.chars_used = 0

    def speak(self, text, audio):
        # pcm_24000 = raw 16-bit mono at 24 kHz, exactly what AudioIO.feed_pcm24 plays.
        r = self.http.post(f"{ELEVENLABS}/text-to-speech/{self.voice_id}/stream",
                           params={"output_format": "pcm_24000"}, stream=True, timeout=30,
                           json={"text": text, "model_id": self.cfg["pipeline"]["tts_model"]})
        r.raise_for_status()
        leftover = b""
        for chunk in r.iter_content(4800):
            chunk = leftover + chunk
            cut = len(chunk) - len(chunk) % 2
            leftover = chunk[cut:]
            if cut:
                audio.feed_pcm24(chunk[:cut])
        self.chars_used += len(text)


def heard_wake_phrase(model, samples, rate, phrases):
    """Run the offline Vosk model on one utterance and look for the wake phrase."""
    import soxr
    from vosk import KaldiRecognizer
    rec = KaldiRecognizer(model, 16000, json.dumps(phrases + ["[unk]"]))
    rec.AcceptWaveform(soxr.resample(samples, rate, 16000).astype("<i2").tobytes())
    return phrase_matches(json.loads(rec.FinalResult()).get("text", ""), phrases)


def wait_for_playback(audio):
    while audio.playing():
        time.sleep(0.05)


def main():
    parser = argparse.ArgumentParser(description="Owl with ChatGPT brain and ElevenLabs voice")
    parser.add_argument("--no-wake", action="store_true", help="start awake, no wake phrase")
    parser.add_argument("--text", action="store_true", help="type messages instead of speaking")
    parser.add_argument("--no-display", action="store_true", help="run without the eye screens")
    args = parser.parse_args()

    cfg = load_config(ROOT / "config.toml")
    from dotenv import load_dotenv
    load_dotenv(ROOT / ".env")
    openai_key = os.environ.get("OPENAI_API_KEY", "").strip()
    eleven_key = os.environ.get("ELEVENLABS_API_KEY", "").strip()
    voice_id = (os.environ.get("ELEVENLABS_VOICE_ID", "").strip()
                or cfg["pipeline"]["voice_id"].strip())
    missing = [n for n, v in (("OPENAI_API_KEY", openai_key), ("ELEVENLABS_API_KEY", eleven_key),
                              ("ELEVENLABS_VOICE_ID", voice_id)) if not v]
    if missing:
        raise SystemExit("Add to .env on the Pi: " + ", ".join(missing))

    from audio_io import AudioIO
    brain = Brain(cfg, openai_key)
    voice = Voice(cfg, eleven_key, voice_id)
    rate, block_ms = cfg["audio"]["input_rate"], cfg["audio"]["block_ms"]
    phrases = cfg["assistant"]["wake_phrases"]
    sleep_after = cfg["assistant"]["sleep_after_seconds"]

    state = OwlState(sleep_after)
    eyes = None
    if not args.no_display and cfg["display"]["enabled"]:
        from display import PiDisplays
        from eyes import EyeLoop
        eyes = EyeLoop(state, PiDisplays(cfg["display"]), cfg["display"]["fps"],
                       cfg["display"]["pixel_size"])
        eyes.start()
    audio = AudioIO(cfg["audio"])
    audio.start()

    wake_model = None
    if not (args.no_wake or args.text):
        from vosk import Model, SetLogLevel
        SetLogLevel(-1)
        wake_model = Model(cfg["wake"]["model_path"])

    def go_to_sleep():
        brain.reset()
        state.sleep()
        print("\nSleeping. Say " + " or ".join(repr(p) for p in phrases) + ".", flush=True)

    def wake_up():
        state.wake()
        state.phase("idle")
        state.emotion("happy")
        print("Awake! Talk to me (Ctrl+C to stop).", flush=True)
        voice.speak(cfg["pipeline"]["greeting"], audio)
        wait_for_playback(audio)

    try:
        threshold = 0
        if not args.text:
            print("Calibrating background noise, stay quiet for 2 s...", flush=True)
            floor, threshold = calibrate(audio)
            print(f"Noise floor {floor:.0f}, speech threshold {threshold:.0f}", flush=True)

        awake = args.no_wake or args.text
        if awake:
            wake_up()
        else:
            go_to_sleep()

        while True:
            if eyes and eyes.error:
                raise RuntimeError("Eye display stopped") from eyes.error

            if not awake:
                said = listen(audio, rate, block_ms, threshold)
                if heard_wake_phrase(wake_model, said.samples, rate, phrases):
                    awake = True
                    wake_up()
                continue

            if args.text:
                text = input("\nYou: ").strip()
                if not text:
                    continue
                state.speech(False)
            else:
                state.phase("idle")
                said = listen(audio, rate, block_ms, threshold, timeout=sleep_after,
                              on_start=lambda: state.speech(True))
                if said is None:            # nobody spoke for sleep_after seconds
                    awake = False
                    go_to_sleep()
                    continue
                state.speech(False)          # eyes: thinking
                started = time.monotonic()
                text = brain.transcribe(said.samples, rate)
                if not text:
                    continue
                print(f"\nYou: {text}  ({time.monotonic() - started:.1f} s to transcribe)")

            try:
                t0 = time.monotonic()
                answer = brain.reply(text)
                state.phase("idle")
                state.emotion(answer["emotion"])
                print(f"Owl [{answer['emotion']}]: {answer['say']}"
                      f"  ({time.monotonic() - t0:.1f} s to think)", flush=True)
                voice.speak(answer["say"], audio)
                wait_for_playback(audio)
                print(f"  ElevenLabs characters used this run: {voice.chars_used}", flush=True)
            except requests.RequestException as exc:
                detail = getattr(exc.response, "text", "")[:300] if exc.response is not None else ""
                print(f"  Cloud request failed: {exc} {detail}", flush=True)
                state.phase("idle")
                state.emotion("confused")
                continue

            if answer["sleep"] and not args.text:
                awake = False
                go_to_sleep()
    except (KeyboardInterrupt, EOFError):
        print("\nOwl stopped.")
    finally:
        audio.close()
        if eyes:
            eyes.close()
        print(f"ElevenLabs characters used this run: {voice.chars_used}")


if __name__ == "__main__":
    main()
