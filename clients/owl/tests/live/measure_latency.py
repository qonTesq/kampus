"""Measure the owl's response delay end to end, without a person talking.

ElevenLabs speaks a test question once; that audio is streamed into Gemini Live
at real-time pace exactly like the webcam mic would be (turns.listen streaming),
including the silence the turn detector waits for. The delay reported is from
the end of the question to the first ElevenLabs audio of the answer.
Nothing is played on the speaker. Costs a few hundred ElevenLabs characters.

  uv run python tests/live/measure_latency.py [rounds]
"""
import os
import sys
import time

import numpy as np
import soxr
from dotenv import load_dotenv

from owl.brain.gemini_live import GeminiLiveBrain
from owl.brain.pipeline import instructions
from owl.brain.turns import SILENCE_END
from owl.config import ENV_FILE, apply_env, load_config
from owl.voice.elevenlabs import Voice

load_dotenv(ENV_FILE)

QUESTIONS = ["What is a fun fact about owls?", "Tell me a short joke.", "How are you today?"]


class Sink:
    """Stands in for the speaker: just remembers when audio first arrived."""
    def __init__(self): self.first = None
    def feed_pcm24(self, pcm):
        if self.first is None:
            self.first = time.monotonic()


def main():
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 3
    cfg = apply_env(load_config())
    voice = Voice(cfg, os.environ["ELEVENLABS_API_KEY"].strip(),
                  os.environ.get("ELEVENLABS_VOICE_ID", "").strip() or cfg["pipeline"]["voice_id"])
    brain = GeminiLiveBrain(cfg, os.environ["GEMINI_API_KEY"].strip(), instructions(cfg))
    brain._ensure_open()                          # the session is already open while awake
    print(f"thinking setting in use: {brain.thinking or 'model default'}; "
          f"voice: {cfg['pipeline'].get('voice', 'elevenlabs')}")
    results = []
    for i in range(rounds):
        question = QUESTIONS[i % len(QUESTIONS)]
        recorder = Sink()
        pcm = []
        recorder.feed_pcm24 = lambda b, pcm=pcm: pcm.append(b)
        voice.speak(question, recorder)
        speech = soxr.resample(np.frombuffer(b"".join(pcm), "<i2"), 24000, 48000).astype(np.int16)
        silence = np.zeros(int(48000 * SILENCE_END), dtype=np.int16)

        brain.start_audio(48000)
        for chunk in np.split(speech[: len(speech) // 960 * 960], len(speech) // 960):
            brain.feed_audio(chunk)
            time.sleep(0.02)                      # real-time pace, like the mic
        speech_end = time.monotonic()
        for chunk in np.split(silence, len(silence) // 960):
            brain.feed_audio(chunk)
            time.sleep(0.02)
        sink = Sink()
        if cfg["pipeline"].get("voice") == "gemini":
            answer = brain.finish_audio(on_audio=lambda pcm, e: sink.feed_pcm24(pcm))
        else:
            answer = brain.finish_audio(on_sentence=lambda s, e: voice.speak(s, sink))
        delay = sink.first - speech_end
        results.append(delay)
        print(f"Q: {question!r} heard as {answer['heard']!r}")
        print(f"   A: {answer['say'][:70]!r}  -> first audio {delay:.2f} s after speech ended "
              f"(turn detector wait {SILENCE_END:.1f} s + Gemini {brain.timing.get('first_text', 0):.2f} s + ElevenLabs)")
    brain.close()
    print(f"median delay: {sorted(results)[len(results) // 2]:.2f} s   ElevenLabs chars: {voice.chars_used}")


if __name__ == "__main__":
    main()
