#!/usr/bin/env bash
# One-time setup of the owl on a freshly flashed Raspberry Pi OS card.
# Run on the Pi from the owl-pi folder (it asks for your Pi password once):
#   ssh -t hienle@<pi-address> "cd ~/kampus/owl-pi && bash setup_fresh_pi.sh"
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
say() { echo; echo "==> $*"; }

say "System packages (audio, screens, Python) - asks for your Pi password"
sudo apt-get update -q
sudo apt-get install -y -q python3-venv python3-dev python3-numpy python3-pil python3-lgpio \
  python3-gpiozero python3-spidev libportaudio2 portaudio19-dev pipewire-alsa curl

say "Turn on SPI for the eye screens"
sudo raspi-config nonint do_spi 0

say "Python environment"
python3 -m venv --system-site-packages .venv
.venv/bin/pip install -q --upgrade pip
.venv/bin/pip install -q -r requirements.txt

say "Offline wake-word model"
.venv/bin/python main.py --download-wake-model

say "Key file"
if [ ! -f .env ]; then cp .env.example .env; fi
chmod 600 .env
sed -i 's/\r$//' run_owl.sh set_token.sh
chmod +x run_owl.sh set_token.sh

say "Checks"
.venv/bin/python -m unittest discover -s tests 2>&1 | tail -1
ls /dev/spidev0.0 >/dev/null 2>&1 && echo "SPI ready" || echo "SPI turns on after a reboot"

echo
echo "Done. Next: add GEMINI_API_KEY to .env, run ./set_token.sh for the kampus token,"
echo "pair the JBL, then reboot once:  sudo reboot"
