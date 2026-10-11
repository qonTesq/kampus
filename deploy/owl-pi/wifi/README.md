# Hotspot password tools (SD card)

Two small laptop scripts for the Pi's SD card, with the card in the laptop as `D:`. They read the `network-config` file that Raspberry Pi Imager writes, which stores the Wi-Fi password as a 64-character key derived from the network name and password, never the password itself.

- `check_wifi_password.py`: type a password (hidden) and see whether the card was set up with it. Prints only `MATCH` or `NO MATCH`.
- `set_wifi_password.py`: type a new hotspot password twice (hidden) and save its key to the card. The Pi uses it on its next start.

```bash
python check_wifi_password.py
python set_wifi_password.py
```

Nothing is printed or saved except the result. Never write the password itself into any file in this repository.
