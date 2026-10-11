"""Kampus MCP integration pieces, offline."""
import asyncio
import json
import time
import types
import unittest

from owl.brain.pipeline import instructions
from owl.config import EXAMPLE_CONFIG as CONFIG
from owl.config import load_config
from owl.kampus.bridge import UNREACHABLE, MCPBridge, gemini_schema


def bridge_with(session, call_timeout=10):
    """An MCPBridge wired to a fake session, skipping the network connect."""
    b = MCPBridge.__new__(MCPBridge)
    b.loop = asyncio.new_event_loop()
    import threading
    threading.Thread(target=b.loop.run_forever, daemon=True).start()
    b.session, b.call_timeout = session, call_timeout
    b.tools = [types.SimpleNamespace(name="library_hours", description="Library hours",
                                     inputSchema={"type": "object", "properties": {}}),
               types.SimpleNamespace(name="my_deadlines", description="Deadlines",
                                     inputSchema={"type": "object"})]
    return b


class Session:
    def __init__(self, result=None, delay=0):
        self.result, self.delay = result, delay

    async def call_tool(self, name, args):
        await asyncio.sleep(self.delay)
        return self.result


class KampusBridgeTests(unittest.TestCase):
    def test_schema_is_made_gemini_safe_recursively(self):
        schema = {"$schema": "x", "type": "object", "title": "T", "additionalProperties": False,
                  "properties": {"start": {"type": "string", "format": "date-time", "title": "Start"},
                                 "limit": {"type": "integer", "default": 5},
                                 "nested": {"type": "array", "items": {"type": "object",
                                                                       "additionalProperties": False}}},
                  "required": ["start"]}
        self.assertEqual(gemini_schema(schema), {
            "type": "object", "required": ["start"],
            "properties": {"start": {"type": "string"}, "limit": {"type": "integer"},
                           "nested": {"type": "array", "items": {"type": "object"}}}})

    def test_structured_error_code_reaches_the_model(self):
        refusal = {"status": "error", "error": {"code": "permission_denied",
                                                "message": "guest device", "retryable": False}}
        result = types.SimpleNamespace(structuredContent=refusal, content=[], isError=True)
        out = json.loads(bridge_with(Session(result)).call("my_deadlines", {}))
        self.assertEqual(out["error"]["code"], "permission_denied")

    def test_slow_tool_times_out_with_short_error(self):
        b = bridge_with(Session(delay=5), call_timeout=0.3)
        start = time.monotonic()
        self.assertEqual(b.call("library_hours", {}), UNREACHABLE)
        self.assertLess(time.monotonic() - start, 1.5)

    def test_tool_outside_the_allowed_list_is_refused(self):
        out = json.loads(bridge_with(Session()).call("book_study_room", {}))
        self.assertEqual(out["error"]["code"], "unknown_tool")


class KampusPromptTests(unittest.TestCase):
    def test_prompt_has_rules_and_current_eastern_time(self):
        cfg = load_config(CONFIG)
        text = instructions(cfg, kampus=True)
        self.assertIn("permission_denied", text)
        self.assertIn("You can't book rooms", text)
        self.assertIn("Eastern time", text)
        self.assertNotIn("permission_denied", instructions(cfg, kampus=False))

    def test_only_demo_tools_are_allowed(self):
        cfg = load_config(CONFIG)
        self.assertEqual(sorted(cfg["mcp"]["allowed_tools"]),
                         ["find_study_room", "library_hours", "my_deadlines", "next_departures"])


if __name__ == "__main__":
    unittest.main()
