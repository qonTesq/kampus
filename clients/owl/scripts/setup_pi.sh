#!/usr/bin/env bash
# One-time setup of the owl on a freshly flashed Raspberry Pi OS (64-bit, trixie) card.
# Run on the Pi (it asks for your Pi password once):
#   ssh -t <user>@<pi-address> "bash ~/kampus/clients/owl/scripts/setup_pi.sh"
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."      # clients/owl
if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Run this on the Raspberry Pi, not Windows." >&2
  exit 1
fi
say() { echo; echo "==> $*"; }

say "System packages (audio, screens, camera, Python) - asks for your Pi password"
sudo apt-get update -q
sudo apt-get install -y -q python3-venv python3-dev python3-numpy python3-pil python3-lgpio \
  python3-gpiozero python3-spidev python3-opencv libportaudio2 portaudio19-dev pipewire-alsa curl

say "Turn on SPI for the eye screens"
sudo raspi-config nonint do_spi 0

say "uv (Python project manager)"
if ! command -v uv >/dev/null; then
  curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="$HOME/.local/bin:$PATH"
fi

say "Python environment"
# System site packages give the venv Raspberry Pi OS's lgpio (GPIO) and OpenCV (camera).
uv venv --allow-existing --system-site-packages --python /usr/bin/python3 .venv
uv sync --locked --python /usr/bin/python3

say "Offline wake-word model"
uv run --no-sync owl-realtime --download-wake-model

say "Key and config files"
if [ ! -f .env ]; then cp .env.example .env; fi
chmod 600 .env
if [ ! -f config/config.toml ]; then cp config/config.example.toml config/config.toml; fi
sed -i 's/\r$//' scripts/*.sh
chmod +x scripts/*.sh

say "Checks"
uv run --no-sync pytest -q 2>&1 | tail -1
ls /dev/spidev0.0 >/dev/null 2>&1 && echo "SPI ready" || echo "SPI turns on after a reboot"

echo
echo "Done. Next: scripts/set_key.sh GEMINI_API_KEY, scripts/set_token.sh for the kampus token,"
echo "pair the Bluetooth speaker, then reboot once:  sudo reboot"
