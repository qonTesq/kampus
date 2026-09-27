"""Two-eye animation test for 2x GC9A01 1.28" round displays on Raspberry Pi 5.

Wiring (shared SPI0 bus, separate CS / DC / RST per eye):
  Display pin | Left eye            | Right eye
  VCC         | 3.3V   (pin 1)      | 3.3V   (pin 17)
  GND         | GND    (pin 6)      | GND    (pin 9)
  SCL         | GPIO11 (pin 23)     | GPIO11 (pin 23)
  SDA         | GPIO10 (pin 19)     | GPIO10 (pin 19)
  CS          | GPIO8  (pin 24,CE0) | GPIO7  (pin 26,CE1)
  DC          | GPIO25 (pin 22)     | GPIO24 (pin 18)
  RST         | GPIO27 (pin 13)     | GPIO22 (pin 15)
  BL (if any) | 3.3V                | 3.3V

Run:  python eyes_test.py          (Ctrl+C to stop)
      python eyes_test.py --one    (left eye only, for first wiring check)
"""
import random
import sys
import time

import lgpio
import numpy as np
import spidev
from PIL import Image, ImageDraw

SIZE = 240
SPI_HZ = 40_000_000
EYES = [
    dict(device=0, dc=25, rst=27),
    dict(device=1, dc=24, rst=22),
]
if "--one" in sys.argv:
    EYES = EYES[:1]

# GC9A01 power-on sequence (vendor init, as used by Waveshare/Adafruit drivers).
INIT = [
    (0xEF, []), (0xEB, [0x14]), (0xFE, []), (0xEF, []), (0xEB, [0x14]),
    (0x84, [0x40]), (0x85, [0xFF]), (0x86, [0xFF]), (0x87, [0xFF]),
    (0x88, [0x0A]), (0x89, [0x21]), (0x8A, [0x00]), (0x8B, [0x80]),
    (0x8C, [0x01]), (0x8D, [0x01]), (0x8E, [0xFF]), (0x8F, [0xFF]),
    (0xB6, [0x00, 0x20]), (0x36, [0x08]), (0x3A, [0x05]),
    (0x90, [0x08, 0x08, 0x08, 0x08]), (0xBD, [0x06]), (0xBC, [0x00]),
    (0xFF, [0x60, 0x01, 0x04]), (0xC3, [0x13]), (0xC4, [0x13]), (0xC9, [0x22]),
    (0xBE, [0x11]), (0xE1, [0x10, 0x0E]), (0xDF, [0x21, 0x0C, 0x02]),
    (0xF0, [0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]),
    (0xF1, [0x43, 0x70, 0x72, 0x36, 0x37, 0x6F]),
    (0xF2, [0x45, 0x09, 0x08, 0x08, 0x26, 0x2A]),
    (0xF3, [0x43, 0x70, 0x72, 0x36, 0x37, 0x6F]),
    (0xED, [0x1B, 0x0B]), (0xAE, [0x77]), (0xCD, [0x63]),
    (0x70, [0x07, 0x07, 0x04, 0x0E, 0x0F, 0x09, 0x07, 0x08, 0x03]),
    (0xE8, [0x34]),
    (0x62, [0x18, 0x0D, 0x71, 0xED, 0x70, 0x70, 0x18, 0x0F, 0x71, 0xEF, 0x70, 0x70]),
    (0x63, [0x18, 0x11, 0x71, 0xF1, 0x70, 0x70, 0x18, 0x13, 0x71, 0xF3, 0x70, 0x70]),
    (0x64, [0x28, 0x29, 0xF1, 0x01, 0xF1, 0x00, 0x07]),
    (0x66, [0x3C, 0x00, 0xCD, 0x67, 0x45, 0x45, 0x10, 0x00, 0x00, 0x00]),
    (0x67, [0x00, 0x3C, 0x00, 0x00, 0x00, 0x01, 0x54, 0x10, 0x32, 0x98]),
    (0x74, [0x10, 0x85, 0x80, 0x00, 0x00, 0x4E, 0x00]),
    (0x98, [0x3E, 0x07]), (0x35, []), (0x21, []),
]


def open_gpiochip():
    """Find the RP1 chip that drives the 40-pin header (number varies by kernel)."""
    for n in (0, 4, 10, 11, 12, 13):
        try:
            h = lgpio.gpiochip_open(n)
        except lgpio.error:
            continue
        if "rp1" in lgpio.gpio_get_chip_info(h)[3]:
            return h
        lgpio.gpiochip_close(h)
    raise RuntimeError("40-pin header GPIO chip not found")


class GC9A01:
    def __init__(self, chip, device, dc, rst):
        self.chip, self.dc = chip, dc
        self.spi = spidev.SpiDev()
        self.spi.open(0, device)
        self.spi.max_speed_hz = SPI_HZ
        self.spi.mode = 0
        lgpio.gpio_claim_output(chip, dc, 0)
        lgpio.gpio_claim_output(chip, rst, 1)
        for level, pause in ((1, 0.01), (0, 0.01), (1, 0.12)):  # hardware reset
            lgpio.gpio_write(chip, rst, level)
            time.sleep(pause)
        for cmd, data in INIT:
            self.command(cmd, data)
        self.command(0x11)            # sleep out
        time.sleep(0.12)
        self.command(0x29)            # display on
        time.sleep(0.02)

    def command(self, cmd, data=()):
        lgpio.gpio_write(self.chip, self.dc, 0)
        self.spi.writebytes([cmd])
        if data:
            lgpio.gpio_write(self.chip, self.dc, 1)
            self.spi.writebytes2(bytes(data))

    def display(self, img):
        a = np.asarray(img, dtype=np.uint16)
        rgb565 = ((a[..., 0] & 0xF8) << 8) | ((a[..., 1] & 0xFC) << 3) | (a[..., 2] >> 3)
        self.command(0x2A, [0, 0, 0, SIZE - 1])   # column range
        self.command(0x2B, [0, 0, 0, SIZE - 1])   # row range
        self.command(0x2C)                        # memory write
        lgpio.gpio_write(self.chip, self.dc, 1)
        self.spi.writebytes2(rgb565.astype(">u2").tobytes())


CHIP = open_gpiochip()


def make_device(cfg):
    return GC9A01(CHIP, **cfg)


def draw_eye(px, py, lid):
    """px, py: pupil offset in pixels; lid: 0.0 open .. 1.0 closed."""
    img = Image.new("RGB", (SIZE, SIZE), "black")
    d = ImageDraw.Draw(img)
    c = SIZE // 2
    d.ellipse((4, 4, SIZE - 4, SIZE - 4), fill=(235, 235, 235))          # sclera
    ix, iy = c + px, c + py
    d.ellipse((ix - 62, iy - 62, ix + 62, iy + 62), fill=(30, 120, 200))  # iris
    d.ellipse((ix - 28, iy - 28, ix + 28, iy + 28), fill="black")         # pupil
    d.ellipse((ix - 30, iy - 38, ix - 12, iy - 20), fill="white")         # glint
    if lid > 0:
        h = int(c * lid)
        d.rectangle((0, 0, SIZE, h), fill=(20, 20, 20))                   # upper lid
        d.rectangle((0, SIZE - h, SIZE, SIZE), fill=(20, 20, 20))         # lower lid
    return img


def main():
    devices = [make_device(cfg) for cfg in EYES]
    px = py = 0.0
    tx = ty = 0.0
    next_look = time.time()
    next_blink = time.time() + random.uniform(2, 5)
    blink_start = None
    frames, t0 = 0, time.time()

    while True:
        now = time.time()
        if now >= next_look:
            tx, ty = random.uniform(-45, 45), random.uniform(-35, 35)
            next_look = now + random.uniform(0.6, 2.5)
        px += (tx - px) * 0.25
        py += (ty - py) * 0.25

        lid = 0.0
        if blink_start is None and now >= next_blink:
            blink_start = now
        if blink_start is not None:
            t = (now - blink_start) / 0.25          # 250 ms blink
            if t >= 1:
                blink_start = None
                next_blink = now + random.uniform(2, 6)
            else:
                lid = 1 - abs(2 * t - 1)

        frame = draw_eye(int(px), int(py), lid)
        for dev in devices:
            dev.display(frame)

        frames += 1
        if now - t0 >= 5:
            print(f"{frames / (now - t0):.1f} fps")
            frames, t0 = 0, now


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
