# Running the owl on a Raspberry Pi

Setup and service files for running the owl client on the Pi.

- **First-time setup:** `clients/owl/scripts/setup_pi.sh` (system packages, SPI, uv, the speech model).
- **Start on boot:** a systemd unit goes here (`owl.service`), running `clients/owl/scripts/run_owl.sh` as a non-root user.
- **Secrets** stay in `clients/owl/.env` on the Pi, readable only by that user (`chmod 600`).
- **Kampus connection:** set `MCP_SERVER_URL` to the school's Kampus URL, and `MCP_AUTHORIZATION` to the owl's device token.
