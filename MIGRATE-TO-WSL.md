# Migrating this project into the WSL2 filesystem

## Why

Docker Desktop runs the dev container inside **WSL2** (a Linux VM). When the
project lives on the Windows drive (`C:\Users\Dean\source\repos\...`), the
container reaches it through the slow 9P bridge mounted at `/mnt/c/...`. Cargo
does thousands of small file operations, and every one crosses that boundary —
hence Docker's *"Using an I/O intensive operation like cargo on your Windows
drives will have poor performance"* tip.

Moving the source onto the **native Linux filesystem inside WSL2** (e.g.
`~/investment-projection`) removes `/mnt/c` from the loop entirely.

### What you actually gain

The dev container already sets `CARGO_TARGET_DIR=/home/vscode/target`
(`.devcontainer/devcontainer.json`), and the CLAUDE.md `cargo check`/`test`
commands write `target/` into Docker **named volumes** — so the high-churn build
output is *already* off the Windows drive. The remaining wins from moving are:

- **`./dev.sh` (Trunk live-reload):** the big one. inotify file-watching does
  not work reliably across `/mnt/c`, so today edits can be slow to reload or
  missed. On native Linux, reloads are instant and reliable.
- **Faster incremental `cargo check`:** source reads no longer cross `/mnt/c`.

If you mostly run the containerized `cargo check`/`test` commands, this move is
**marginal — skip it**. If you actively iterate on `app/` with `./dev.sh`
live-reload, it's **worth it**, and it's a one-time ~5-minute move with zero
file edits.

## Steps

1. **Open your WSL distro** (e.g. Ubuntu). From PowerShell:

   ```bash
   wsl
   ```

2. **Put the project on the Linux filesystem.**

   If the repo has a git remote, clone it into your Linux home dir:

   ```bash
   cd ~
   git clone <your-repo-url> investment-projection
   ```

   If it's local-only, copy it across once from inside WSL (this pays the
   `/mnt/c` cost a single time):

   ```bash
   cp -r /mnt/c/Users/Dean/source/repos/investment-projection ~/investment-projection
   ```

3. **Open it from inside WSL** so VS Code attaches to the Linux filesystem, not
   `/mnt/c`:

   ```bash
   cd ~/investment-projection
   code .
   ```

   This launches VS Code with the WSL extension attached to
   `\\wsl$\Ubuntu\home\<you>\investment-projection`. Then run
   **"Reopen in Container"** as usual — now the bind-mounted source is native
   Linux.

4. **Nothing to edit.** `devcontainer.json`, `dev.sh`, `docker-compose.yml`, and
   the CLAUDE.md Docker commands all use relative paths (`${PWD}`, `build: .`,
   `cd "$(dirname "$0")/app"`), so they work unchanged. `./dev.sh` and
   `docker compose up --build` behave identically — just faster, with reliable
   file-watching.

## Things to know

- **Accessing the files from Windows:** they now live at
  `\\wsl$\Ubuntu\home\<you>\investment-projection` — browsable from Windows
  Explorer via that UNC path, but native Windows tools won't see them at the old
  `C:\...` location.
- **Don't keep two live copies.** After migrating, delete or stop editing the
  `C:\Users\Dean\source\repos\investment-projection` copy so the two don't
  diverge. Pick the WSL copy as the single source of truth.
- **Docker Desktop WSL integration** must be enabled for your distro
  (Settings → Resources → WSL Integration). It usually already is if the dev
  container runs today.
