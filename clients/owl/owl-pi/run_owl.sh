#!/usr/bin/env bash
# One-command demo launcher for the talking owl.
# Checks everything the demo needs, fixes what it can, then starts the owl.
#
#   ./run_owl.sh              # full owl: say "Hey Owl" to wake it
#   ./run_owl.sh --no-wake    # start awake (skip "Hey Owl")
#   ./run_owl.sh --text       # type instead of speaking (backup if the room is loud)
set -u
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
SPEAKER=18:90:67:87:94:0C      # JBL Go 3
VOLUME=0.62

ok()   { echo "  [ok]   $*"; }
fail() { echo "  [FAIL] $*"; exit 1; }

echo "Pre-flight checks"

# 1. API keys present in .env (values never printed)
keys="GEMINI_API_KEY"
grep -Eq '^voice = "elevenlabs"' config.toml && keys="$keys ELEVENLABS_API_KEY"
for key in $keys; do
  grep -Eq "^${key}=.{10,}" .env 2>/dev/null || fail "$key missing in owl-pi/.env"
done
ok "API keys present"

# 2. Internet (phone hotspot)
curl -s -o /dev/null -m 5 https://generativelanguage.googleapis.com \
  || fail "no internet: turn on the iPhone hotspot (keep its screen open)"
ok "internet"

# 3. JBL speaker: reconnect if needed, make it the output, restore volume
for _ in 1 2 3; do
  bluetoothctl info "$SPEAKER" | grep -q "Connected: yes" && break
  bluetoothctl --timeout 10 connect "$SPEAKER" >/dev/null 2>&1
  sleep 3
done
bluetoothctl info "$SPEAKER" | grep -q "Connected: yes" \
  || fail "JBL not connected: press its power button, then run this again"
sleep 1
wpctl status | sed -n '/Audio/,/Video/p' | grep -E '│\s+\*' | grep -q "JBL" \
  || fail "JBL connected but not the default output: run  wpctl status  and  wpctl set-default <id>"
wpctl set-volume @DEFAULT_AUDIO_SINK@ "$VOLUME"
ok "JBL speaker connected, volume ${VOLUME}"

# 4. Webcam microphone
lsusb | grep -qi "brio" || fail "webcam not detected: plug it into a blue USB port"
ok "webcam mic"

# 5. Power and temperature
throttled=$(vcgencmd get_throttled | cut -d= -f2)
[ "$throttled" = "0x0" ] || echo "  [warn] power warning ($throttled): use the official 27W supply"
ok "$(vcgencmd measure_temp)"

# 6. Nothing else holding the screens or audio
pkill -INT -f "[.]venv/bin/python .*(main|echo_test|soak_test|voice_pipeline)\.py" && sleep 2

echo "All set. Starting the owl (Ctrl+C to stop)."
echo
exec .venv/bin/python -u voice_pipeline.py "$@"
