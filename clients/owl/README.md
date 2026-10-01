# Owl: the talking campus mascot

The owl is a voice client for kampus: a Raspberry Pi 5 inside a 3D-printed mascot, with two round-screen eyes, a microphone and a Bluetooth speaker. Anyone can walk up and ask about study rooms, trains or library hours. It connects to kampus as a **public device**, so it answers campus questions but never reads personal data.

- **Listening and thinking:** Gemini Live
- **Speaking:** ElevenLabs
- **Campus answers:** the kampus MCP server, through the owl's own device credential

## Layout

```
clients/owl/
├── pyproject.toml          Python project (uv), with ruff and pytest settings
├── uv.lock
├── src/owl/
│   ├── hardware/           eyes, displays, audio devices, power and health checks
│   ├── brain/              Gemini Live session, turn detection, the voice pipeline
│   ├── voice/              ElevenLabs text-to-speech streaming
│   └── kampus/             the kampus MCP bridge
├── tests/
├── config/
│   └── config.example.toml  copy to config.toml (untracked) and edit
└── scripts/                 setup_pi.sh, run_owl.sh
```

The physical build (CAD, wiring, parts list) lives in [`hardware/owl/`](../../hardware/owl/), and the Pi service setup in [`deploy/owl-pi/`](../../deploy/owl-pi/).

## Secrets and local files

Keys (`GEMINI_API_KEY`, `ELEVENLABS_API_KEY`, `MCP_AUTHORIZATION`) go in `clients/owl/.env`, which is never committed. Commit `config/config.example.toml`, never `config/config.toml`.

## Checks

```bash
cd clients/owl
uv sync
uv run ruff check . && uv run ruff format --check .
uv run pytest
```

CI runs the same checks whenever files under `clients/owl/` change.
