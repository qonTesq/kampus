"""Loading config/config.toml and the .env overrides."""
import os
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]          # clients/owl
EXAMPLE_CONFIG = ROOT / "config" / "config.example.toml"
LOCAL_CONFIG = ROOT / "config" / "config.toml"      # untracked copy of the example
ENV_FILE = ROOT / ".env"


def default_config():
    """config/config.toml if it exists, else the committed example."""
    return LOCAL_CONFIG if LOCAL_CONFIG.is_file() else EXAMPLE_CONFIG


def load_config(path=None):
    with open(path or default_config(), "rb") as handle: cfg = tomllib.load(handle)
    if cfg["assistant"]["sleep_after_seconds"] <= 0:
        raise ValueError("sleep_after_seconds must be positive")
    if not cfg["assistant"]["wake_phrases"]:
        raise ValueError("At least one wake phrase is required")
    if not 1 <= cfg["display"]["fps"] <= 60:
        raise ValueError("display fps must be between 1 and 60")
    if cfg["display"]["pixel_size"] not in (1, 2, 3, 4, 5, 6, 8):
        raise ValueError("pixel_size must be 1, 2, 3, 4, 5, 6 or 8")
    if not 0 < cfg["audio"]["volume"] <= 1:
        raise ValueError("audio volume must be between 0 and 1")
    if cfg["audio"]["output_channels"] not in (1, 2):
        raise ValueError("Use one or two output channels")
    if not 0 <= cfg["audio"]["mic_channel"] < cfg["audio"]["input_channels"]:
        raise ValueError("mic_channel must exist in input_channels")
    pins = [cfg["display"][p] for p in ("left_dc_gpio", "right_dc_gpio", "left_reset_gpio", "right_reset_gpio")]
    if len(set(pins)) != 4 or any(p in (7, 8, 9, 10, 11) for p in pins):
        raise ValueError("DC/reset pins must be distinct and not use the SPI0 pins")
    if cfg["display"]["left_ce"] == cfg["display"]["right_ce"]:
        raise ValueError("Screens need different chip-select lines")
    for name in ("left_rotation", "right_rotation"):
        if cfg["display"][name] not in (0, 90, 180, 270):
            raise ValueError("Display rotation must be 0, 90, 180 or 270")
    model = Path(cfg["wake"]["model_path"])
    cfg["wake"]["model_path"] = str(model if model.is_absolute() else ROOT / model)
    return cfg

def apply_env(cfg):
    """Settings in .env (or the environment) override config.toml."""
    env = {k: v.strip() for k, v in os.environ.items()}
    if env.get("OPENAI_REALTIME_MODEL"):
        cfg["assistant"]["model"] = env["OPENAI_REALTIME_MODEL"]
    for name, key in (("AUDIO_INPUT_DEVICE", "input_device"), ("AUDIO_OUTPUT_DEVICE", "output_device")):
        if env.get(name):
            value = env[name]
            cfg["audio"][key] = int(value) if value.lstrip("-").isdigit() else value
    mcp = cfg.setdefault("mcp", {})
    for name, key in (("MCP_SERVER_URL", "server_url"), ("MCP_AUTHORIZATION", "authorization")):
        if env.get(name):
            mcp[key] = env[name]
    if env.get("MCP_ALLOWED_TOOLS"):
        mcp["allowed_tools"] = [t.strip() for t in env["MCP_ALLOWED_TOOLS"].split(",") if t.strip()]
    return cfg
