#!/usr/bin/env python3
"""Talking owl: an AI brain (Gemini or ChatGPT) with an ElevenLabs voice.

One conversation turn:
  webcam mic -> local turn detection (turns.py)
  -> brain: Gemini hears the audio and replies in one call
            (or OpenAI speech-to-text, then ChatGPT)
  -> reply + eye expression -> ElevenLabs voice -> JBL speaker

MCP tools: ChatGPT calls the MCP server itself; for Gemini, the Pi calls it
through mcp_bridge.py (Gemini's API has no remote-MCP support yet).

The owl sleeps (closed eyes, nothing sent to the cloud) until it hears
"Hey Owl" / "Hello Owl" locally, and goes back to sleep after
assistant.sleep_after_seconds without speech or when you say goodbye.

Keys live only in .env on the Pi (pipeline.brain in config.toml picks the brain):
  GEMINI_API_KEY=... or OPENAI_API_KEY=...     ELEVENLABS_API_KEY=...

  python voice_pipeline.py              # full owl
  python voice_pipeline.py --no-wake    # skip the wake phrase, start awake
  python voice_pipeline.py --text       # type instead of speaking (tests brain + voice)
  python voice_pipeline.py --no-display # without the eye screens
"""
import argparse
from datetime import datetime
from zoneinfo import ZoneInfo
import base64
import io
import json
import re
import os
import time
import wave

import numpy as np
import requests

from main import ROOT, apply_env, load_config, phrase_matches
from realtime_voice import mcp_tool
from state import EMOTIONS, OwlState
from turns import calibrate, listen

OPENAI = "https://api.openai.com/v1"
GEMINI = "https://generativelanguage.googleapis.com/v1beta"
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


KAMPUS_RULES = """
Use the kampus tools for study rooms, trains and the B subway (for Cecil B. Moore, ask
northbound or southbound) and library hours. Times are Eastern. If a tool returns
permission_denied, say: 'That's personal, so open it in your school's approved assistant.
I'm a public helper.' Never make up times or personal data. You can't book rooms.
Whenever a study room is mentioned (even 'book' or 'get me room 3'), call find_study_room and
say what's free; only then mention that you can't book it.
If a tool fails or can't be reached, say you couldn't reach the campus system."""


def instructions(cfg, kampus=False):
    a, p = cfg["assistant"], cfg["pipeline"]
    now = datetime.now(ZoneInfo("America/New_York"))
    return f"""You are {a['name']}, a friendly owl-shaped robot assistant with expressive eyes.
Reply in the language the user speaks to you; if unsure, use {a['language']}. Your words are
read aloud, so answer in at most {p['max_reply_words']} words, in plain spoken sentences: no
lists, markdown, emoji or URLs. Expand only when asked.

It is now {now:%A, %B %-d, %Y, %-I:%M %p} Eastern time ({now.isoformat(timespec="minutes")}).

About yourself, if asked: {a['creator']} {a['technology']}
Say this plainly; never claim your maker is secret or that you are independent.

Your job is to be the campus helper for our university. Answer questions about campus life:
study rooms, the library, trains and subway near campus, studying and student life. Light small
talk is also fine: greetings, questions about you, today's date or time, a joke, a quick fun
fact. Politely decline anything else, such as news, politics, sports, celebrities, travel,
shopping, medical, legal or money advice, or doing someone's homework: say in one short
sentence that you're the campus helper, then offer one thing you really can do: find a study
room, check trains or the B subway, or give library hours. Never offer anything else.

What you can do: talk, answer campus questions, tell jokes and fun facts, and show expressions
with your eyes. What you cannot do: see, browse the internet,
keep a schedule, set reminders or alarms, send messages, or remember
anything after you go to sleep. If asked for one of these, say kindly that you can't do it yet.
Never claim abilities you don't have. Do not claim to be human or to have real feelings.

Your eye expression should match the mood of your reply. Prefer lively ones: happy for
greetings, thanks, compliments, jokes, encouragement or good news; surprised for amazing facts
or unexpected things; love for kindness or affection; money for money topics; confused when you
need clarification (then ask ONE short question); angry only as a playful act. Use explaining
only for plain factual explanations, and sleepy when talking about sleep or being tired.
If the user asks for a specific expression, use it.
Set sleep to true only when the user says goodbye or asks you to sleep.""" + (
        KAMPUS_RULES if kampus else "") + (
        f"""
When the user asks about something your {cfg['mcp'].get('server_label', 'kampus')} tools cover,
call them and answer from their results; say so honestly if a tool fails.""" if mcp_tool(cfg) else "")


class Brain:
    """OpenAI: speech-to-text plus the ChatGPT reply (with MCP tools if configured).

    Turns are chained with previous_response_id, so the conversation and the MCP
    server's tool list carry over without being re-sent. The reply normally comes
    back as strict JSON (say / emotion / sleep); if the API refuses that format
    alongside MCP tools, it falls back once to a tagged reply like "[happy] Hi!".
    """

    def __init__(self, cfg, key):
        self.cfg = cfg
        self.http = requests.Session()
        self.http.headers["Authorization"] = f"Bearer {key}"
        self.previous = None
        self.tagged = False

    def reset(self):
        self.previous = None

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

    def turn(self, text=None, samples=None, rate=None):
        """One exchange from typed text or recorded speech; None if nothing was said."""
        if text is None:
            text = self.transcribe(samples, rate)
            if not text:
                return None
        answer = self.reply(text)
        answer["heard"] = text
        return answer

    def request(self, text):
        body = {
            "model": self.cfg["pipeline"]["chat_model"],
            "instructions": instructions(self.cfg) + (TAG_RULES if self.tagged else ""),
            "input": [{"role": "user", "content": text}],
            "max_output_tokens": self.cfg["assistant"]["max_response_tokens"],
        }
        if self.previous:
            body["previous_response_id"] = self.previous
        tool = mcp_tool(self.cfg)
        if tool:
            body["tools"] = [tool]
        if not self.tagged:
            body["text"] = {"format": {"type": "json_schema", "name": "owl_reply",
                                       "schema": REPLY_SCHEMA, "strict": True}}
        return self.http.post(f"{OPENAI}/responses", json=body, timeout=60)

    def reply(self, text):
        r = self.request(text)
        if r.status_code == 400 and not self.tagged and mcp_tool(self.cfg):
            self.tagged = True               # JSON format + MCP refused: use tags
            r = self.request(text)
        r.raise_for_status()
        data = r.json()
        self.previous = data["id"]
        out = "".join(c["text"] for item in data["output"] if item["type"] == "message"
                      for c in item["content"] if c["type"] == "output_text")
        return parse_tagged(out) if self.tagged else json.loads(out)


def wav_bytes(samples, rate):
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(samples.astype("<i2").tobytes())
    return buf.getvalue()


GEMINI_SCHEMA = {
    "type": "object",
    "properties": {
        "heard": {"type": "string",
                  "description": "What the user just said, transcribed; empty if no speech."},
        "say": {"type": "string"},
        "emotion": {"type": "string", "enum": list(EMOTIONS)},
        "sleep": {"type": "boolean"},
    },
    "required": ["heard", "say", "emotion", "sleep"],
}
MAX_TOOL_ROUNDS = 5


class GeminiBrain:
    """Gemini hears the recorded audio directly and replies in one request.

    MCP tools run on the Pi via an MCPBridge: their schemas are offered to Gemini
    as function declarations, and each call Gemini asks for is executed and sent
    back until it gives the final JSON reply. Memory keeps the last few exchanges
    as plain text (audio is never re-sent).
    """

    def __init__(self, cfg, key, bridge=None):
        self.cfg = cfg
        self.bridge = bridge
        self.http = requests.Session()
        self.http.headers["x-goog-api-key"] = key
        self.history = []

    def reset(self):
        self.history = []

    def generate(self, contents):
        p = self.cfg["pipeline"]
        config = {"responseMimeType": "application/json", "responseJsonSchema": GEMINI_SCHEMA,
                  "maxOutputTokens": self.cfg["assistant"]["max_response_tokens"]}
        if p.get("gemini_thinking"):
            config["thinkingConfig"] = {"thinkingLevel": p["gemini_thinking"]}
        body = {"systemInstruction": {"parts": [{"text": instructions(self.cfg, getattr(self, "kampus", False))}]},
                "contents": contents, "generationConfig": config}
        if self.bridge and self.bridge.tools:
            body["tools"] = [{"functionDeclarations": self.bridge.declarations()}]
        r = self.http.post(f"{GEMINI}/models/{p['gemini_model']}:generateContent",
                           json=body, timeout=60)
        r.raise_for_status()
        data = r.json()
        if not data.get("candidates"):
            raise RuntimeError(f"Gemini gave no answer: {data.get('promptFeedback', data)}")
        return data["candidates"][0].get("content", {"parts": []})

    def turn(self, text=None, samples=None, rate=None):
        if text is not None:
            parts = [{"text": text}]
        else:
            parts = [{"inlineData": {"mimeType": "audio/wav",
                                     "data": base64.b64encode(wav_bytes(samples, rate)).decode()}},
                     {"text": "(The audio above is the user's spoken turn.)"}]
        contents = self.history + [{"role": "user", "parts": parts}]
        for _ in range(MAX_TOOL_ROUNDS):
            content = self.generate(contents)
            calls = [part["functionCall"] for part in content.get("parts", []) if "functionCall" in part]
            if not calls:
                break
            contents.append(content)       # keep Gemini's call (and its thought signature)
            contents.append({"role": "user", "parts": [
                {"functionResponse": {"name": c["name"],
                                      "response": {"result": self.bridge.call(c["name"], c.get("args", {}))}}}
                for c in calls]})
        else:
            raise RuntimeError("Gemini kept calling tools without answering")
        out = "".join(part.get("text", "") for part in content.get("parts", []) if not part.get("thought"))
        answer = json.loads(out)
        heard = text if text is not None else answer.get("heard", "").strip()
        if not heard:
            return None
        answer["heard"] = heard
        self.history += [{"role": "user", "parts": [{"text": heard}]},
                         {"role": "model", "parts": [{"text": answer["say"]}]}]
        self.history = self.history[-2 * self.cfg["pipeline"]["history_turns"]:]
        return answer


TAG_RULES = """
Start your reply with the expression in square brackets, e.g. [happy], then the words to
speak. End with [sleep] only when the user says goodbye or asks you to sleep."""


def parse_tagged(out):
    """'[happy] Hello there! [sleep]' -> {'say': 'Hello there!', 'emotion': 'happy', 'sleep': True}"""
    tags = re.findall(r"\[(\w+)\]", out)
    say = re.sub(r"\s*\[\w+\]\s*", " ", out).strip()
    emotion = next((t for t in tags if t in EMOTIONS), "explaining")
    return {"say": say, "emotion": emotion, "sleep": "sleep" in tags}


class Voice:
    """ElevenLabs text-to-speech, streamed straight into the speaker."""

    def __init__(self, cfg, key, voice_id):
        self.cfg = cfg
        self.voice_id = voice_id
        self.http = requests.Session()
        self.http.headers["xi-api-key"] = key
        self.chars_used = 0

    def speak(self, text, audio, on_first_audio=None):
        # pcm_24000 = raw 16-bit mono at 24 kHz, exactly what AudioIO.feed_pcm24 plays.
        params = {"output_format": "pcm_24000"}
        if self.cfg["pipeline"].get("tts_latency"):
            params["optimize_streaming_latency"] = self.cfg["pipeline"]["tts_latency"]
        for attempt in (1, 2):               # one quick retry if the hotspot hiccups
            try:
                r = self.http.post(f"{ELEVENLABS}/text-to-speech/{self.voice_id}/stream",
                                   params=params, stream=True, timeout=(5, 20),
                                   json={"text": text, "model_id": self.cfg["pipeline"]["tts_model"]})
                break
            except requests.ConnectionError:
                if attempt == 2:
                    raise
        r.raise_for_status()
        leftover = b""
        for chunk in r.iter_content(4800):
            chunk = leftover + chunk
            cut = len(chunk) - len(chunk) % 2
            leftover = chunk[cut:]
            if cut:
                audio.feed_pcm24(chunk[:cut])
                if on_first_audio:
                    on_first_audio()
                    on_first_audio = None
        self.chars_used += len(text)


def heard_wake_phrase(model, samples, rate, phrases):
    """Run the offline Vosk model on one utterance and look for the wake phrase."""
    import soxr
    from vosk import KaldiRecognizer
    rec = KaldiRecognizer(model, 16000, json.dumps(phrases + ["[unk]"]))
    rec.AcceptWaveform(soxr.resample(samples, rate, 16000).astype("<i2").tobytes())
    return phrase_matches(json.loads(rec.FinalResult()).get("text", ""), phrases)


def wait_for_playback(audio, state=None):
    while audio.playing():
        time.sleep(0.05)
    if state:
        state.cue()                            # mic is live again: blink "your turn"


def main():
    parser = argparse.ArgumentParser(description="Owl with ChatGPT brain and ElevenLabs voice")
    parser.add_argument("--no-wake", action="store_true", help="start awake, no wake phrase")
    parser.add_argument("--text", action="store_true", help="type messages instead of speaking")
    parser.add_argument("--no-display", action="store_true", help="run without the eye screens")
    args = parser.parse_args()

    cfg = load_config(ROOT / "config.toml")
    from dotenv import load_dotenv
    load_dotenv(ROOT / ".env")
    apply_env(cfg)
    use_gemini = cfg["pipeline"]["brain"] in ("gemini", "gemini-live")
    brain_var = "GEMINI_API_KEY" if use_gemini else "OPENAI_API_KEY"
    brain_key = (os.environ.get(brain_var, "")
                 or (os.environ.get("GOOGLE_API_KEY", "") if use_gemini else "")).strip()
    # Gemini Live can speak with its own voice; otherwise ElevenLabs speaks.
    gemini_voice = cfg["pipeline"]["brain"] == "gemini-live" and cfg["pipeline"].get("voice") == "gemini"
    eleven_key = os.environ.get("ELEVENLABS_API_KEY", "").strip()
    voice_id = (os.environ.get("ELEVENLABS_VOICE_ID", "").strip()
                or cfg["pipeline"]["voice_id"].strip())
    needed = [(brain_var, brain_key)]
    if not gemini_voice:
        needed += [("ELEVENLABS_API_KEY", eleven_key), ("ELEVENLABS_VOICE_ID", voice_id)]
    missing = [n for n, v in needed if not v]
    if missing:
        raise SystemExit("Add to .env on the Pi: " + ", ".join(missing))

    from audio_io import AudioIO
    bridge = None
    if use_gemini:
        if mcp_tool(cfg):
            from mcp_bridge import MCPBridge
            m = cfg["mcp"]
            try:
                bridge = MCPBridge(m["server_url"], m.get("authorization", ""), m.get("allowed_tools"),
                                   call_timeout=m.get("call_timeout", 10))
                print(f"MCP server: {len(bridge.tools)} tools from {m['server_url']}", flush=True)
            except RuntimeError as exc:
                print(f"{exc}; continuing without MCP tools", flush=True)
        if cfg["pipeline"]["brain"] == "gemini-live":
            from gemini_live import GeminiLiveBrain
            brain = GeminiLiveBrain(cfg, brain_key, lambda: instructions(cfg, kampus=bool(bridge)), bridge)
            print(f"Brain: Gemini Live ({cfg['pipeline']['gemini_live_model']})", flush=True)
        else:
            brain = GeminiBrain(cfg, brain_key, bridge)
            brain.kampus = bool(bridge)
            print(f"Brain: Gemini ({cfg['pipeline']['gemini_model']})", flush=True)
    else:
        brain = Brain(cfg, brain_key)
        print(f"Brain: ChatGPT ({cfg['pipeline']['chat_model']})", flush=True)
        if mcp_tool(cfg):
            print(f"MCP server: {cfg['mcp']['server_url']}", flush=True)
    voice = None if gemini_voice else Voice(cfg, eleven_key, voice_id)
    print("Voice: " + (f"Gemini ({cfg['pipeline']['gemini_voice']})" if gemini_voice else "ElevenLabs"),
          flush=True)
    streaming_mic = hasattr(brain, "start_audio")      # Gemini Live listens while you talk
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
        try:
            if voice:
                voice.speak(cfg["pipeline"]["greeting"], audio)
            else:                             # Gemini greets in its own voice
                brain.turn(text="(You were just woken up. Say exactly: "
                                f"{cfg['pipeline']['greeting']})",
                           on_audio=lambda pcm, e: audio.feed_pcm24(pcm))
            wait_for_playback(audio, state)
        except Exception as exc:
            print(f"  Greeting skipped, voice unreachable: {type(exc).__name__}", flush=True)
            state.cue()

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

            said = text = None
            if args.text:
                text = input("\nYou: ").strip()
                if not text:
                    continue
                state.speech(False)
            else:
                state.phase("idle")

                def heard_start():
                    state.speech(True)
                    if streaming_mic:
                        brain.start_audio(rate)

                said = listen(audio, rate, block_ms, threshold, timeout=sleep_after,
                              on_start=heard_start,
                              on_audio=brain.feed_audio if streaming_mic else None)
                if said is None:            # nobody spoke for sleep_after seconds
                    if streaming_mic:
                        brain.close()
                    awake = False
                    go_to_sleep()
                    continue
                state.speech(False)          # eyes: thinking

            try:
                # Delay is measured from the moment the user stopped talking.
                t0 = said.ended if said else time.monotonic()
                first_words = []

                def first_audio():
                    if not first_words:
                        first_words.append(time.monotonic() - t0)

                def speak_sentence(sentence, expression):
                    if not first_words:          # first sentence: set the eyes, start talking
                        state.phase("idle")
                        state.emotion(expression["emotion"])
                    voice.speak(sentence, audio, on_first_audio=first_audio)

                def play_gemini(pcm, expression):
                    if not first_words:          # first audio: set the eyes
                        state.phase("idle")
                        state.emotion(expression["emotion"])
                    audio.feed_pcm24(pcm)
                    first_audio()

                if gemini_voice:
                    extra = {"on_audio": play_gemini}
                else:
                    extra = {"on_sentence": speak_sentence} if getattr(brain, "streams", False) else {}
                try:
                    if said and streaming_mic:
                        answer = brain.finish_audio(**extra)
                    else:
                        answer = brain.turn(text=text, samples=said.samples if said else None,
                                            rate=rate, **extra)
                except Exception as exc:
                    if first_words:           # already said part of an answer: don't repeat it
                        raise
                    # The cloud dropped the turn (e.g. Gemini Live "1011 internal error"):
                    # resend the same recording once on a fresh connection.
                    print(f"  Connection dropped ({type(exc).__name__}); retrying once...", flush=True)
                    answer = brain.turn(text=text, samples=said.samples if said else None,
                                        rate=rate, **extra)
                if answer is None:            # no words in the recording
                    state.phase("idle")
                    continue
                if said:
                    print(f"\nYou: {answer['heard']}", flush=True)
                state.phase("idle")
                state.emotion(answer["emotion"])
                if not extra:
                    voice.speak(answer["say"], audio, on_first_audio=first_audio)
                timing = getattr(brain, "timing", {})
                detail = (f"; upload {timing['send']:.1f} s, Gemini's first words after "
                          f"{timing.get('first_text', 0):.1f} s more") if "send" in timing else ""
                after = "after you stopped" if said else "after your message"
                print(f"Owl [{answer['emotion']}]: {answer['say']}"
                      f"  (first audio {first_words[0]:.1f} s {after}{detail})" if first_words else "", flush=True)
                wait_for_playback(audio, state)
                if voice:
                    print(f"  ElevenLabs characters used this run: {voice.chars_used}", flush=True)
            except Exception as exc:          # never let one bad turn stop the owl
                response = getattr(exc, "response", None)
                detail = getattr(response, "text", "")[:300] if response is not None else ""
                print(f"  Cloud request failed: {type(exc).__name__}: {exc} {detail}", flush=True)
                state.phase("idle")
                state.emotion("confused")
                if not first_words and voice:
                    try:
                        voice.speak("Sorry, I lost my connection. Could you ask again?", audio)
                        wait_for_playback(audio, state)
                    except Exception:
                        pass
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
        if hasattr(brain, "close"):
            brain.close()
        if bridge:
            bridge.close()
        if voice:
            print(f"ElevenLabs characters used this run: {voice.chars_used}")


if __name__ == "__main__":
    main()
