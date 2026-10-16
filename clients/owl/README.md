# Owl: the talking campus mascot

The owl is a voice client for Kampus: a Raspberry Pi 5 inside a 3D-printed mascot, with two round-screen eyes, a microphone and a Bluetooth speaker. Anyone can walk up and ask about study rooms, trains or library hours. It connects to Kampus as a **public device**, so it answers campus questions but never reads personal data.

- **Listening and thinking:** Gemini Live
- **Speaking:** Gemini Live's own voice by default, or ElevenLabs
- **Campus answers:** the Kampus MCP server, through the owl's own device credential

![The owl's eye expressions](docs/owl-expressions.png)

## What it does

- Sleeps with its eyes closed and listens **locally** (offline Vosk model) for "Hey Owl" or "Hello Owl". Nothing goes to the cloud while it sleeps.
- When awake, detects where you start and stop talking on the Pi, streams your words to Gemini Live and answers out loud. It is half-duplex: the mic is off while it speaks, and a blink means "your turn".
- Picks an eye expression for every reply (happy, surprised, explaining, love, money, confused, sleepy, playful angry).
- Calls the Kampus tools for study rooms, departures and library hours. A `permission_denied` result is a normal answer: "That's personal, so open it in your school's approved assistant."
- Goes back to sleep after five minutes without speech, or when you say goodbye. Each wake starts a fresh conversation.

## Layout

```
clients/owl/
├── pyproject.toml          Python project (uv), with ruff and pytest settings
├── uv.lock
├── src/owl/
│   ├── config.py           loads config/config.toml and the .env overrides
│   ├── state.py            conversation state shared by the eyes and the brain
│   ├── main.py             owl-realtime: OpenAI Realtime owl, eye demo, audio tools
│   ├── hardware/           eyes, displays, audio devices, power and health checks
│   ├── brain/              wake listening, turn detection, Gemini Live, the voice pipeline
│   ├── voice/              ElevenLabs text-to-speech streaming
│   └── kampus/             the kampus MCP bridge, remote tool and prompt rules
├── tests/                  offline tests (tests/live/ needs a network or API credit)
├── config/
│   └── config.example.toml  copy to config.toml (untracked) and edit
├── scripts/                 setup_pi.sh, run_owl.sh, set_key.sh, set_token.sh, eyes_test.py
└── docs/                    images used in this README
```

The physical build (wiring, parts list, build status) lives in [`hardware/owl/`](../../hardware/owl/), and running the owl on the Pi (setup, demo day, recovery) in [`deploy/owl-pi/`](../../deploy/owl-pi/).

## Commands

After `uv sync` (or `scripts/setup_pi.sh` on the Pi):

| Command | What it does |
|---|---|
| `uv run owl` | The talking owl (wake phrase, Gemini Live, Kampus tools). On the Pi, start it with `scripts/run_owl.sh`, which checks the hardware first. |
| `uv run owl --no-wake` | Start awake, without the wake phrase |
| `uv run owl --text` | Type instead of speaking; the owl still talks and shows its eyes |
| `uv run owl --no-display` | Run without the eye screens |
| `uv run owl-realtime --demo` | Eyes only: cycles every expression, no mic, network or key |
| `uv run owl-realtime --list-audio` | List audio devices for `[audio]` in the config |
| `uv run owl-realtime --audio-test` | Record three seconds, play it back (local only) |
| `uv run owl-realtime --download-wake-model` | Download the offline wake-phrase model (about 40 MB) to `models/` |
| `uv run owl-realtime --contact-sheet eyes.png` | Render every expression into one image |
| `uv run owl-realtime` | Backup owl on the OpenAI Realtime API (needs `OPENAI_API_KEY`) |
| `uv run owl-echo-test` | Speak and hear yourself back 3 s later: checks turn detection, no cloud |
| `uv run owl-soak-test` | Eyes, mic and speaker together for 10 minutes, with health readings |

## Configuration

`config/config.example.toml` holds every setting with comments. To change one, copy it to `config/config.toml` (git-ignored) and edit the copy. If there's no `config.toml`, the owl uses the example.

- `[pipeline] brain`: `"gemini-live"` (default), `"gemini"` (Flash-Lite, one request per turn) or `"openai"` (ChatGPT).
- `[pipeline] voice`: `"gemini"` (Gemini Live's own voice, fastest) or `"elevenlabs"`.
- `[mcp]`: the Kampus server and the tools the owl may call. The URL and token usually come from `.env`.
- `[audio]`, `[display]`: devices, rates, GPIO pins and screen rotation. See [`hardware/owl/wiring/`](../../hardware/owl/wiring/).

## Secrets and local files

Keys (`GEMINI_API_KEY`, `ELEVENLABS_API_KEY`, `MCP_AUTHORIZATION`) go in `clients/owl/.env`, which is never committed. Copy `.env.example` to start; on the Pi, `scripts/set_key.sh <NAME>` and `scripts/set_token.sh` save a key without showing it on screen. Commit `config/config.example.toml`, never `config/config.toml`.

## Checks

```bash
cd clients/owl
uv sync
uv run ruff check . && uv run ruff format --check .
uv run pytest
```

CI runs the same checks whenever files under `clients/owl/` change. The offline tests need no Pi, microphone, network or API key. `tests/live/` holds two manual checks that pytest doesn't collect: `check_mcp_bridge.py` (runs a local MCP server) and `measure_latency.py` (spends a few hundred ElevenLabs characters).

## Credits

The GC9A01 initialization sequence in `src/owl/hardware/display.py` is adapted from [Adafruit_CircuitPython_GC9A01A](https://github.com/adafruit/Adafruit_CircuitPython_GC9A01A) (MIT, see [THIRD_PARTY_LICENSE.txt](THIRD_PARTY_LICENSE.txt)).
