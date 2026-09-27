"""Gemini Live brain for the ElevenLabs-voiced owl (voice_pipeline.py).

Gemini 3.8 Live is a native-audio model: it only answers with its own voice.
The owl wants the ElevenLabs voice, so this brain uses Live for hearing and
thinking only: it sends the recorded turn, collects the transcripts of what the
user said and what Gemini answered, and discards Gemini's audio. The answer
text is then spoken by ElevenLabs.

Turn-taking stays local (turns.py), so Gemini's own voice-activity detection is
switched off and each turn is framed with activityStart / activityEnd. The eye
expression comes from a set_expression function call; MCP tools, if any, run
on the Pi through mcp_bridge.py.

One WebSocket session is kept open while the owl is awake. Live sessions are
limited to 15 minutes of audio, so it reconnects before that and replays the
last few exchanges as text.
"""
import base64
import json
import re
import time

import numpy as np
import soxr

from state import EMOTIONS

LIVE_URL = ("wss://generativelanguage.googleapis.com/ws/"
            "google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent")
SESSION_MAX = 14 * 60        # seconds; Live audio sessions end at 15 minutes
CHUNK_MS = 100               # audio sent in 100 ms pieces
TURN_TIMEOUT = 60            # seconds to wait for Gemini to finish a turn
MAX_TOOL_ROUNDS = 5
LONG_CLAUSE = 60             # characters before a long sentence is split at a comma
FIRST_CLAUSE = 25            # the very first words go out at a comma even sooner

EXPRESSION_TOOL = {
    "name": "set_expression",
    "description": "Set the owl's eye expression for the reply you are about to give. "
                   "Call it once at the start of every reply.",
    "parameters": {
        "type": "OBJECT",
        "properties": {
            "emotion": {"type": "STRING", "enum": list(EMOTIONS)},
            "sleep": {"type": "BOOLEAN",
                      "description": "True only if the user said goodbye or asked you to sleep."},
        },
        "required": ["emotion", "sleep"],
    },
}

LIVE_RULES = """
Always call set_expression at the start of each reply, then answer out loud."""


class GeminiLiveBrain:
    streams = True               # can hand over the reply sentence by sentence

    timing = {}                  # seconds spent sending the turn / until Gemini's first words

    def __init__(self, cfg, key, instructions, bridge=None, connect=None):
        self.cfg, self.key, self.instructions, self.bridge = cfg, key, instructions, bridge
        if connect is None:
            from websockets.sync.client import connect
        self.connect = connect
        self.ws = None
        self.opened = 0.0
        self.history = []            # text-only memory, replayed after a reconnect
        self.thinking = cfg["pipeline"].get("gemini_live_thinking", "")
        self.stream_error = None
        self.resampler = None

    # ---------- session ----------

    def reset(self):
        """Owl fell asleep: close the session and forget the conversation."""
        self.close()
        self.history = []

    def close(self):
        if self.ws:
            try:
                self.ws.close()
            except Exception:
                pass
        self.ws = None

    def _send(self, message):
        self.ws.send(json.dumps(message))

    def _recv(self, deadline):
        raw = self.ws.recv(timeout=max(0.1, deadline - time.monotonic()))
        return json.loads(raw)

    def _open(self):
        self.close()
        p = self.cfg["pipeline"]
        tools = [EXPRESSION_TOOL]
        if self.bridge:
            tools += self.bridge.declarations()
        generation = {"responseModalities": ["AUDIO"]}
        if self.thinking:                     # less thinking = faster first words
            generation["thinkingConfig"] = {"thinkingLevel": self.thinking}
        self.ws = self.connect(f"{LIVE_URL}?key={self.key}", open_timeout=20,
                               max_size=16 * 1024 * 1024)
        self._send({"setup": {
            "model": f"models/{p['gemini_live_model']}",
            "generationConfig": generation,
            "systemInstruction": {"parts": [{"text": (self.instructions() if callable(self.instructions)
                                                       else self.instructions) + LIVE_RULES}]},
            "tools": [{"functionDeclarations": tools}],
            "inputAudioTranscription": {},
            "outputAudioTranscription": {},
            "realtimeInputConfig": {"automaticActivityDetection": {"disabled": True}},
        }})
        deadline = time.monotonic() + 20
        try:
            while "setupComplete" not in self._recv(deadline):
                pass
        except Exception:
            if not self.thinking:
                raise
            self.thinking = ""                # model refused the thinking setting: retry without
            return self._open()
        self.opened = time.monotonic()
        if self.history:                      # carry the conversation over a reconnect
            self._send({"clientContent": {"turns": self.history, "turnComplete": False}})

    def _ensure_open(self):
        if self.ws is None or time.monotonic() - self.opened > SESSION_MAX:
            self._open()

    # ---------- one exchange ----------

    def turn(self, text=None, samples=None, rate=None, on_sentence=None):
        """Send one user turn; return {heard, say, emotion, sleep}, or None if nothing was said.

        Gemini produces its answer at speaking pace, so waiting for the whole
        reply takes seconds. on_sentence(sentence, expression) is called as soon
        as each sentence is complete, letting ElevenLabs start on the first one.
        """
        try:
            start = time.monotonic()
            self._ensure_open()
            self._send_turn(text, samples, rate)
            self.timing = {"send": time.monotonic() - start}
            return self._collect(text, on_sentence)
        except Exception:
            self.close()                      # a broken session is reopened next turn
            raise

    # ---------- streaming a spoken turn while it is recorded ----------

    def start_audio(self, rate):
        """User started talking: open the turn and stream audio as it arrives."""
        self.stream_error = None
        try:
            self._ensure_open()
            self.resampler = soxr.ResampleStream(rate, 16000, 1, dtype="int16")
            self._send({"realtimeInput": {"activityStart": {}}})
        except Exception as exc:
            self.stream_error = exc

    def feed_audio(self, samples):
        if self.stream_error or self.resampler is None:
            return
        try:
            self._send_pcm(self.resampler.resample_chunk(samples))
        except Exception as exc:
            self.stream_error = exc

    def finish_audio(self, on_sentence=None):
        """User stopped: close the turn and collect the answer (see turn())."""
        try:
            if self.stream_error:
                raise self.stream_error
            start = time.monotonic()
            self._send_pcm(self.resampler.resample_chunk(np.zeros(0, dtype=np.int16), last=True))
            self._send({"realtimeInput": {"activityEnd": {}}})
            self.timing = {"send": time.monotonic() - start}
            return self._collect(None, on_sentence)
        except Exception:
            self.close()
            raise
        finally:
            self.resampler = None

    def _send_pcm(self, pcm):
        if pcm.size:
            self._send({"realtimeInput": {"audio": {
                "data": base64.b64encode(pcm.astype("<i2").tobytes()).decode(),
                "mimeType": "audio/pcm;rate=16000"}}})

    def _send_turn(self, text, samples, rate):
        if text is not None:
            self._send({"realtimeInput": {"text": text}})
            return
        pcm = soxr.resample(samples, rate, 16000).astype("<i2")
        step = 16000 * CHUNK_MS // 1000
        self._send({"realtimeInput": {"activityStart": {}}})
        for i in range(0, len(pcm), step):
            self._send({"realtimeInput": {"audio": {
                "data": base64.b64encode(pcm[i:i + step].tobytes()).decode(),
                "mimeType": "audio/pcm;rate=16000"}}})
        self._send({"realtimeInput": {"activityEnd": {}}})

    def _collect(self, text, on_sentence=None):
        heard, said = [], []
        expression = {"emotion": "explaining", "sleep": False}
        pending = ""                          # reply text not yet handed to on_sentence
        said_any = []                         # sentences already handed over

        def emit(final=False):
            nonlocal pending
            parts = re.split(r"(?<=[.!?])\s+", pending)
            ready, pending = (parts, "") if final else (parts[:-1], parts[-1])
            # A long sentence still growing: speak up to its last comma now
            # (sooner for the very first words, so the owl starts talking quickly).
            limit = LONG_CLAUSE if said_any else FIRST_CLAUSE
            if not final and not ready and len(pending) > limit and ", " in pending:
                head, _, pending = pending.rpartition(", ")
                ready.append(head + ",")
            for sentence in ready:
                sentence = " ".join(sentence.split())
                if sentence and on_sentence:
                    said_any.append(sentence)
                    on_sentence(sentence, expression)

        deadline = time.monotonic() + TURN_TIMEOUT
        rounds = 0
        # A tool call ends Gemini's current model turn (turnComplete) and the
        # spoken answer follows in a new one once our toolResponse arrives, so
        # a turnComplete right after a tool call is not the end of the reply.
        answer_pending = False
        while True:
            msg = self._recv(deadline)
            if "toolCall" in msg:
                answer_pending = True
                rounds += 1
                if rounds > MAX_TOOL_ROUNDS:
                    raise RuntimeError("Gemini kept calling tools without answering")
                replies = []
                for call in msg["toolCall"].get("functionCalls", []):
                    args = call.get("args", {}) or {}
                    if call["name"] == "set_expression":
                        if args.get("emotion") in EMOTIONS:
                            expression["emotion"] = args["emotion"]
                        expression["sleep"] = bool(args.get("sleep", False))
                        result = {"ok": True}
                    elif self.bridge:
                        result = {"result": self.bridge.call(call["name"], args)}
                    else:
                        result = {"error": f"unknown tool {call['name']}"}
                    replies.append({"id": call.get("id"), "name": call["name"], "response": result})
                self._send({"toolResponse": {"functionResponses": replies}})
                continue
            if "goAway" in msg:               # server will close soon; reopen next turn
                self.opened = 0.0
            content = msg.get("serverContent", {})
            if "inputTranscription" in content:
                heard.append(content["inputTranscription"].get("text", ""))
            if "outputTranscription" in content:
                if "first_text" not in self.timing:
                    self.timing["first_text"] = time.monotonic() - deadline + TURN_TIMEOUT
                piece = content["outputTranscription"].get("text", "")
                said.append(piece)
                pending += piece
                emit()
                answer_pending = False
            # modelTurn audio (Gemini's own voice) is ignored: ElevenLabs speaks instead
            if content.get("turnComplete"):
                if answer_pending:
                    answer_pending = False    # tool step done; wait for the answer turn
                    continue
                break

        emit(final=True)
        user = (text if text is not None else "".join(heard)).strip()
        say = " ".join("".join(said).split())
        if not user or not say:
            return None
        self.history += [{"role": "user", "parts": [{"text": user}]},
                         {"role": "model", "parts": [{"text": say}]}]
        self.history = self.history[-2 * self.cfg["pipeline"]["history_turns"]:]
        return {"heard": user, "say": say, **expression}
