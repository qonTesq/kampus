# owl client: agent instructions

This folder is the Python voice client for the Raspberry Pi mascot. It takes precedence over the root `AGENTS.md` for files in this folder; the root rules on secrets, privacy and commits still apply. How to contribute (branch, PR, review): the root `AGENTS.md`, section "Contributing from another machine".

- **Keep the tooling the code already uses.** If the Pi code arrives with uv, pip + `requirements.txt`, Poetry or anything else, keep it. Don't migrate tools, rename packages or restructure folders unless both owners agree in the PR. Only a brand-new Python project starts with uv (`pyproject.toml` plus a committed `uv.lock`), ruff and pytest.
- **CI** (`.github/workflows/owl.yml`) lints and tests only uv projects, detected by `uv.lock`. Anything else is reported as "not checked", never failed. If you use other tooling, say in `README.md` how to install and run it.
- **Ownership** (see `.github/CODEOWNERS`): the hardware owner owns this folder together with the software owner. Once these folders exist, `src/owl/brain/`, `src/owl/voice/` and `src/owl/kampus/` stay with the software owner; hardware-specific code (GPIO, SPI, displays, audio devices) and Pi setup scripts belong to the hardware owner.
- **Never commit secrets:** API keys and tokens live in `.env` or a local config file that is git-ignored. Commit an example file (for example `config/config.example.toml` or `.env.example`) instead.
- **Kampus is reached only over MCP** with the owl's own device credential. The owl is a public device: it must never request, store or speak personal student data, and it treats a `permission_denied` result as a normal answer ("that's personal; open it in your school's approved assistant").
- **Privacy:** don't log transcripts, audio or tool results by default.
- Keep hardware-specific code behind small interfaces where you can, so the rest can be tested on a laptop.
