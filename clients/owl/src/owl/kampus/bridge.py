"""Client-side bridge to a remote MCP server, for AI APIs without built-in MCP.

Gemini's API can't call a remote MCP server by itself, so the Pi does it: this
keeps one MCP session open (streamable HTTP) on a background event loop, lists
the server's tools once, and runs the tool calls the model asks for.
"""
import asyncio
import json
import threading

UNSUPPORTED_SCHEMA_KEYS = {"additionalProperties", "$schema", "format", "default", "title"}
UNREACHABLE = json.dumps({"status": "error", "error": {
    "code": "unreachable", "message": "Couldn't reach the campus system.", "retryable": True}})


def gemini_schema(schema):
    """Copy of a JSON schema without keys Gemini rejects (one bad declaration
    can break a whole Live session)."""
    if isinstance(schema, dict):
        return {k: gemini_schema(v) for k, v in schema.items() if k not in UNSUPPORTED_SCHEMA_KEYS}
    if isinstance(schema, list):
        return [gemini_schema(v) for v in schema]
    return schema


class MCPBridge:
    def __init__(self, url, authorization="", allowed_tools=(), timeout=30, call_timeout=10):
        self.url, self.timeout, self.call_timeout = url, timeout, call_timeout
        self.headers = {"Authorization": f"Bearer {authorization}"} if authorization else {}
        self.allowed = set(allowed_tools or ())
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self.loop.run_forever, name="mcp", daemon=True)
        self.thread.start()
        self.ready = threading.Event()
        self.stop_event = None
        self.session = None
        self.error = None
        self.tools = []
        asyncio.run_coroutine_threadsafe(self._run(), self.loop)
        if not self.ready.wait(timeout) or self.error:
            raise RuntimeError(f"MCP server unavailable: {self.error or 'timed out'}")

    async def _run(self):
        from mcp import ClientSession
        from mcp.client.streamable_http import streamablehttp_client
        self.stop_event = asyncio.Event()
        try:
            async with (
                streamablehttp_client(self.url, headers=self.headers) as (read, write, _),
                ClientSession(read, write) as session,
            ):
                await session.initialize()
                listed = (await session.list_tools()).tools
                self.tools = [t for t in listed if not self.allowed or t.name in self.allowed]
                self.session = session
                self.ready.set()
                await self.stop_event.wait()
        except Exception as exc:
            self.error = exc
            self.ready.set()

    def declarations(self):
        """The allowed tools as Gemini function declarations."""
        return [{"name": t.name, "description": t.description or t.name,
                 "parametersJsonSchema": gemini_schema(t.inputSchema)} for t in self.tools]

    def call(self, name, arguments):
        """Run one tool and return its result as text for the model.

        Prefers the server's structuredContent ({"status": ..., "data"/"error": ...})
        so error codes such as permission_denied reach the model unchanged.
        Tool errors are normal results (isError); a hang or transport failure
        after call_timeout seconds becomes a short "unreachable" error."""
        if name not in {t.name for t in self.tools}:
            return json.dumps({"status": "error", "error": {"code": "unknown_tool", "message": name}})
        future = asyncio.run_coroutine_threadsafe(
            self.session.call_tool(name, arguments or {}), self.loop)
        try:
            result = future.result(self.call_timeout)
        except Exception:
            future.cancel()
            return UNREACHABLE
        if getattr(result, "structuredContent", None):
            return json.dumps(result.structuredContent)
        text = "\n".join(getattr(c, "text", "") for c in result.content).strip()
        return ("Error: " if result.isError else "") + (text or "(no output)")

    def close(self):
        if self.stop_event:
            self.loop.call_soon_threadsafe(self.stop_event.set)
