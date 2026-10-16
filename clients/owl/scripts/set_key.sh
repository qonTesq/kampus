#!/usr/bin/env bash
# Save one key into .env without showing it on screen.
#   ssh -t <user>@<pi-address> ~/kampus/clients/owl/scripts/set_key.sh GEMINI_API_KEY
#   ssh -t <user>@<pi-address> ~/kampus/clients/owl/scripts/set_key.sh MCP_AUTHORIZATION   (kampus token, no "Bearer")
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."      # clients/owl, where .env lives
name="${1:-}"
case "$name" in
  GEMINI_API_KEY|ELEVENLABS_API_KEY|OPENAI_API_KEY|MCP_AUTHORIZATION) ;;
  *) echo "Usage: set_key.sh GEMINI_API_KEY | MCP_AUTHORIZATION | ELEVENLABS_API_KEY"; exit 1 ;;
esac
read -rsp "Paste $name, then press Enter: " value
echo
value="${value#Bearer }"
value="$(printf '%s' "$value" | tr -d '[:space:]')"
[ "${#value}" -ge 10 ] || { echo "That looks too short. Nothing saved."; exit 1; }
touch .env && chmod 600 .env
grep -v "^${name}=" .env > .env.tmp || true
printf '%s=%s\n' "$name" "$value" >> .env.tmp
mv .env.tmp .env && chmod 600 .env
echo "Saved $name (${#value} characters)."
