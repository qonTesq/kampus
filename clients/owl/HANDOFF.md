# Kampus hardware — handoff

Robot with animated eyes on two round screens, a mic (Logitech webcam) and a speaker (JBL Go 3), driven by a Raspberry Pi 5.

## Status

| Phase | Task | Status |
|---|---|---|
| 1 | Pi boots, SSH, OS updated, health + 3-min stress test | Done (max 73.6°C, no throttling) |
| 2 | **Eyes:** GC9A01 screens wired, `owl-pi/main.py --demo` runs all expressions | Done 2026-09-26 (25 fps, no errors, 39.5°C) |
| 3 | Speaker: JBL Go 3 paired + trusted (18:90:67:87:94:0C), default output | Done 2026-09-26 (owl volume 0.8, system 62%) |
| 4 | Mic: Logitech Brio 100 webcam, `main.py --audio-test` record + playback | Done 2026-09-26 |
| 5 | Eyes + mic + speaker together, `owl-pi/soak_test.py` 10 min | Done 2026-09-26 (25 fps, 0 audio glitches, max 43.3°C, no throttling) |
| 6 | Voice loop: `owl-pi/voice_pipeline.py` (ChatGPT brain + ElevenLabs voice), or `main.py` (OpenAI Realtime) | Built, tested offline with simulated cloud; **needs API keys** |

## Hardware

- Raspberry Pi 5 (8 GB), Raspberry Pi OS 64-bit (Debian 13 "trixie")
- 3× 1.28" round 240×240 GC9A01 SPI screens (Teyleten Robot); two are used as eyes
- Logitech webcam: its built-in mic is the robot's mic
- JBL Go 3: Bluetooth only; its USB-C port is for charging

## Connecting to the Pi

1. Turn on the iPhone hotspot `kampus-hotspot` and keep its Personal Hotspot screen open.
2. Connect the laptop to `kampus-hotspot`. Windows tends to switch back to eduroam on its own.
3. Run `ssh <your-user>@172.20.10.8`. If the IP changes, try `hienle.local`.

Campus Wi-Fi (eduroam) won't work for this: it blocks laptop-to-Pi connections.

## Files

- `eyes_test.py`: two-eye animation for the Pi (Python, with a built-in GC9A01 driver using `spidev` and `lgpio`). The wiring table is at the top of the file.
- `eyes_test/eyes_test.ino`: the same animation for an Arduino Uno. Needs a level shifter, because the Uno's pins are 5V.

## Phase 2: getting the eyes running on the Pi

1. Ask the Pi owner to turn on SPI. This needs admin rights: `sudo raspi-config nonint do_spi 0 && sudo reboot`
2. **Power the Pi off**, then wire the **left eye only**, following the table in `eyes_test.py`.
3. On the Pi:
   ```bash
   python3 -m venv ~/kampus-env --system-site-packages
   source ~/kampus-env/bin/activate
   python eyes_test.py --one
   ```
4. Once one eye works, wire the right eye and run `python eyes_test.py`.

Troubleshooting:
- **Black screen:** check VCC and GND, and check that SPI is on (`ls /dev/spidev0.*`).
- **Garbled image:** check the DC and RST wires, or lower `SPI_HZ` to `20_000_000`.
- **Mirrored or rotated image:** change `rotate=` in the script.

## Rules (don't break the baseline)

- **3.3V only** on the screens. Never connect them to 5V.
- **Power the Pi off** before changing any wiring.
- Shut down with `sudo shutdown -h now`, then unplug. Don't pull the power while the Pi is running.
- Work on the **`eyes` branch**. Merge into `main` only after the other person has looked at it.
- Don't change system settings or install system packages. Ask the Pi owner.
- Commit and push to the `eyes` branch often. GitHub is the only backup of this work.
