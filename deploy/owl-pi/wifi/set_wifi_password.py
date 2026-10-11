"""Save a new hotspot password onto the Pi's SD card (card in the laptop as D:).

Type the password twice (hidden). It is stored the same way Raspberry Pi Imager
does, as a 64-character key computed from the network name and password, so the
plain password is never written to the card. Nothing is printed except the result.

  python set_wifi_password.py
"""
import getpass
import hashlib
import re

PATH = r"D:\network-config"
text = open(PATH, encoding="utf-8").read()
ssid = re.search(r'"([^"]+)":\s*\n\s*password:', text).group(1)
first = getpass.getpass(f'New password for hotspot "{ssid}" (hidden): ')
second = getpass.getpass("Type it again: ")
if first != second:
    raise SystemExit("The two passwords don't match. Nothing saved.")
if not 8 <= len(first) <= 63:
    raise SystemExit("Hotspot passwords must be 8-63 characters. Nothing saved.")
key = hashlib.pbkdf2_hmac("sha1", first.encode(), ssid.encode(), 4096, 32).hex()
new_text, count = re.subn(r'(password:\s*)"[^"]*"', rf'\1"{key}"', text, count=1)
if count != 1:
    raise SystemExit("Couldn't find the password line. Nothing saved.")
with open(PATH, "w", encoding="utf-8", newline="\n") as f:
    f.write(new_text)
print(f'Saved. The Pi will use this password for "{ssid}" on its next start.')
