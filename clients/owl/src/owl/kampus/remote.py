"""The Kampus MCP server as a remote tool that OpenAI's APIs call themselves."""


def mcp_tool(cfg):
    """Remote MCP server as a Realtime tool, or None when no server is configured."""
    m = cfg.get("mcp", {})
    if not m.get("server_url"):
        return None
    tool = {
        "type": "mcp",
        "server_label": m.get("server_label", "kampus"),
        "server_url": m["server_url"],
        "require_approval": "never",
    }
    for key in ("authorization", "allowed_tools", "server_description"):
        if m.get(key):
            tool[key] = m[key]
    return tool
