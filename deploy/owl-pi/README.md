# Running the owl on a Raspberry Pi

Setup, operation and recovery for the owl client ([`clients/owl/`](../../clients/owl/)) on its Raspberry Pi 5. Wire the screens first: [`hardware/owl/wiring/`](../../hardware/owl/wiring/).

| Guide | When |
|---|---|
| This page | First-time setup, everyday running, troubleshooting |
| [demo-guide.md](demo-guide.md) | The day before and the day of a demo |
| [restart-after-power-loss.md](restart-after-power-loss.md) | The Pi lost power |
| [`wifi/`](wifi/) | Checking or changing the hotspot password on the Pi's SD card, from a laptop |

## First-time setup

On a fresh 64-bit Raspberry Pi OS (Debian 13 "trixie") card:

```bash
git clone https://github.com/qonTesq/kampus.git ~/kampus
bash ~/kampus/clients/owl/scripts/setup_pi.sh
```

`setup_pi.sh` installs the system packages (audio, SPI, lgpio, OpenCV), turns on SPI, installs [uv](https://docs.astral.sh/uv/), creates `clients/owl/.venv` from `uv.lock`, downloads the offline wake-phrase model, creates `.env` and `config/config.toml` from their examples, and runs the offline tests. It needs admin rights, so check with the Pi's owner before running it.

Then, from `~/kampus/clients/owl`:

1. Save the keys without showing them on screen: `scripts/set_key.sh GEMINI_API_KEY` (and `ELEVENLABS_API_KEY` if `[pipeline] voice = "elevenlabs"`).
2. Save the owl's Kampus device token: `scripts/set_token.sh`.
3. Pair the Bluetooth speaker from the desktop's Bluetooth menu, trust it and make it the default output (A2DP profile).
4. Reboot once: `sudo reboot`.
5. Check the eyes: `uv run owl-realtime --demo` should cycle every expression on both screens.

## Kampus connection

Set `MCP_SERVER_URL` in `clients/owl/.env` to the school's Kampus URL, and `MCP_AUTHORIZATION` to the owl's device token (`scripts/set_token.sh` writes it). The owl calls only the tools in `[mcp] allowed_tools`. If the server can't be reached at start-up, the owl keeps running without campus tools.

## Run the owl

```bash
cd ~/kampus/clients/owl
scripts/run_owl.sh            # say "Hey Owl"
scripts/run_owl.sh --no-wake  # start awake
scripts/run_owl.sh --text     # type questions on the laptop
```

The launcher checks the keys, internet, Bluetooth speaker (it reconnects it and sets the volume), webcam mic, power and temperature, stops any owl process that is already running, and starts `owl`. It expects the demo hardware: a Logitech Brio webcam, and the speaker's Bluetooth address in `OWL_SPEAKER_MAC` (the default is the team's JBL Go 3).

**Start on boot:** not set up yet. A systemd unit (`owl.service`) running `clients/owl/scripts/run_owl.sh` as a non-root user belongs in this folder.

## Choose and test the microphone and speaker

```bash
uv run owl-realtime --list-audio
nano config/config.toml
```

Under `[audio]`, set `input_device` to the USB microphone's number or name. `output_device = -1` uses the default output; if that misses Bluetooth, use the listed `pulse` device. Numbers can change after reconnecting devices. You can also set `AUDIO_INPUT_DEVICE` / `AUDIO_OUTPUT_DEVICE` in `.env`.

```bash
uv run owl-realtime --audio-test   # record 3 s, play it back; nothing is saved
uv run owl-echo-test               # speak, hear it back 3 s after you stop
uv run owl-soak-test               # eyes + mic + speaker for 10 minutes
```

If the microphone needs stereo, set `input_channels = 2` and pick `mic_channel`. If a device rejects 48000 Hz, use a rate it supports; the owl resamples.

## Secrets

Keys stay in `clients/owl/.env` on the Pi, readable only by the owl's user (`chmod 600`; the scripts set it). Never paste a key into chat or commit it.

## Troubleshooting

- **Black screens:** check power, ground, CS, DC and reset wiring, and that SPI is on (`ls /dev/spidev0.*`). The two screens need different CS lines.
- **Wrong colors:** try `display.bgr = false`; some modules need a different `invert`.
- **Garbled image or glitches:** shorten the wires and lower `display.spi_hz` (for example `8000000`).
- **Mirrored or rotated eyes:** change `display.left_rotation` / `right_rotation`.
- **Permission errors:** run as the normal desktop user in the `spi`, `gpio`, `audio` and `video` groups, not as root (Bluetooth audio is per-user).
- **GPIO backend error:** use an up-to-date Raspberry Pi OS with its `python3-lgpio` and `python3-gpiozero` packages (the venv sees them through system site packages).
- **No sound:** reconnect the speaker, make it the default output, repeat `--list-audio` and `--audio-test`.
- **The owl hears itself:** lower the volume, move the speaker away from the webcam, or raise `audio.echo_guard_seconds` to `1.2` or more.
- **Wake phrase missed:** speak clearly near the webcam, try "Hello Owl", or start with `--no-wake`.
- **"Cloud request failed":** usually the hotspot dropped. Check the phone and ask again; one failed turn never stops the owl.

## Limits

The eyes run in their own thread and send only the changed rectangle of each screen over SPI. The owl is half-duplex on purpose: it doesn't listen while it speaks, because a Bluetooth speaker and a webcam mic have no echo cancellation. While awake and listening, microphone audio goes to the configured AI service. The owl doesn't save recordings to disk.
