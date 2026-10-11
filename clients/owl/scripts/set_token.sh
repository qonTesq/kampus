#!/usr/bin/env bash
# Save the kampus MCP token into .env without showing it.
#   ssh -t <user>@<pi-address> ~/kampus/clients/owl/scripts/set_token.sh
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."      # clients/owl, where .env lives
read -rsp "Paste the kampus token (without Bearer), then press Enter: " token
echo
token="${token#Bearer }"
token="$(printf '%s' "$token" | tr -d '[:space:]')"
if [ "$(printf '%s' "$token" | tr -cd '.' | wc -c)" -ne 2 ]; then
  echo "That doesn't look like a kampus token (expected three parts separated by dots). Nothing saved."
  exit 1
fi
touch .env && chmod 600 .env
grep -v '^MCP_AUTHORIZATION=' .env > .env.tmp || true
printf 'MCP_AUTHORIZATION=%s\n' "$token" >> .env.tmp
mv .env.tmp .env && chmod 600 .env
echo "Saved (${#token} characters)."
