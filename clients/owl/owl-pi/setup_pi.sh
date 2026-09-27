#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Run this on the Raspberry Pi, not Windows." >&2
  exit 1
fi
sudo apt update
sudo apt install -y python3-venv python3-pip python3-dev python3-numpy python3-pil \
  python3-lgpio python3-gpiozero python3-spidev python3-opencv libportaudio2 portaudio19-dev
python3 -m venv --system-site-packages .venv
.venv/bin/python -m pip install --upgrade pip
.venv/bin/python -m pip install -r requirements.txt
if [[ ! -f .env ]]; then
  cp .env.example .env
  chmod 600 .env
fi
echo
echo 'Installed. Next: enable SPI with sudo raspi-config, pair your Bluetooth speaker,'
echo 'then read README.md for audio selection, wake-model download, and your API key.'
