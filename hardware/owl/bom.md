# Owl: bill of materials

The parts in the working prototype. Wiring: [`wiring/pi-display-pinout.md`](wiring/pi-display-pinout.md).

| Part | Qty | Link | Notes |
|---|---|---|---|
| Raspberry Pi 5 (8 GB) | 1 | [raspberrypi.com](https://www.raspberrypi.com/products/raspberry-pi-5/) | Runs the owl client on 64-bit Raspberry Pi OS (Debian 13 "trixie") |
| Raspberry Pi 27 W USB-C power supply | 1 | [raspberrypi.com](https://www.raspberrypi.com/products/27w-power-supply/) | Weaker supplies cause throttling (`vcgencmd get_throttled` must be `0x0`) |
| microSD card | 1 | | Raspberry Pi OS |
| 1.28" 240×240 round GC9A01 SPI display (Teyleten Robot) | 2 | | The eyes, 3.3 V only. The pack had three; two are used. |
| Logitech Brio 100 USB webcam | 1 | | Only its built-in microphone is used; the camera stays off |
| JBL Go 3 Bluetooth speaker | 1 | | Bluetooth only; its USB-C port only charges it |
| 3D-printed owl shell | 1 | | CAD and print files to come in `cad/` and `print/` |

For the optional Arduino Uno eye test, add an Arduino Uno and a 74HC4050 level shifter (or 1 kΩ / 2 kΩ resistor dividers).
