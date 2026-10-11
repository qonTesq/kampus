"""Live check of owl.kampus.bridge against a throwaway local MCP server.

Starts a tiny MCP server on 127.0.0.1:8765 (streamable HTTP, bearer-token
protected), connects the bridge, lists tools, and calls one. No cloud.

  uv run python tests/live/check_mcp_bridge.py
"""
import subprocess
import sys
import time

SERVER = r'''
from mcp.server.fastmcp import FastMCP
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.responses import JSONResponse
import uvicorn

mcp = FastMCP("kampus-test", host="127.0.0.1", port=8765)

@mcp.tool()
def next_event(topic: str = "") -> str:
    """Next campus event, optionally about a topic."""
    return f"Next {topic or 'campus'} event: hackathon demo, Friday 12:00"

@mcp.tool()
def secret_tool() -> str:
    """Should be hidden by allowed_tools."""
    return "hidden"

class Auth(BaseHTTPMiddleware):
    async def dispatch(self, request, call_next):
        if request.headers.get("authorization") != "Bearer test-token":
            return JSONResponse({"error": "unauthorized"}, status_code=401)
        return await call_next(request)

app = mcp.streamable_http_app()
app.add_middleware(Auth)
uvicorn.run(app, host="127.0.0.1", port=8765, log_level="warning")
'''


def main():
    server = subprocess.Popen([sys.executable, "-c", SERVER])
    try:
        time.sleep(3)
        from owl.kampus.bridge import MCPBridge
        url = "http://127.0.0.1:8765/mcp"
        try:
            MCPBridge(url, "wrong-token", timeout=10)
            print("FAIL: wrong token was accepted")
            return 1
        except RuntimeError:
            print("ok  wrong token rejected")
        bridge = MCPBridge(url, "test-token", allowed_tools=["next_event"])
        names = [t.name for t in bridge.tools]
        assert names == ["next_event"], names
        print("ok  tools listed, allowed_tools filter applied:", names)
        result = bridge.call("next_event", {"topic": "hackathon"})
        assert "Friday 12:00" in result, result
        print("ok  tool call result:", result)
        assert "unknown_tool" in bridge.call("secret_tool", {})
        print("ok  disallowed tool refused")
        bridge.close()
        print("MCP bridge: all checks passed")
        return 0
    finally:
        server.terminate()


if __name__ == "__main__":
    sys.exit(main())
