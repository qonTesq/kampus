"""GC9A01 SPI output for Pi 5. GPIO uses gpiozero's lgpio backend.

Initialization sequence adapted from Adafruit_CircuitPython_GC9A01A:
Copyright (c) 2021 Tyler Crumpton; (c) 2025 Tim C for Adafruit Industries.
MIT license; see clients/owl/THIRD_PARTY_LICENSE.txt.
"""
import os
import time

import numpy as np

INIT = bytes.fromhex('''
FE00 EF00 B6020000 360148 3A0105 C30113 C40113 C90122
F00645090808262A F10643707236376F F20645090808262A F30643707236376F
660A3C00CD67454510000000 670A003C0000000154103298
740710858000004E00 98023E07 3500 2100 118078 298014
''')

def rgb565(image):
    rgb = np.asarray(image.convert("RGB"), dtype=np.uint16)
    return (((rgb[..., 0] & 0xF8) << 8) |
            ((rgb[..., 1] & 0xFC) << 3) | (rgb[..., 2] >> 3)).astype(">u2")

class PiDisplays:
    def __init__(self, cfg):
        import spidev
        os.environ.setdefault("GPIOZERO_PIN_FACTORY", "lgpio")
        from gpiozero import OutputDevice
        self.cfg = cfg
        self.spies = []
        self.resets = []
        self.previous = [None, None]
        self.dcs = []
        try:
            for name in ("left_dc_gpio", "right_dc_gpio"):
                self.dcs.append(OutputDevice(cfg[name], initial_value=False))
            for ce, pin in ((cfg["left_ce"], cfg["left_reset_gpio"]),
                            (cfg["right_ce"], cfg["right_reset_gpio"])):
                spi = spidev.SpiDev()
                spi.open(cfg["spi_bus"], ce)
                spi.max_speed_hz = cfg["spi_hz"]
                spi.mode = 0
                self.spies.append(spi)
                self.resets.append(OutputDevice(pin, initial_value=True))
            for i, reset in enumerate(self.resets):
                reset.off(); time.sleep(.02)
                reset.on(); time.sleep(.12)
                pos = 0
                while pos < len(INIT):
                    command, count = INIT[pos:pos+2]; pos += 2
                    size = count & 0x7F
                    self.command(i, command, INIT[pos:pos+size]); pos += size
                    if count & 0x80:
                        wait = INIT[pos]; pos += 1
                        time.sleep((500 if wait == 255 else wait)/1000)
                # RGB byte order and inversion vary between module variants.
                self.command(i, 0x36, bytes([0x40 | (8 if cfg["bgr"] else 0)]))
                self.command(i, 0x21 if cfg["invert"] else 0x20)
        except Exception:
            self.close()
            raise

    def command(self, i, command, data=b""):
        self.dcs[i].off()
        self.spies[i].writebytes([command])
        if data:
            self.dcs[i].on()
            self.spies[i].writebytes2(data)

    def show(self, frames):
        for i, image in enumerate(frames):
            rotation = self.cfg["left_rotation" if i == 0 else "right_rotation"]
            if rotation: image = image.rotate(rotation, expand=False)
            pixels = rgb565(image)
            old = self.previous[i]
            if old is None:
                x0, y0, x1, y1 = 0, 0, 239, 239
            else:
                yy, xx = np.nonzero(pixels != old)
                if not len(xx): continue
                x0, x1 = int(xx.min()), int(xx.max())
                y0, y1 = int(yy.min()), int(yy.max())
            self.command(i, 0x2A, bytes([0, x0, 0, x1]))
            self.command(i, 0x2B, bytes([0, y0, 0, y1]))
            self.command(i, 0x2C)
            self.dcs[i].on()
            data = pixels[y0:y1+1, x0:x1+1].tobytes()
            self.spies[i].writebytes2(data)
            self.previous[i] = pixels

    def close(self):
        for spi in self.spies:
            try: spi.close()
            except Exception: pass
        for pin in self.resets:
            try: pin.close()
            except Exception: pass
        for pin in self.dcs:
            try: pin.close()
            except Exception: pass

class NullDisplays:
    def show(self, frames): pass
    def close(self): pass
