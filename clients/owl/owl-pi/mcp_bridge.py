"""Client-side bridge to a remote MCP server, for AI APIs without built-in MCP.

Gemini's API can't call a remote MCP server by itself, so the Pi does it: this
keeps one MCP session open (streamable HTTP) on a background event loop, lists
the server's tools once, and runs the tool calls the model asks for.
"""
import asyncio
import threading


class MCPBridge:
    def __init__(self, url, authorization="", allowed_tools=(), timeout=30):
        self.url, self.timeout = url, timeout
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
            async with streamablehttp_client(self.url, headers=self.headers) as (read, write, _):
                async with ClientSession(read, write) as session:
                    await session.initialize()
                    listed = (await session.list_tools()).tools
                    self.tools = [t for t in listed if not self.allowed or t.name in self.allowed]
                    self.session = session
                    self.ready.set()
                    await self.stop_event.wait()
        except Exception as exc:
            self.error = exc
            self.ready.set()

    def call(self, name, arguments):
        """Run one tool; return its text result (errors come back as text too)."""
        if name not in {t.name for t in self.tools}:
            return f"Error: unknown tool {name}"
        future = asyncio.run_coroutine_threadsafe(
            self.session.call_tool(name, arguments or {}), self.loop)
        try:
            result = future.result(self.timeout)
        except Exception as exc:
            return f"Error: tool {name} failed ({type(exc).__name__})"
        text = "\n".join(getattr(c, "text", "") for c in result.content).strip()
        return ("Error: " if result.isError else "") + (text or "(no output)")

    def close(self):
        if self.stop_event:
            self.loop.call_soon_threadsafe(self.stop_event.set)
