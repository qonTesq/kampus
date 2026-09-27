"""Gemini Live brain protocol, with a fake WebSocket (no network, no credit)."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gemini_live import GeminiLiveBrain
from main import load_config

CONFIG = Path(__file__).resolve().parents[1] / "config.toml"


class FakeSocket:
    """Replays scripted server messages; each script entry answers one client turn."""

    def __init__(self, scripts):
        self.scripts, self.sent, self.pending, self.closed = list(scripts), [], [], False

    def send(self, text):
        msg = json.loads(text)
        self.sent.append(msg)
        ends_turn = ("setup" in msg or "toolResponse" in msg
                     or "text" in msg.get("realtimeInput", {})
                     or "activityEnd" in msg.get("realtimeInput", {}))
        if ends_turn and self.scripts:
            self.pending += self.scripts.pop(0)

    def recv(self, timeout=None):
        if not self.pending:
            raise TimeoutError("nothing scripted")
        return json.dumps(self.pending.pop(0))

    def close(self): self.closed = True


def content(**kw): return {"serverContent": kw}


SETUP = [{"setupComplete": {}}]


class FakeTool:
    name, description = "next_event", "Next campus event"
    inputSchema = {"type": "object", "properties": {"topic": {"type": "string"}}}


class FakeBridge:
    tools = [FakeTool()]
    def __init__(self): self.calls = []
    def call(self, name, args):
        self.calls.append((name, args))
        return "Hackathon demo, Friday 12:00"


class GeminiLiveTests(unittest.TestCase):
    def brain(self, scripts, bridge=None):
        cfg = load_config(CONFIG)
        self.sockets = []

        def connect(url, **kw):
            self.url = url
            sock = FakeSocket(scripts.pop(0))
            self.sockets.append(sock)
            return sock
        return GeminiLiveBrain(cfg, "gkey", "Be an owl.", bridge, connect=connect)

    def test_spoken_turn_uses_transcripts_and_ignores_gemini_audio(self):
        brain = self.brain([[SETUP, [
            {"toolCall": {"functionCalls": [{"id": "c1", "name": "set_expression",
                                             "args": {"emotion": "happy", "sleep": False}}]}}], [
            content(inputTranscription={"text": "Hi "}),
            content(inputTranscription={"text": "owl"}),
            content(modelTurn={"parts": [{"inlineData": {"data": "AAAA"}}]}),
            content(outputTranscription={"text": "Hello "}),
            content(outputTranscription={"text": "there!"}),
            content(turnComplete=True)]]])
        got = brain.turn(samples=np.zeros(48000, dtype=np.int16), rate=48000)
        self.assertEqual(got, {"heard": "Hi owl", "say": "Hello there!", "emotion": "happy", "sleep": False})
        sent = self.sockets[0].sent
        setup = sent[0]["setup"]
        self.assertEqual(setup["model"], "models/gemini-3.8-live")
        self.assertEqual(setup["generationConfig"]["responseModalities"], ["AUDIO"])
        self.assertTrue(setup["realtimeInputConfig"]["automaticActivityDetection"]["disabled"])
        self.assertIn("inputAudioTranscription", setup)
        self.assertIn("outputAudioTranscription", setup)
        self.assertTrue(self.url.endswith("?key=gkey"))
        inputs = [m["realtimeInput"] for m in sent if "realtimeInput" in m]
        self.assertIn("activityStart", inputs[0])
        self.assertIn("activityEnd", inputs[-1])
        audio = [i["audio"] for i in inputs if "audio" in i]
        self.assertEqual(audio[0]["mimeType"], "audio/pcm;rate=16000")
        self.assertEqual(len(audio), 10)                       # 1 s at 16 kHz in 100 ms pieces
        self.assertEqual(sent[-1]["toolResponse"]["functionResponses"][0]["id"], "c1")

    def test_mcp_tool_runs_on_the_pi(self):
        bridge = FakeBridge()
        brain = self.brain([[SETUP, [
            {"toolCall": {"functionCalls": [{"id": "t1", "name": "next_event", "args": {"topic": "demo"}}]}}], [
            content(outputTranscription={"text": "It's Friday at noon."}),
            content(turnComplete=True)]]], bridge)
        got = brain.turn(text="When is the demo?")
        self.assertEqual(got["say"], "It's Friday at noon.")
        self.assertEqual(got["emotion"], "explaining")          # no set_expression -> default
        self.assertEqual(bridge.calls, [("next_event", {"topic": "demo"})])
        decls = self.sockets[0].sent[0]["setup"]["tools"][0]["functionDeclarations"]
        self.assertEqual([d["name"] for d in decls], ["set_expression", "next_event"])
        self.assertIn("parametersJsonSchema", decls[1])
        response = self.sockets[0].sent[-1]["toolResponse"]["functionResponses"][0]
        self.assertEqual(response["response"], {"result": "Hackathon demo, Friday 12:00"})

    def test_session_is_reused_then_reopened_with_history_after_limit(self):
        first = [content(outputTranscription={"text": "One."}), content(turnComplete=True)]
        second = [content(outputTranscription={"text": "Two."}), content(turnComplete=True)]
        third = [content(outputTranscription={"text": "Three."}), content(turnComplete=True)]
        brain = self.brain([[SETUP, first, second], [SETUP, third]])
        brain.turn(text="a")
        brain.turn(text="b")
        self.assertEqual(len(self.sockets), 1)                  # same session
        brain.opened -= 15 * 60                                 # pretend 15 minutes passed
        brain.turn(text="c")
        self.assertEqual(len(self.sockets), 2)
        self.assertTrue(self.sockets[0].closed)
        replay = self.sockets[1].sent[1]["clientContent"]
        self.assertEqual([t["parts"][0]["text"] for t in replay["turns"]], ["a", "One.", "b", "Two."])
        self.assertFalse(replay["turnComplete"])

    def test_sleep_forgets_and_closes(self):
        brain = self.brain([[SETUP, [content(outputTranscription={"text": "Hi."}), content(turnComplete=True)]]])
        brain.turn(text="hello")
        brain.reset()
        self.assertTrue(self.sockets[0].closed)
        self.assertEqual(brain.history, [])

    def test_silence_returns_none(self):
        brain = self.brain([[SETUP, [content(turnComplete=True)]]])
        self.assertIsNone(brain.turn(samples=np.zeros(4800, dtype=np.int16), rate=48000))
        self.assertEqual(brain.history, [])

    def test_broken_session_is_closed_and_error_raised(self):
        brain = self.brain([[SETUP, []]])                        # server never finishes the turn
        with patch("gemini_live.TURN_TIMEOUT", 0.2):
            with self.assertRaises(TimeoutError):
                brain.turn(text="hello")
        self.assertIsNone(brain.ws)


if __name__ == "__main__":
    unittest.main()
