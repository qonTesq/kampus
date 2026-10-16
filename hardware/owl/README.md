# Owl: hardware

The physical build of the talking owl mascot.

## Layout

```
hardware/owl/
├── cad/          source models (STEP, Fusion/FreeCAD files)
├── print/        printable STL/3MF files and print settings
├── wiring/       wiring diagrams and pin maps
├── photos/       build photos used in the docs
├── arduino/      the original Arduino Uno eye test
└── bom.md        bill of materials (parts, quantities, links)
```

`cad/`, `print/` and `photos/` are still empty: the shell's CAD and print files haven't been added yet.

## Parts (summary)

| Part | Role |
|---|---|
| Raspberry Pi 5 (8 GB) | Runs the owl client |
| 2× GC9A01 1.28" round displays | Eyes (SPI) |
| USB webcam | Microphone (camera off) |
| Bluetooth speaker | Voice |

The full list is in [`bom.md`](bom.md), and the pin map in [`wiring/pi-display-pinout.md`](wiring/pi-display-pinout.md).

## Verified on hardware

| Phase | Check | Result |
|---|---|---|
| 1 | Pi boots, SSH, OS updated, health and 3-minute stress test | Max 73.6 °C, no throttling |
| 2 | Eyes wired; `owl-realtime --demo` runs every expression | 2026-09-26: 25 fps, no errors, 39.5 °C |
| 3 | Speaker (JBL Go 3) paired, trusted, default output | 2026-09-26: owl volume 0.8, system volume 62% |
| 4 | Mic (Logitech Brio 100 webcam): `owl-realtime --audio-test` record and playback | 2026-09-26 |
| 5 | Eyes, mic and speaker together: `owl-soak-test`, 10 minutes | 2026-09-26: 25 fps, 0 audio glitches, max 43.3 °C, no throttling |
| 6 | Talking owl: Gemini Live, Kampus tools over MCP, your-turn blink | Rehearsed live for the demo |

## Safety rules

- **3.3 V only** on the screens. Never connect them to 5 V.
- **Power the Pi off** before changing any wiring.
- Shut down with `sudo shutdown -h now` and wait for the green LED to stop blinking before unplugging; pulling power can corrupt the SD card.
- Use the official 27 W supply; anything weaker throttles the Pi.

## Large files

CAD and print files are stored with **Git LFS** (see the root `.gitattributes`). Before adding them, install it once:

```bash
git lfs install
```

The software side lives in [`clients/owl/`](../../clients/owl/), and running it on the Pi in [`deploy/owl-pi/`](../../deploy/owl-pi/).
