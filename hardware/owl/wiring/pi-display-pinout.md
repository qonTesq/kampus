# Raspberry Pi 5 to the two GC9A01 eyes

Both 1.28" 240×240 round screens share the SPI0 bus and each has its own chip select, data/command and reset line. The numbers match `[display]` in `clients/owl/config/config.example.toml` (BCM GPIO numbering) and `clients/owl/scripts/eyes_test.py`.

| Display pin | Left eye | Right eye |
|---|---|---|
| VCC | 3.3 V, physical pin 1 | 3.3 V, physical pin 17 |
| GND | GND, physical pin 6 | GND, physical pin 9 |
| SCL / CLK / SCK | GPIO11 (SCLK), pin 23 | GPIO11 (SCLK), pin 23 (shared) |
| SDA / DIN / MOSI | GPIO10 (MOSI), pin 19 | GPIO10 (MOSI), pin 19 (shared) |
| CS | GPIO8 (CE0), pin 24 | GPIO7 (CE1), pin 26 |
| DC | GPIO25, pin 22 | GPIO24, pin 18 |
| RST / RES | GPIO27, pin 13 | GPIO22, pin 15 |
| BL / LED, if present | 3.3 V | 3.3 V |

- **3.3 V only.** Never connect the screens to 5 V. Check the labels on your exact module, especially VCC and BL; the controller name alone doesn't tell you its power needs.
- **Power the Pi off** before changing any wiring.
- No MISO wire is needed. On these SPI modules a pin labelled SDA is the SPI data input, not I²C.
- The software doesn't control the backlight; don't power it from a signal GPIO.
- Turn SPI on once: `sudo raspi-config nonint do_spi 0`, then reboot (`clients/owl/scripts/setup_pi.sh` does this).

## First wiring check

1. Wire the **left eye only** and run `uv run python scripts/eyes_test.py --one` from `clients/owl`.
2. Once it animates, wire the right eye and run `uv run python scripts/eyes_test.py`.
3. Then `uv run owl-realtime --demo` cycles every owl expression on both eyes.

| Symptom | Check |
|---|---|
| Black screen | VCC and GND, and that SPI is on (`ls /dev/spidev0.*`) |
| Garbled image | DC and RST wires; lower the SPI speed |
| Mirrored or rotated image | `display.left_rotation` / `right_rotation` in the config |

## Other connections

| Device | Connection |
|---|---|
| Logitech Brio 100 webcam (its built-in mic is the owl's mic) | USB, a blue USB 3 port |
| JBL Go 3 speaker | Bluetooth only (its USB-C port only charges it) |
| Power | Official Raspberry Pi 27 W USB-C supply |

## Arduino Uno version

[`../arduino/eyes_test/eyes_test.ino`](../arduino/eyes_test/eyes_test.ino) is the original eye test for an Arduino Uno (library: Adafruit GC9A01A). Its wiring is at the top of the sketch. The Uno's pins are 5 V, so it needs a level shifter (for example a 74HC4050) on SCL, SDA, CS, DC and RST. The Pi version doesn't use the Arduino.
