#!/bin/sh
# The one command for the kampus trial. First run:
#   KAMPUS_DOMAIN=trial.example.edu ACME_EMAIL=ops@example.edu ./up.sh
# Later runs: ./up.sh
# Writes missing secrets to .env (mode 600, never printed), renders data/,
# then builds and starts the stack. Re-running changes nothing that exists.
set -eu
cd "$(dirname "$0")"
umask 077

[ -f .env ] || : > .env
chmod 600 .env

get() { sed -n "s/^$1=//p" .env | tail -n 1; }
put() { printf '%s=%s\n' "$1" "$2" >> .env; }

# Non-secret settings: taken from the environment once, then kept in .env.
for name in KAMPUS_DOMAIN ACME_EMAIL; do
  stored=$(get "$name")
  given=$(printenv "$name" || true)
  if [ -z "$stored" ]; then
    [ -n "$given" ] || { echo "up.sh: set $name on the first run" >&2; exit 1; }
    case $given in
      *[!A-Za-z0-9.@_+-]*) echo "up.sh: $name has unexpected characters" >&2; exit 1 ;;
    esac
    put "$name" "$given"
  elif [ -n "$given" ] && [ "$given" != "$stored" ]; then
    echo "up.sh: $name differs from .env; edit .env to change it" >&2
    exit 1
  fi
done
domain=$(get KAMPUS_DOMAIN)
case $domain in
  *[!A-Za-z0-9.-]* | .* | *.) echo "up.sh: KAMPUS_DOMAIN must be a host name" >&2; exit 1 ;;
esac

# Secrets: generated once. The subject key must never change (it would
# change every user's subject), so an existing value is always kept.
for name in KAMPUS_TRIAL_AUDIT_SALT KAMPUS_TRIAL_SUBJECT_KEY \
  KAMPUS_TRIAL_CLAUDE_SECRET KAMPUS_TRIAL_CHATGPT_SECRET; do
  if [ -z "$(get "$name")" ]; then
    value=$(openssl rand -hex 32)
    put "$name" "$value"
  fi
done

# The container runs as the owner of data/, never as root.
uid=$(id -u) gid=$(id -g)
[ "$uid" -ne 0 ] || uid=65532 gid=65532
[ -n "$(get KAMPUS_UID)" ] || put KAMPUS_UID "$uid"
[ -n "$(get KAMPUS_GID)" ] || put KAMPUS_GID "$gid"

mkdir -p data
sed "s/trial\.kampus\.invalid/$domain/g" school.trial.yaml > data/school.yaml.new
if cmp -s data/school.yaml.new data/school.yaml; then
  rm data/school.yaml.new
else
  mv data/school.yaml.new data/school.yaml
fi
# The admin page edits data/policy.yaml; never overwrite it.
[ -f data/policy.yaml ] || cp policy.trial.yaml data/policy.yaml
[ "$(id -u)" -ne 0 ] || chown -R "$(get KAMPUS_UID):$(get KAMPUS_GID)" data

docker compose config -q
docker compose up -d --build
echo "kampus trial: https://$domain/mcp"
