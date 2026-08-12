#!/usr/bin/env bash
# Local development helper (for use inside the devcontainer, or anywhere with
# Rust + the wasm32 target + Trunk installed). Serves the app with live reload
# on http://localhost:8080.
#
# For a zero-setup run, prefer `docker compose up --build` instead.
set -euo pipefail
# Trunk runs from the `app` crate dir: in a virtual workspace it only resolves
# the target package when invoked from within the member crate.
cd "$(dirname "$0")/app"

# Debug profile deliberately: the release profile carries `opt-level = "s"` and
# LTO, which every live-reload rebuild would otherwise pay for. The wasm is much
# larger and slower, which is fine locally. Add `--release` back when you need to
# check real bundle size or runtime performance.
exec trunk serve --address 0.0.0.0 --port 8080
