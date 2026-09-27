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

from luma.core.interface.serial import spi
from luma.lcd.device import gc9a01
from PIL import Image, ImageDraw

SIZE = 240
SPI_HZ = 40_000_000
EYES = [
    dict(device=0, gpio_DC=25, gpio_RST=27),
    dict(device=1, gpio_DC=24, gpio_RST=22),
]
if "--one" in sys.argv:
    EYES = EYES[:1]


def make_device(cfg):
    serial = spi(port=0, bus_speed_hz=SPI_HZ, **cfg)
    return gc9a01(serial, width=SIZE, height=SIZE, rotate=0)


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
