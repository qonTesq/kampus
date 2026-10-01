# owl client: agent instructions

This folder is the Python voice client for the Raspberry Pi mascot. It takes precedence over the root `AGENTS.md` for files in this folder; the root rules on secrets, privacy and commits still apply.

- **Python, managed with uv.** Dependencies go in `pyproject.toml`; commit `uv.lock`. Lint and format with ruff; test with pytest.
- **Ownership:** `src/owl/hardware/` and `scripts/` belong to the hardware owner; `src/owl/brain/`, `src/owl/voice/` and `src/owl/kampus/` belong to the software owner (see `.github/CODEOWNERS`).
- **Never commit secrets:** API keys and tokens live in `.env` (untracked). Commit `config/config.example.toml`, never `config/config.toml`.
- **kampus is reached only over MCP** with the owl's own device credential. The owl is a public device: it must never request, store or speak personal student data, and it treats a `permission_denied` result as a normal answer ("that's personal; open it in your school's approved assistant").
- **Privacy:** don't log transcripts, audio or tool results by default.
- Keep hardware-specific code (GPIO, SPI, audio devices) behind small interfaces so the rest can be tested on a laptop.
