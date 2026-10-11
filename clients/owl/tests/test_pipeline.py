"""ChatGPT + ElevenLabs pipeline, with MCP, without network or API credit."""
import json
import unittest
from typing import ClassVar
from unittest.mock import patch

import requests

from owl.brain.pipeline import Brain, parse_tagged
from owl.config import EXAMPLE_CONFIG as CONFIG
from owl.config import load_config
from owl.voice.elevenlabs import Voice


class Resp:
    def __init__(self, status=200, js=None, body=b""):
        self.status_code, self._js, self._body = status, js, body

    def raise_for_status(self):
        if self.status_code >= 400:
            raise requests.HTTPError(f"{self.status_code}", response=self)

    def json(self): return self._js

    def iter_content(self, n):
        for i in range(0, len(self._body), 777):
            yield self._body[i:i + 777]


def answer(text, rid="resp_1"):
    return Resp(js={"id": rid, "output": [
        {"type": "mcp_list_tools", "tools": []},
        {"type": "mcp_call", "name": "events", "output": "..."},
        {"type": "message", "content": [{"type": "output_text", "text": text}]}]})


def config(mcp=True):
    cfg = load_config(CONFIG)
    if mcp:
        cfg["mcp"].update(server_url="https://mcp.test/mcp", authorization="tok")
    return cfg


class BrainTests(unittest.TestCase):
    def test_mcp_tool_json_reply_and_turn_chaining(self):
        sent = []
        replies = iter([answer(json.dumps({"say": "Hi!", "emotion": "happy", "sleep": False}), "r1"),
                        answer(json.dumps({"say": "Bye.", "emotion": "sleepy", "sleep": True}), "r2")])
        brain = Brain(config(), "key")
        with patch.object(brain.http, "post", side_effect=lambda url, **kw: (sent.append(kw["json"]), next(replies))[1]):
            first = brain.reply("hello")
            second = brain.reply("goodbye")
        self.assertEqual(first, {"say": "Hi!", "emotion": "happy", "sleep": False})
        self.assertTrue(second["sleep"])
        self.assertEqual(sent[0]["tools"][0]["type"], "mcp")
        self.assertEqual(sent[0]["tools"][0]["require_approval"], "never")
        self.assertEqual(sent[0]["text"]["format"]["type"], "json_schema")
        self.assertNotIn("previous_response_id", sent[0])
        self.assertEqual(sent[1]["previous_response_id"], "r1")
        self.assertIn("kampus", sent[0]["instructions"])

    def test_falls_back_to_tags_when_json_format_is_refused_with_mcp(self):
        sent = []
        replies = iter([Resp(400, {"error": {"message": "format not supported with tools"}}),
                        answer("[surprised] The library opens at eight!")])
        brain = Brain(config(), "key")
        with patch.object(brain.http, "post", side_effect=lambda url, **kw: (sent.append(kw["json"]), next(replies))[1]):
            got = brain.reply("when does the library open")
        self.assertEqual(got, {"say": "The library opens at eight!", "emotion": "surprised", "sleep": False})
        self.assertNotIn("text", sent[1])
        self.assertIn("square brackets", sent[1]["instructions"])
        self.assertTrue(brain.tagged)

    def test_without_mcp_no_tools_and_errors_are_not_retried(self):
        sent = []
        brain = Brain(config(mcp=False), "key")
        with (
            patch.object(brain.http, "post", side_effect=lambda url, **kw: (sent.append(kw["json"]), Resp(400, {}))[1]),
            self.assertRaises(requests.HTTPError),
        ):
            brain.reply("hi")
        self.assertEqual(len(sent), 1)
        self.assertNotIn("tools", sent[0])

    def test_sleep_forgets_the_conversation(self):
        brain = Brain(config(), "key")
        brain.previous = "r9"
        brain.reset()
        self.assertIsNone(brain.previous)

    def test_tag_parsing(self):
        self.assertEqual(parse_tagged("[love] Aww, thanks! [sleep]"),
                         {"say": "Aww, thanks!", "emotion": "love", "sleep": True})
        self.assertEqual(parse_tagged("No tag here."),
                         {"say": "No tag here.", "emotion": "explaining", "sleep": False})


def gemini_reply(obj):
    return Resp(js={"candidates": [{"content": {"role": "model", "parts": [
        {"text": "thinking...", "thought": True}, {"text": json.dumps(obj)}]}}]})


class FakeTool:
    name, description = "next_event", "Next campus event"
    inputSchema: ClassVar[dict] = {"type": "object", "properties": {"topic": {"type": "string"}}}


class FakeBridge:
    tools: ClassVar[list] = [FakeTool()]

    def declarations(self):
        from owl.kampus.bridge import gemini_schema
        return [{"name": t.name, "description": t.description,
                 "parametersJsonSchema": gemini_schema(t.inputSchema)} for t in self.tools]

    def __init__(self): self.calls = []

    def call(self, name, args):
        self.calls.append((name, args))
        return "Hackathon demo, Friday 12:00"


class GeminiBrainTests(unittest.TestCase):
    def brain(self, bridge=None):
        from owl.brain.pipeline import GeminiBrain
        cfg = config(mcp=False)
        return GeminiBrain(cfg, "gkey", bridge)

    def test_audio_turn_hears_and_replies_in_one_request(self):
        import numpy as np
        sent = []
        brain = self.brain()
        reply = gemini_reply({"heard": "Hi owl", "say": "Hello!", "emotion": "happy", "sleep": False})
        with patch.object(brain.http, "post", side_effect=lambda url, **kw: (sent.append((url, kw["json"])), reply)[1]):
            got = brain.turn(samples=np.zeros(4800, dtype=np.int16), rate=48000)
        self.assertEqual(got, {"heard": "Hi owl", "say": "Hello!", "emotion": "happy", "sleep": False})
        url, body = sent[0]
        self.assertTrue(url.endswith("/models/gemini-3.5-flash-lite:generateContent"))
        self.assertEqual(brain.http.headers["x-goog-api-key"], "gkey")
        self.assertEqual(body["contents"][-1]["parts"][0]["inlineData"]["mimeType"], "audio/wav")
        self.assertEqual(body["generationConfig"]["responseMimeType"], "application/json")
        self.assertEqual(body["generationConfig"]["thinkingConfig"], {"thinkingLevel": "low"})
        self.assertNotIn("tools", body)
        # memory keeps the transcript as text, never the audio
        self.assertEqual(brain.history, [{"role": "user", "parts": [{"text": "Hi owl"}]},
                                         {"role": "model", "parts": [{"text": "Hello!"}]}])

    def test_mcp_tool_call_round_trip(self):
        sent = []
        bridge = FakeBridge()
        brain = self.brain(bridge)
        call = Resp(js={"candidates": [{"content": {"role": "model", "parts": [
            {"functionCall": {"name": "next_event", "args": {"topic": "hackathon"}},
             "thoughtSignature": "sig"}]}}]})
        final = gemini_reply({"heard": "", "say": "The demo is Friday at noon.", "emotion": "explaining",
                              "sleep": False})
        replies = iter([call, final])
        with patch.object(brain.http, "post",
                          side_effect=lambda url, **kw: (sent.append(json.loads(json.dumps(kw["json"]))), next(replies))[1]):
            got = brain.turn(text="When is the demo?")
        self.assertEqual(got["say"], "The demo is Friday at noon.")
        self.assertEqual(got["heard"], "When is the demo?")
        self.assertEqual(bridge.calls, [("next_event", {"topic": "hackathon"})])
        decl = sent[0]["tools"][0]["functionDeclarations"][0]
        self.assertEqual(decl["name"], "next_event")
        self.assertEqual(decl["parametersJsonSchema"]["properties"]["topic"]["type"], "string")
        followup = sent[1]["contents"]
        self.assertEqual(followup[-2]["parts"][0]["thoughtSignature"], "sig")   # call echoed back as-is
        self.assertEqual(followup[-1]["parts"][0]["functionResponse"],
                         {"name": "next_event", "response": {"result": "Hackathon demo, Friday 12:00"}})
        self.assertEqual(len(brain.history), 2)                                 # tool steps not kept

    def test_silence_returns_none_and_is_not_remembered(self):
        import numpy as np
        brain = self.brain()
        reply = gemini_reply({"heard": "", "say": "", "emotion": "normal", "sleep": False})
        with patch.object(brain.http, "post", return_value=reply):
            self.assertIsNone(brain.turn(samples=np.zeros(480, dtype=np.int16), rate=48000))
        self.assertEqual(brain.history, [])

    def test_blocked_answer_is_an_error(self):
        brain = self.brain()
        with (
            patch.object(brain.http, "post", return_value=Resp(js={"promptFeedback": {"blockReason": "SAFETY"}})),
            self.assertRaisesRegex(RuntimeError, "no answer"),
        ):
            brain.turn(text="hi")


class VoiceTests(unittest.TestCase):
    def test_streams_pcm_in_whole_samples_and_counts_characters(self):
        fed = []

        class Audio:
            def feed_pcm24(self, pcm): fed.append(pcm)

        voice = Voice(config(), "key", "EXAVITQu4vr4xnSDxMaL")
        with patch.object(voice.http, "post", return_value=Resp(body=b"\x01\x00" * 1000)) as post:
            voice.speak("Hello there.", Audio())
        self.assertTrue(all(len(c) % 2 == 0 for c in fed))
        self.assertEqual(sum(map(len, fed)), 2000)
        self.assertEqual(voice.chars_used, 12)
        self.assertEqual(post.call_args.kwargs["params"]["output_format"], "pcm_24000")
        self.assertEqual(post.call_args.kwargs["json"]["model_id"], "eleven_flash_v2_5")


if __name__ == "__main__":
    unittest.main()
