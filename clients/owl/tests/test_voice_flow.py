"""Exercise voice event ordering without microphones, network, or API credit."""

import base64
import json
import unittest
from unittest.mock import patch

from owl.brain.realtime import VoiceSession
from owl.config import EXAMPLE_CONFIG, apply_env, load_config
from owl.state import OwlState


class Socket:
    def __init__(self, events):
        self.events = iter(events)
        self.sent = []

    def __aiter__(self):
        return self

    async def __anext__(self):
        try:
            return json.dumps(next(self.events))
        except StopIteration:
            raise StopAsyncIteration

    async def send(self, text):
        self.sent.append(json.loads(text))


class Audio:
    def __init__(self):
        self.chunks, self.cleared = [], False

    def playing(self):
        return False

    def feed_pcm24(self, pcm):
        self.chunks.append(pcm)

    def clear_output(self):
        self.cleared = True


def planned(emotion="love", camera=False):
    return {
        "type": "response.done",
        "response": {
            "status": "completed",
            "output": [
                {
                    "type": "function_call",
                    "name": "prepare_reply",
                    "call_id": "test-call",
                    "arguments": json.dumps({"emotion": emotion, "use_camera": camera}),
                }
            ],
        },
    }


class VoiceFlowTests(unittest.IsolatedAsyncioTestCase):
    def session(self, events):
        cfg = load_config(EXAMPLE_CONFIG)
        state = OwlState(300)
        state.wake()
        session = VoiceSession(cfg, state, Audio(), None)
        session.ws = Socket(events)
        return session

    async def test_greeting_then_human_turn_plans_emotion_and_speaks(self):
        pcm = b"\x01\x00" * 16
        session = self.session(
            [
                {"type": "session.updated"},
                {"type": "response.done", "response": {"status": "completed"}},
                {"type": "input_audio_buffer.speech_started"},
                {"type": "input_audio_buffer.speech_stopped"},
                {"type": "input_audio_buffer.committed"},
                planned(),
                {"type": "response.output_audio.delta", "delta": base64.b64encode(pcm).decode()},
                {"type": "response.done", "response": {"status": "completed"}},
            ]
        )
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
        with patch("owl.brain.realtime.capture_camera", side_effect=AssertionError("Must not capture")):
            await session.receive()
        items = [e["item"] for e in session.ws.sent if e["type"] == "conversation.item.create"]
        self.assertEqual(len(items), 1)
        self.assertIn("disabled", json.loads(items[0]["output"])["camera"])

    async def test_enabled_requested_camera_sends_one_image_before_answer(self):
        session = self.session([planned(camera=True)])
        session.cfg["camera"]["enabled"] = True
        session.stage = "planning"
        with patch(
            "owl.brain.realtime.capture_camera", return_value="data:image/jpeg;base64,TEST"
        ) as capture:
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
        session = self.session(
            [{"type": "error", "error": {"code": "invalid_api_key", "message": "Invalid key"}}]
        )
        with self.assertRaisesRegex(RuntimeError, "invalid_api_key"):
            await session.receive()

    async def test_mcp_server_is_offered_to_answers_but_not_plans(self):
        session = self.session([planned("happy")])
        session.cfg["mcp"] = {
            "server_label": "kampus",
            "server_url": "https://mcp.test/mcp",
            "authorization": "secret",
            "allowed_tools": ["events"],
        }
        session.stage = "planning"
        await session.receive()
        answer = [e["response"] for e in session.ws.sent if e["type"] == "response.create"][-1]
        self.assertEqual(answer["tool_choice"], "auto")
        self.assertEqual(
            answer["tools"],
            [
                {
                    "type": "mcp",
                    "server_label": "kampus",
                    "server_url": "https://mcp.test/mcp",
                    "require_approval": "never",
                    "authorization": "secret",
                    "allowed_tools": ["events"],
                }
            ],
        )
        await session.request_answer(planning=True)
        self.assertNotIn("tools", session.ws.sent[-1]["response"])

    async def test_mcp_list_failure_does_not_end_conversation(self):
        session = self.session([{"type": "mcp_list_tools.failed"}, {"type": "response.mcp_call.failed"}])
        await session.receive()

    async def test_barge_in_stops_playback_and_truncates_heard_audio(self):
        pcm = b"\x01\x00" * 2400  # 100 ms at 24 kHz
        session = self.session(
            [
                {
                    "type": "response.output_audio.delta",
                    "item_id": "reply-1",
                    "delta": base64.b64encode(pcm).decode(),
                },
                {"type": "input_audio_buffer.speech_started"},
                {"type": "response.done", "response": {"status": "cancelled"}},
            ]
        )
        session.barge_in, session.busy, session.stage = True, True, "answer"
        await session.receive()
        truncate = [e for e in session.ws.sent if e["type"] == "conversation.item.truncate"]
        self.assertEqual(len(truncate), 1)
        self.assertEqual(truncate[0]["item_id"], "reply-1")
        self.assertLessEqual(truncate[0]["audio_end_ms"], 100)
        self.assertTrue(session.audio.cleared)
        self.assertFalse(session.busy)

    async def test_without_barge_in_speech_never_interrupts(self):
        pcm = b"\x01\x00" * 2400
        session = self.session(
            [
                {
                    "type": "response.output_audio.delta",
                    "item_id": "reply-1",
                    "delta": base64.b64encode(pcm).decode(),
                },
                {"type": "input_audio_buffer.speech_started"},
            ]
        )
        session.busy, session.stage = True, "answer"
        await session.receive()
        self.assertFalse([e for e in session.ws.sent if e["type"] == "conversation.item.truncate"])
        self.assertFalse(session.audio.cleared)


class EnvOverrideTests(unittest.TestCase):
    def test_env_overrides_config(self):
        cfg = load_config(EXAMPLE_CONFIG)
        env = {
            "OPENAI_REALTIME_MODEL": "gpt-test",
            "AUDIO_INPUT_DEVICE": "3",
            "AUDIO_OUTPUT_DEVICE": "JBL Go 3",
            "MCP_SERVER_URL": "https://mcp.test/mcp",
            "MCP_AUTHORIZATION": "tok",
            "MCP_ALLOWED_TOOLS": "a, b",
        }
        with patch.dict("os.environ", env):
            apply_env(cfg)
        self.assertEqual(cfg["assistant"]["model"], "gpt-test")
        self.assertEqual(cfg["audio"]["input_device"], 3)
        self.assertEqual(cfg["audio"]["output_device"], "JBL Go 3")
        self.assertEqual(cfg["mcp"]["server_url"], "https://mcp.test/mcp")
        self.assertEqual(cfg["mcp"]["allowed_tools"], ["a", "b"])

    def test_no_mcp_server_means_no_mcp_tool(self):
        from owl.brain.realtime import session_settings

        cfg = load_config(EXAMPLE_CONFIG)
        self.assertEqual([t["type"] for t in session_settings(cfg)["tools"]], ["function"])


if __name__ == "__main__":
    unittest.main()
