# owl client: agent instructions

This folder is the Python voice client for the Raspberry Pi mascot. It takes precedence over the root `AGENTS.md` for files in this folder; the root rules on secrets, privacy and commits still apply. How to contribute (branch, PR, review): the root `AGENTS.md`, section "Contributing from another machine".

- **One professional layout, the one in `README.md`:** a uv project (`pyproject.toml` + committed `uv.lock`, Python version pinned), the package under `src/owl/` split into `hardware/`, `brain/`, `voice/` and `kampus/`, tests in `tests/`, `config/config.example.toml`, and `scripts/` for Pi setup and run. Lint and format with ruff, test with pytest.
- **Bringing existing code in = migrating it into that layout,** not copying its old shape: move modules into the right subpackage, turn `requirements.txt`/Poetry/pip setups into `pyproject.toml` dependencies (same versions; `uv lock`), fix imports, and keep the behaviour identical. Verify it still runs (or that each module imports, where hardware is missing) and that `uv run ruff check .` and `uv run pytest` pass. Anything that couldn't move cleanly goes in the PR description, not silently changed.
- **CI** (`.github/workflows/owl.yml`) runs those checks once `uv.lock` exists.
- **Ownership** (see `.github/CODEOWNERS`): the hardware owner owns this folder together with the software owner. Once these folders exist, `src/owl/brain/`, `src/owl/voice/` and `src/owl/kampus/` stay with the software owner; hardware-specific code (GPIO, SPI, displays, audio devices) and Pi setup scripts belong to the hardware owner.
- **Never commit secrets:** API keys and tokens live in `.env` or a local config file that is git-ignored. Commit an example file (for example `config/config.example.toml` or `.env.example`) instead.
- **Kampus is reached only over MCP** with the owl's own device credential. The owl is a public device: it must never request, store or speak personal student data, and it treats a `permission_denied` result as a normal answer ("that's personal; open it in your school's approved assistant").
- **Privacy:** don't log transcripts, audio or tool results by default.
- Keep hardware-specific code behind small interfaces where you can, so the rest can be tested on a laptop.
