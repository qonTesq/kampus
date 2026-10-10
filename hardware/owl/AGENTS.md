# owl hardware: agent instructions

This folder is the physical build of the owl mascot: CAD, print files, wiring and parts. It takes precedence over the root `AGENTS.md` for files here; the root rules on secrets and commits still apply. How to contribute (branch, PR, review): the root `AGENTS.md`, section "Contributing from another machine". The Rust rules don't apply here.

- **Where things go** (see `README.md`): `cad/` source models, `print/` STL/3MF and print settings, `wiring/` diagrams and pin maps, `photos/` build photos, `bom.md` the parts list with links. Keep the files' original formats and names; add a short note in `README.md` for anything a reader needs to open them (tool and version).
- **Large files use Git LFS.** Run `git lfs install` once before adding CAD or print files. `.stl`, `.3mf`, `.step` and `.stp` are tracked by LFS through the root `.gitattributes`; if you add another large binary type (for example `.f3d`, `.FCStd`, `.blend`), add a matching LFS line to `.gitattributes` in the same PR.
- **No secrets or personal data:** no Wi-Fi passwords, API keys or account details in wiring notes, photos or scripts.
- Pi software (the voice client, setup scripts) lives in `clients/owl/` and `deploy/owl-pi/`, not here.
