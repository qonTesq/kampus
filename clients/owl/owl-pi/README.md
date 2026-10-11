# Your talking owl — Raspberry Pi 5

This is a **Python project for Raspberry Pi OS**, not an Arduino sketch. It drives two GC9A01 240×240 SPI screens directly from the Pi, listens through a USB microphone, and speaks through your paired Bluetooth speaker. The Arduino is no longer needed for this version.

The owl has yellow pixel eyes on black, moving pupils, synchronized blinks, and normal, happy, surprised, confused, love, money, playful angry, sleepy, and explaining expressions. Sleeping eyes stay closed with **no Zzz**. Expressions are chosen for each reply from the conversation; they are not a measurement of somebody's feelings.

After five minutes without detected human speech, it finishes any current reply and sleeps. Say **“Hey Owl”** or **“Hello Owl”** to wake it. Sleeping keeps the Pi on and listens locally for the wake phrase; the cloud voice connection is closed. Each wake starts a fresh conversation.

## 1. What you need

- Raspberry Pi 5 with current **64-bit Raspberry Pi OS Desktop**, internet, and a suitable Pi 5 power supply.
- Two GC9A01 **SPI** display modules with separate CS pins.
- USB microphone, USB camera (optional), and Bluetooth speaker.
- An OpenAI API key with API billing enabled and access to the configured Realtime model. API usage has charges; a ChatGPT subscription does not provide API credit.

The code defaults to `gpt-realtime-2.1`; change `assistant.model` in `config.toml` if your account uses another compatible Realtime model. Keep the key in the Pi's local `.env` file, never in a chat or shared code.

## 2. Connect the screens

Shut down and unplug the Pi before wiring. Disconnect the screens from the Arduino first. **Pi GPIO signals are 3.3 V.** Check the printed labels and documentation for your exact display module, particularly VCC and BL; the GC9A01 controller name alone does not establish its power requirements.

| Display connection | Left screen | Right screen |
|---|---|---|
| GND | Pi physical pin 6 (ground) | Pi ground, such as physical pin 9 |
| DIN / SDA / MOSI | Physical pin **19**, BCM GPIO10 | Same physical pin **19** |
| CLK / SCL / SCK | Physical pin **23**, BCM GPIO11 | Same physical pin **23** |
| CS | Physical pin **24**, BCM GPIO8 / CE0 | Physical pin **26**, BCM GPIO7 / CE1 |
| DC | Physical pin **22**, BCM GPIO25 | Physical pin **18**, BCM GPIO24 |
| RST / RES | Physical pin **13**, BCM GPIO27 | Physical pin **15**, BCM GPIO22 |
| VCC | 3.3 V, physical pin 1 | 3.3 V, physical pin 17 |
| BL / LED, if present | 3.3 V per repository wiring | 3.3 V per repository wiring |

This matches this repository's Teyleten Robot module wiring: **3.3 V only; never 5 V**. Do not power a backlight directly from a signal GPIO. The software does not control BL. No MISO wire is needed. On these SPI displays, a pin labelled SDA is the SPI data input, not an I²C connection. This GitHub version uses separate left and right DC pins to match the existing `eyes_test.py`; the earlier ZIP used a shared DC pin.

Plug the microphone and optional camera into USB. Pair the speaker from the Pi desktop's Bluetooth menu, then select it as the sound output. Choose its high-quality/A2DP playback profile if available. Use the USB microphone as input.

## 3. Install the project on the Pi

Clone the `eyes` branch on your other laptop to edit or back up the files. To run the hardware, open a terminal **on the Pi** and clone it there too:

```bash
git clone --branch eyes https://github.com/hienle0401/kampus_hardware_version.git
cd kampus_hardware_version/owl-pi
```

For an existing clone, first save your work, then switch to `eyes` and pull its changes. All commands below assume you are inside this repository's `owl-pi` folder.

The repository handoff requires the Pi owner's approval before installing system packages or changing system settings. Have the owner handle the installer and SPI configuration below. These commands require admin access; cloning, editing, and running the offline tests do not change system settings.

```bash
bash setup_pi.sh
sudo raspi-config
```

In Raspberry Pi Configuration, enable **SPI** under Interface Options. Reboot if requested, then open the terminal again:

```bash
cd ~/kampus_hardware_version/owl-pi
source .venv/bin/activate
python main.py --demo
```

Both eyes should cycle through the expressions. This demo uses no microphone, camera, or online service. Press **Ctrl+C** to stop. Test this before setting up the AI.

## 4. Choose and test the microphone and speaker

With the Bluetooth speaker connected and selected in the desktop sound menu:

```bash
python main.py --list-audio
nano config.toml
```

Under `[audio]`, set `input_device` to your USB microphone's listed number. `output_device = -1` uses the default output; if that misses Bluetooth, use the listed `pulse` device or the device routed to your Bluetooth speaker. Numbers are local to your Pi and can change after reconnecting devices.

Example only — use your own microphone number:

```toml
[audio]
input_device = 2
output_device = "pulse"
```

Edit only those existing lines, leaving the other audio settings in place. Save in nano with **Ctrl+O**, Enter, then **Ctrl+X**. Test:

```bash
python main.py --audio-test
```

Speak for three seconds. You should hear your recording through the Bluetooth speaker. This test stays local and does not save a recording. Fix audio selection before continuing. If the microphone needs stereo, set `input_channels = 2` and select `mic_channel = 0` or `1`. If a device rejects 48000 Hz, use a sample rate it supports; the app resamples it for the AI.

## 5. Add your API key and wake model

The installer creates `.env` from `.env.example` without overwriting an existing file.

```bash
nano .env
```

Set its line to your actual key locally:

```text
OPENAI_API_KEY=your_actual_key_here
```

Save and close, then run:

```bash
python main.py --download-wake-model
python main.py
```

The first command downloads the official Vosk small English speech model, approximately 40 MB. Once downloaded, wake listening works locally. The conversation itself needs internet.

The eyes begin asleep. Say **“Hey Owl”**, wait for the greeting to finish, then ask a question. Wait until the owl finishes speaking before replying. Press **Ctrl+C** to stop the program.

For later starts:

```bash
cd ~/kampus_hardware_version/owl-pi
source .venv/bin/activate
python main.py
```

## 6. Try the expressions

- “Thanks, that helped!” — happy.
- “Explain how rainbows form.” — explaining.
- “Show me your love eyes.” — love hearts.
- “Let's talk about saving money.” — money may be appropriate.
- “Pretend to be a grumpy owl.” — playful angry.
- Give an unclear question — confused while asking for more details.

The model chooses one expression for each answer, so these are examples, not fixed command mappings. Blinking and gaze run independently while it listens and speaks. It does not continuously switch expressions word by word. Sleep starts after 300 seconds without detected speech, after any reply finishes; background speech or noise misclassified as speech can delay it.

## 7. Optional USB camera

The camera is off by default. To allow explicitly requested photos, change this existing setting in `config.toml`:

```toml
[camera]
enabled = true
```

Restart the app, wake it, and ask **“What do you see through the camera?”** A requested capture sends one image to the AI. There is no continuous video stream or face recognition. Camera selection defaults to device `0`; change it if your USB camera appears under a different index.

## Settings you can change

In `config.toml`:

| Setting | What it changes |
|---|---|
| `assistant.sleep_after_seconds = 300` | Five-minute sleep timer |
| `assistant.wake_phrases` | Local wake phrases; default model recognizes English |
| `assistant.language = "English"` | Requested spoken reply language |
| `display.fps = 25` | Target animation rate, not a measured hardware guarantee |
| `display.pixel_size = 4` | Pixel-art block size; smaller is less blocky |
| `display.left_rotation` / `right_rotation` | 0, 90, 180, or 270 degrees |
| `audio.echo_guard_seconds = 0.8` | Extra listening pause after Bluetooth playback |

Changing the spoken language does not change the English wake recognizer. Other wake languages require an appropriate Vosk model and `wake.model_path` change.

## Troubleshooting

- **Black screens:** check module power/backlight, common ground, CS, DC and reset wiring, and that SPI is enabled. The two screens must use different CS lines. Confirm this is a GC9A01 module, not a different controller.
- **Wrong colors:** try `display.bgr = false`; some modules need a different `invert` setting. Restart after changes.
- **Display noise or glitches:** shorten wires and reduce `display.spi_hz` to `8000000`. Lower SPI speeds can reduce frame rate.
- **Permission errors:** use the regular Pi desktop user with access to the `spi`, `gpio`, `audio`, and `video` groups. After changing group membership, log out and in again. Avoid running the app as root because desktop Bluetooth audio may be unavailable there.
- **GPIO backend error:** use an up-to-date Raspberry Pi OS installation and its `python3-lgpio` / `python3-gpiozero` packages; this app uses the Pi 5-compatible lgpio backend.
- **No speaker audio:** reconnect and select the Bluetooth speaker, repeat `--list-audio` and `--audio-test`, and use the desktop sound server output rather than an HDMI hardware device.
- **Owl hears itself:** lower speaker volume, move the USB microphone farther away, or increase `echo_guard_seconds` to `1.2` or more.
- **Wake phrase is missed:** test the USB mic, speak clearly near it, and try “Hello Owl.” Local speech recognition can false-trigger or miss a phrase; it is not speaker identification.
- **API authentication, billing, or model error:** check the local key, available credit and model access. Errors return to local wake listening instead of repeatedly reconnecting. Say the wake phrase to retry after fixing the issue.
- **Test voice without screens:** `python main.py --no-display`.

## Practical limits and verification

The animation runs in its own thread, renders a small pixel grid, and sends only the bounding rectangle of changed pixels to each screen. This avoids clearing and redrawing the whole screen every frame. Actual smoothness depends on SPI speed, wiring, and Pi load; the two SPI screens are updated sequentially.

This first version deliberately pauses microphone transmission while preparing and playing a reply, plus a short Bluetooth delay. **You cannot interrupt it while it speaks.** This avoids speaker echo without pretending to provide acoustic echo cancellation. Online response latency also depends on the network and service.

The cloud connection starts after a local wake phrase. While awake and listening, microphone audio is sent to OpenAI; selected camera images are sent only through the optional visual-request path. The app does not write conversation recordings or photos to disk. Service-side data handling follows your API account's policies. Awake sessions can incur API charges. At 55 minutes, an idle conversation returns to sleep to stay below the service's session limit; wake it again to begin a new conversation.

The code and renderer have been tested offline, including sleep timing, expression rendering, and simulated voice events. **It has not been tested on your physical Pi, displays, microphone, Bluetooth speaker, or a live paid API session.** Hardware checks above are necessary before relying on it unattended.

Run offline tests with:

```bash
python -m unittest discover -s tests -v
```

## References

- [OpenAI Realtime conversations and tool calling](https://developers.openai.com/api/docs/guides/realtime-conversations)
- [OpenAI Realtime WebSocket connection](https://developers.openai.com/api/docs/guides/realtime-websocket)
- [Raspberry Pi hardware documentation](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html)
- [Waveshare 1.28-inch LCD reference module](https://www.waveshare.com/wiki/1.28inch_LCD_Module) — check your actual module before applying its electrical specifications.
- [Vosk models](https://alphacephei.com/vosk/models)
- [Adafruit GC9A01 initialization source](https://github.com/adafruit/Adafruit_CircuitPython_GC9A01A) — adapted initialization sequence; MIT notice included in `THIRD_PARTY_LICENSE.txt`.
