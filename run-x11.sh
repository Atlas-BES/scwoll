#!/usr/bin/env bash
# X11 fallback (XWayland) if your Wayland socket/libs misbehave.
cd "$(dirname "$0")"
exec env WINIT_UNIX_BACKEND=x11 nix-shell --keep WINIT_UNIX_BACKEND --command 'cargo run --release'
