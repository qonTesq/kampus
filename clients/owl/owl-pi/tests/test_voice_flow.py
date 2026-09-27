"""Exercise voice event ordering without microphones, network, or API credit."""
import base64
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from main import load_config
from realtime_voice import VoiceSession
from state import OwlState


class Socket:
    def __init__(self, events):
        self.events = iter(events)
        self.sent = []

    def __aiter__(self): return self

    async def __anext__(self):
        try: return json.dumps(next(self.events))
        except StopIteration: raise StopAsyncIteration

    async def send(self, text): self.sent.append(json.loads(text))


class Audio:
    def __init__(self): self.chunks = []
    def playing(self): return False
    def feed_pcm24(self, pcm): self.chunks.append(pcm)


def planned(emotion="love", camera=False):
    return {"type": "response.done", "response": {"status": "completed", "output": [
        {"type": "function_call", "name": "prepare_reply", "call_id": "test-call",
         "arguments": json.dumps({"emotion": emotion, "use_camera": camera})}]}}


class VoiceFlowTests(unittest.IsolatedAsyncioTestCase):
    def session(self, events):
        cfg = load_config(Path(__file__).resolve().parents[1] / "config.toml")
        state = OwlState(300)
        state.wake()
        session = VoiceSession(cfg, state, Audio(), None)
        session.ws = Socket(events)
        return session

    async def test_greeting_then_human_turn_plans_emotion_and_speaks(self):
        pcm = b"\x01\x00" * 16
        session = self.session([
            {"type": "session.updated"},
            {"type": "response.done", "response": {"status": "completed"}},
            {"type": "input_audio_buffer.speech_started"},
            {"type": "input_audio_buffer.speech_stopped"},
            {"type": "input_audio_buffer.committed"},
            planned(),
            {"type": "response.output_audio.delta", "delta": base64.b64encode(pcm).decode()},
            {"type": "response.done", "response": {"status": "completed"}},
        ])
        await session.receive()
        responses = [e["response"] for e in session.ws.sent if e["type"] == "response.create"]
        self.assertEqual(len(responses), 3)
        self.assertEqual(responses[0]["tool_choice"], "none")
        self.assertEqual(responses[1]["tool_choice"]["name"], "prepare_reply")
        self.assertEqual(responses[2]["tool_choice"], "none")
        self.assertEqual(session.state.snapshot().emotion, "love")
        self.assertEqual(session.audio.chunks, [pcm])
        self.assertFalse(session.busy)
        self.assertFalse(session.state.snapshot().user_speaking)

    async def test_disabled_camera_never_captures_or_sends_image(self):
        session = self.session([planned(camera=True)])
        session.stage = "planning"
        with patch("realtime_voice.capture_camera", side_effect=AssertionError("Must not capture")):
            await session.receive()
        items = [e["item"] for e in session.ws.sent if e["type"] == "conversation.item.create"]
        self.assertEqual(len(items), 1)
        self.assertIn("disabled", json.loads(items[0]["output"])["camera"])

    async def test_enabled_requested_camera_sends_one_image_before_answer(self):
        session = self.session([planned(camera=True)])
        session.cfg["camera"]["enabled"] = True
        session.stage = "planning"
        with patch("realtime_voice.capture_camera", return_value="data:image/jpeg;base64,TEST") as capture:
            await session.receive()
        capture.assert_called_once()
        events = session.ws.sent
        self.assertEqual(events[0]["item"]["type"], "function_call_output")
        self.assertEqual(events[1]["item"]["content"][1]["type"], "input_image")
        self.assertEqual(events[2]["type"], "response.create")

    async def test_unknown_expression_falls_back_to_normal(self):
        session = self.session([planned("not-an-expression")])
        session.stage = "planning"
        await session.receive()
        self.assertEqual(session.state.snapshot().emotion, "normal")

    async def test_service_failure_is_reported(self):
        session = self.session([{"type": "error", "error": {"code": "invalid_api_key", "message": "Invalid key"}}])
        with self.assertRaisesRegex(RuntimeError, "invalid_api_key"):
            await session.receive()


if __name__ == "__main__": unittest.main()
