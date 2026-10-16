# Restart after power loss

Use this checklist whenever the Raspberry Pi 5 loses power (cable pulled, battery died). Nothing needs to be reinstalled: the code, the speaker pairing and the wake-phrase model all survive a power cut.

## 1. Power the Pi back on

- Plug the 27 W USB-C power supply back into the Pi. It has no power button and starts by itself.
- Wait about a minute; the green LED flickers while it boots.
  - Red LED only, no green: the SD card wasn't read. Reseat the card.
  - No light at all: check the power cable and supply.

## 2. Turn on the other hardware

- **Speaker (JBL Go 3):** press its power button. It reconnects by itself (already paired and trusted).
- **Webcam (Logitech Brio 100, the mic):** stays plugged into a blue USB port; nothing to do.
- **Phone hotspot:** Settings → Personal Hotspot → "Allow Others to Join" on, "Maximize Compatibility" on. Keep this screen open until the Pi joins (iPhones hide the hotspot when the screen locks).

## 3. Connect the laptop to the Pi

Connect the laptop to the hotspot (not eduroam: campus Wi-Fi blocks laptop-to-Pi connections), open a terminal and log in. The laptop's SSH key is installed, so no password is needed.

```bash
ssh <user>@<pi-address>
```

If that address doesn't answer, try `ssh <user>@<pi-hostname>.local`.

## 4. Go to the project folder (on the Pi)

```bash
cd ~/kampus/clients/owl
```

## 5. Restore the speaker volume

The speaker often comes back too quiet after reconnecting. Set it to the tested level (62%):

```bash
wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.62
```

## 6. Run what you need (Ctrl+C stops any of them)

```bash
uv run owl-realtime --demo      # eye animation only: every expression, no mic, no internet
uv run owl-realtime --audio-test   # records 3 s from the webcam, plays it on the speaker
uv run owl-echo-test            # speak, hear it back 3 s after you stop (5 rounds)
uv run owl-soak-test            # 10 minutes of eyes + mic + speaker, prints health
scripts/run_owl.sh              # the talking owl: say "Hey Owl"
uv run owl-realtime             # backup owl, OpenAI voice only (needs OPENAI_API_KEY in .env)
```

## Quick health check (optional)

```bash
vcgencmd get_throttled                    # must say throttled=0x0; anything else = weak power supply
vcgencmd measure_temp                     # fine below 80 °C
bluetoothctl info <speaker-address> | grep Connected   # must say "Connected: yes"
bluetoothctl connect <speaker-address>    # reconnect the speaker if it isn't
lsusb | grep -i brio                      # webcam mic detected
```

## Before unplugging: always shut down first

Pulling the power while the Pi is running can corrupt the SD card.

```bash
sudo shutdown -h now
```

Wait until the green LED stops blinking, then unplug.
