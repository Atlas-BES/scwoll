#!/usr/bin/env bash
# Launch Scwoll the right way: inside nix-shell so winit finds
# libwayland / libGL / libxkbcommon at runtime.
cd "$(dirname "$0")"
exec nix-shell --command 'cargo run --release'
