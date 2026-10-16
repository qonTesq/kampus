"""Gemini Live brain protocol, with a fake WebSocket (no network, no credit)."""

import base64
import json
import unittest
from typing import ClassVar
from unittest.mock import patch

import numpy as np

from owl.brain.gemini_live import GeminiLiveBrain
from owl.config import EXAMPLE_CONFIG as CONFIG
from owl.config import load_config


class FakeSocket:
    """Replays scripted server messages; each script entry answers one client turn."""

    def __init__(self, scripts):
        self.scripts, self.sent, self.pending, self.closed = list(scripts), [], [], False

    def send(self, text):
        msg = json.loads(text)
        self.sent.append(msg)
        ends_turn = (
            "setup" in msg
            or "toolResponse" in msg
            or "text" in msg.get("realtimeInput", {})
            or "activityEnd" in msg.get("realtimeInput", {})
        )
        if ends_turn and self.scripts:
            self.pending += self.scripts.pop(0)

    def recv(self, timeout=None):
        if not self.pending:
            raise TimeoutError("nothing scripted")
        return json.dumps(self.pending.pop(0))

    def close(self):
        self.closed = True


def content(**kw):
    return {"serverContent": kw}


SETUP = [{"setupComplete": {}}]


class FakeTool:
    name, description = "next_event", "Next campus event"
    inputSchema: ClassVar[dict] = {"type": "object", "properties": {"topic": {"type": "string"}}}


class FakeBridge:
    tools: ClassVar[list] = [FakeTool()]

    def declarations(self):
        from owl.kampus.bridge import gemini_schema

        return [
            {
                "name": t.name,
                "description": t.description,
                "parametersJsonSchema": gemini_schema(t.inputSchema),
            }
            for t in self.tools
        ]

    def __init__(self):
        self.calls = []

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
        brain = self.brain(
            [
                [
                    SETUP,
                    [
                        {
                            "toolCall": {
                                "functionCalls": [
                                    {
                                        "id": "c1",
                                        "name": "set_expression",
                                        "args": {"emotion": "happy", "sleep": False},
                                    }
                                ]
                            }
                        }
                    ],
                    [
                        content(inputTranscription={"text": "Hi "}),
                        content(inputTranscription={"text": "owl"}),
                        content(modelTurn={"parts": [{"inlineData": {"data": "AAAA"}}]}),
                        content(outputTranscription={"text": "Hello "}),
                        content(outputTranscription={"text": "there!"}),
                        content(turnComplete=True),
                    ],
                ]
            ]
        )
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
        self.assertEqual(len(audio), 10)  # 1 s at 16 kHz in 100 ms pieces
        self.assertEqual(sent[-1]["toolResponse"]["functionResponses"][0]["id"], "c1")

    def test_answer_after_tool_call_arrives_in_a_second_model_turn(self):
        # Real Gemini Live order (seen on the Pi): tool call, turnComplete, then
        # after our toolResponse a new model turn with the spoken answer.
        brain = self.brain(
            [
                [
                    SETUP,
                    [
                        {
                            "toolCall": {
                                "functionCalls": [
                                    {
                                        "id": "c1",
                                        "name": "set_expression",
                                        "args": {"emotion": "happy", "sleep": False},
                                    }
                                ]
                            }
                        },
                        content(generationComplete=True),
                        content(turnComplete=True),
                    ],
                    [
                        content(outputTranscription={"text": "I am Owl, "}),
                        content(outputTranscription={"text": "your friendly robot."}),
                        content(generationComplete=True),
                        content(turnComplete=True),
                    ],
                ]
            ]
        )
        got = brain.turn(text="Who are you?")
        self.assertEqual(
            got,
            {
                "heard": "Who are you?",
                "say": "I am Owl, your friendly robot.",
                "emotion": "happy",
                "sleep": False,
            },
        )

    def test_sentences_are_handed_over_as_soon_as_complete(self):
        brain = self.brain(
            [
                [
                    SETUP,
                    [
                        {
                            "toolCall": {
                                "functionCalls": [
                                    {
                                        "id": "c1",
                                        "name": "set_expression",
                                        "args": {"emotion": "surprised", "sleep": False},
                                    }
                                ]
                            }
                        },
                        content(turnComplete=True),
                    ],
                    [
                        content(outputTranscription={"text": "Owls can turn "}),
                        content(outputTranscription={"text": "their heads far. They "}),
                        content(outputTranscription={"text": "also fly silently!"}),
                        content(turnComplete=True),
                    ],
                ]
            ]
        )
        spoken = []
        brain.turn(text="fun fact?", on_sentence=lambda s, e: spoken.append((s, e["emotion"], len(spoken))))
        self.assertEqual(
            spoken,
            [("Owls can turn their heads far.", "surprised", 0), ("They also fly silently!", "surprised", 1)],
        )

    def test_long_sentence_is_started_at_a_comma(self):
        brain = self.brain(
            [
                [
                    SETUP,
                    [
                        content(
                            outputTranscription={
                                "text": "Many owls can rotate their necks up to two hundred and "
                            }
                        ),
                        content(outputTranscription={"text": "seventy degrees, which helps them spot prey "}),
                        content(outputTranscription={"text": "without moving their body."}),
                        content(turnComplete=True),
                    ],
                ]
            ]
        )
        spoken = []
        brain.turn(text="fact?", on_sentence=lambda s, e: spoken.append(s))
        self.assertEqual(
            spoken,
            [
                "Many owls can rotate their necks up to two hundred and seventy degrees,",
                "which helps them spot prey without moving their body.",
            ],
        )

    def test_audio_is_streamed_while_the_user_talks(self):
        brain = self.brain(
            [
                [
                    SETUP,
                    [
                        content(inputTranscription={"text": "Hello"}),
                        content(outputTranscription={"text": "Hi there!"}),
                        content(turnComplete=True),
                    ],
                ]
            ]
        )
        brain.start_audio(48000)
        sock = self.sockets[0]
        self.assertIn("activityStart", sock.sent[-1]["realtimeInput"])  # sent before speech ends
        for _ in range(10):
            brain.feed_audio(np.zeros(960, dtype=np.int16))  # 10 x 20 ms
        streamed = [m for m in sock.sent if "audio" in m.get("realtimeInput", {})]
        self.assertGreater(len(streamed), 0)  # already on its way
        self.assertEqual(streamed[0]["realtimeInput"]["audio"]["mimeType"], "audio/pcm;rate=16000")
        got = brain.finish_audio()
        self.assertEqual(got["heard"], "Hello")
        self.assertEqual(got["say"], "Hi there!")
        self.assertIn("activityEnd", [k for m in sock.sent for k in m.get("realtimeInput", {})])

    def test_stream_failure_is_raised_when_the_turn_finishes(self):
        def broken(url, **kw):
            raise OSError("no network")

        brain = GeminiLiveBrain(load_config(CONFIG), "gkey", "Be an owl.", connect=broken)
        brain.start_audio(48000)  # must not raise inside the mic loop
        brain.feed_audio(np.zeros(960, dtype=np.int16))
        with self.assertRaises(OSError):
            brain.finish_audio()

    def test_thinking_setting_is_dropped_if_the_model_refuses_it(self):
        class Refusing(FakeSocket):
            def recv(self, timeout=None):
                if any("thinkingConfig" in m.get("setup", {}).get("generationConfig", {}) for m in self.sent):
                    raise ConnectionError("1007 invalid argument")
                return super().recv(timeout)

        socks = iter(
            [
                Refusing([SETUP]),
                FakeSocket(
                    [SETUP, [content(outputTranscription={"text": "Hi."}), content(turnComplete=True)]]
                ),
            ]
        )
        cfg = load_config(CONFIG)
        cfg["pipeline"]["gemini_live_thinking"] = "minimal"
        brain = GeminiLiveBrain(cfg, "gkey", "Be an owl.", connect=lambda url, **kw: next(socks))
        self.assertEqual(brain.turn(text="hi")["say"], "Hi.")
        self.assertEqual(brain.thinking, "")

    def test_gemini_voice_is_played_as_it_streams(self):
        pcm = b"\x01\x00" * 480
        brain = self.brain(
            [
                [
                    SETUP,
                    [
                        {
                            "toolCall": {
                                "functionCalls": [
                                    {
                                        "id": "c1",
                                        "name": "set_expression",
                                        "args": {"emotion": "happy", "sleep": False},
                                    }
                                ]
                            }
                        },
                        content(turnComplete=True),
                    ],
                    [
                        content(
                            modelTurn={
                                "parts": [
                                    {
                                        "inlineData": {
                                            "mimeType": "audio/pcm;rate=24000",
                                            "data": base64.b64encode(pcm).decode(),
                                        }
                                    }
                                ]
                            }
                        ),
                        content(outputTranscription={"text": "Hi there!"}),
                        content(
                            modelTurn={
                                "parts": [
                                    {
                                        "inlineData": {
                                            "mimeType": "audio/pcm;rate=24000",
                                            "data": base64.b64encode(pcm).decode(),
                                        }
                                    }
                                ]
                            }
                        ),
                        content(turnComplete=True),
                    ],
                ]
            ]
        )
        played = []
        got = brain.turn(text="hello", on_audio=lambda b, e: played.append((len(b), e["emotion"])))
        self.assertEqual(played, [(960, "happy"), (960, "happy")])
        self.assertEqual(got["say"], "Hi there!")
        setup = self.sockets[0].sent[0]["setup"]
        self.assertEqual(
            setup["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]["voiceName"],
            "Puck",
        )

    def test_mcp_tool_runs_on_the_pi(self):
        bridge = FakeBridge()
        brain = self.brain(
            [
                [
                    SETUP,
                    [
                        {
                            "toolCall": {
                                "functionCalls": [
                                    {"id": "t1", "name": "next_event", "args": {"topic": "demo"}}
                                ]
                            }
                        }
                    ],
                    [
                        content(outputTranscription={"text": "It's Friday at noon."}),
                        content(turnComplete=True),
                    ],
                ]
            ],
            bridge,
        )
        got = brain.turn(text="When is the demo?")
        self.assertEqual(got["say"], "It's Friday at noon.")
        self.assertEqual(got["emotion"], "explaining")  # no set_expression -> default
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
        self.assertEqual(len(self.sockets), 1)  # same session
        brain.opened -= 15 * 60  # pretend 15 minutes passed
        brain.turn(text="c")
        self.assertEqual(len(self.sockets), 2)
        self.assertTrue(self.sockets[0].closed)
        replay = self.sockets[1].sent[1]["clientContent"]
        self.assertEqual([t["parts"][0]["text"] for t in replay["turns"]], ["a", "One.", "b", "Two."])
        self.assertFalse(replay["turnComplete"])

    def test_sleep_forgets_and_closes(self):
        brain = self.brain(
            [[SETUP, [content(outputTranscription={"text": "Hi."}), content(turnComplete=True)]]]
        )
        brain.turn(text="hello")
        brain.reset()
        self.assertTrue(self.sockets[0].closed)
        self.assertEqual(brain.history, [])

    def test_silence_returns_none(self):
        brain = self.brain([[SETUP, [content(turnComplete=True)]]])
        self.assertIsNone(brain.turn(samples=np.zeros(4800, dtype=np.int16), rate=48000))
        self.assertEqual(brain.history, [])

    def test_broken_session_is_closed_and_error_raised(self):
        brain = self.brain([[SETUP, []]])  # server never finishes the turn
        with patch("owl.brain.gemini_live.TURN_TIMEOUT", 0.2), self.assertRaises(TimeoutError):
            brain.turn(text="hello")
        self.assertIsNone(brain.ws)


if __name__ == "__main__":
    unittest.main()
