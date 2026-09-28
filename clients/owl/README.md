# Kampus Hardware — Talking Owl

A Raspberry Pi-powered talking owl for the Kampus project, with animated eyes, a microphone, and a Bluetooth speaker. The voice version connects to Gemini Live for spoken conversations and can use the Kampus MCP server to retrieve campus information.

## Demo

**[Watch the project demo](demo/WhatsApp%20Video%202026-09-27%20at%209.22.07%20AM.mp4)**

The original MP4 is included in this repository. Open the link to view the video file or download it for playback.

## Project versions

| Branch | Contents |
| --- | --- |
| [`main`](https://github.com/hienle0401/kampus_hardware_version/tree/main) | Project overview, demo video, and original Raspberry Pi and Arduino eye tests. |
| [`gemini-voice`](https://github.com/hienle0401/kampus_hardware_version/tree/gemini-voice) | Talking-owl application under `owl-pi/`, including Gemini Live voice, animated expressions, local wake listening, and campus-tool integration. |
| [`eyes`](https://github.com/hienle0401/kampus_hardware_version/tree/eyes) | Earlier development branch for the owl and display work. |

**Use `gemini-voice` for the talking-owl setup below.** The `owl-pi/` directory is not on `main`.

## What the owl does

- Animates two round screens with pupils, blinking, and different expressions.
- Listens locally for “Hey Owl” or “Hello Owl” to wake up.
- Supports spoken conversations using Gemini Live; the current branch configuration uses Gemini's Puck voice.
- Returns to sleep after a period without detected speech.
- Connects to campus tools for study-room availability, departures, and library hours when the MCP service and credentials are available.
- Provides a typed-input mode for situations where voice input is difficult.

The voice pipeline also includes optional ElevenLabs speech output and alternative AI backends. These require their own configuration and credentials. Campus-tool access depends on server permissions; the demo configuration does not provide unrestricted access or booking.

## Hardware

| Component | Purpose |
| --- | --- |
| Raspberry Pi 5 | Runs the Python application and controls the displays. |
| Two 1.28-inch GC9A01 240 × 240 SPI displays | Animated owl eyes. |
| Logitech USB webcam with microphone | Voice input. |
| JBL Go 3 Bluetooth speaker | Spoken responses. |
| Pi power supply and internet connection | Power and access to online voice and campus services. |

The Raspberry Pi version drives the screens directly; it does not require an Arduino. The original Arduino eye-test sketch remains in this repository.

## Run the talking owl

Run these commands on the Raspberry Pi with 64-bit Raspberry Pi OS:

```bash
git clone --branch gemini-voice https://github.com/hienle0401/kampus_hardware_version.git
cd kampus_hardware_version/owl-pi
bash setup_fresh_pi.sh
```

The setup script installs system and Python dependencies, enables SPI, downloads the local wake model, and creates `.env` if needed. It requires administrator access; coordinate system changes with the Pi owner.

1. Follow the [display wiring guide](https://github.com/hienle0401/kampus_hardware_version/blob/gemini-voice/owl-pi/README.md#2-connect-the-screens). Shut down and unplug the Pi before wiring; the repository's screens use **3.3 V**.
2. Add your `GEMINI_API_KEY` to the local `owl-pi/.env` file. Keep API keys and tokens out of Git.
3. Pair the Bluetooth speaker, select it as the default audio output, and connect the USB microphone.
4. Check `config.toml` for audio-device settings. If using campus tools, configure the MCP endpoint and authorization locally; `set_token.sh` provides the project's token setup helper.
5. Reboot after the initial setup, then return to `owl-pi/` and start the owl:

```bash
./run_owl.sh
```

Say **“Hey Owl”**, wait for the greeting, then ask a question. Wait for the owl to finish speaking before replying. Press **Ctrl+C** to stop.

The launcher checks the original demo hardware: a specific JBL Bluetooth address and a Brio webcam. For another speaker or microphone, update those checks and the audio configuration for your devices.

### Useful commands

Run from `owl-pi/` after setup:

```bash
# Start without waiting for a wake phrase
./run_owl.sh --no-wake

# Type messages instead of speaking
./run_owl.sh --text

# Test eye expressions without an online conversation
.venv/bin/python main.py --demo

# List audio devices and test recording/playback
.venv/bin/python main.py --list-audio
.venv/bin/python main.py --audio-test

# Run the offline software tests
.venv/bin/python -m unittest discover -s tests -v
```

Online conversation requires internet and a valid API key. Local wake listening and the display demo run locally. While the owl is awake and listening, the voice service receives microphone audio. The voice loop pauses listening while the owl speaks to reduce speaker echo.

## Repository guide

On `main`:

```text
demo/                  Project demo video
eyes_test.py           Original Raspberry Pi two-eye animation test
eyes_test/             Original Arduino eye-test sketch
HANDOFF.md             Early hardware setup and handoff notes
README.md              Project overview
```

On [`gemini-voice`](https://github.com/hienle0401/kampus_hardware_version/tree/gemini-voice/owl-pi), key application files include:

| File | Purpose |
| --- | --- |
| `owl-pi/voice_pipeline.py` | Voice application and backend selection. |
| `owl-pi/run_owl.sh` | Hardware checks and demo launcher. |
| `owl-pi/setup_fresh_pi.sh` | Fresh Raspberry Pi setup. |
| `owl-pi/config.toml` | Display, audio, wake, voice, and tool settings. |
| `owl-pi/.env.example` | Template for local credentials and overrides. |
| `owl-pi/mcp_bridge.py` | Connection to campus tools. |
| `owl-pi/tests/` | Offline software checks. |

The older handoff and detailed `owl-pi/README.md` contain earlier setup and status information, including OpenAI-based startup instructions. For the Gemini voice version, use the branch and launcher described above.
