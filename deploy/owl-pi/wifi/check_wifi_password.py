"""Check whether the hotspot password you type matches the one saved on the Pi's SD card.

The SD card stores the Wi-Fi password as a 64-character key computed from the
network name and password. This recomputes that key from what you type (hidden)
and compares. Nothing is printed or saved except "MATCH" / "NO MATCH".

  python check_wifi_password.py        (with the SD card in the laptop as D:)
"""
import getpass
import hashlib
import re

text = open(r"D:\network-config", encoding="utf-8").read()
ssid = re.search(r'"([^"]+)":\s*\n\s*password:', text).group(1)
saved = re.search(r'password:\s*"([0-9a-fA-F]{64})"', text).group(1).lower()
typed = getpass.getpass(f'Type the password of the hotspot "{ssid}" (hidden), then Enter: ')
key = hashlib.pbkdf2_hmac("sha1", typed.encode(), ssid.encode(), 4096, 32).hex()
print("MATCH: the Pi has the right password." if key == saved
      else "NO MATCH: the Pi was set up with a different password than this one.")
