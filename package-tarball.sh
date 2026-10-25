#!/usr/bin/env bash
# package-tarball.sh — build a release binary and pack it into a .tar.gz
#
# Usage:
#   ./package-tarball.sh [--skip-build] [--output-dir DIR]
#
# Output:
#   dist/scwoll-<version>-<arch>.tar.gz  (+ .sha256)
#
# Contents of the tarball:
#   scwoll-<version>-<arch>/
#     scwoll            release binary
#     README.md
#     sample_urls.txt
#     scwoll.png        app icon (from project logo.png)
#
# Notes:
#   - Build runs inside nix-shell when cargo isn't on PATH (needed on NixOS
#     so winit/wayland deps resolve).
#   - Excludes data/thumbs cache and target/ build dirs on purpose.
#   - Runtime still requires mpv (+ ffmpeg for mosaics) installed.
set -euo pipefail

SKIP_BUILD=0
OUTPUT_DIR="dist"

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
  echo "Options:"
  echo "  --skip-build      reuse existing target/release/scwoll, don't rebuild"
  echo "  --output-dir DIR  where to write the tarball (default: dist)"
  echo "  -h, --help        show this help"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 1 ;;
  esac
done

cd "$(dirname "$0")"

NAME=$(sed -n 's/^name *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
VERSION=$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -1)
: "${NAME:=scwoll}"
: "${VERSION:=0.1.0}"
ARCH="$(uname -m)"

BIN="target/release/${NAME}"
if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "==> Building ${NAME} v${VERSION} (release)…"
  if command -v cargo >/dev/null 2>&1; then
    cargo build --release
  elif command -v nix-shell >/dev/null 2>&1; then
    nix-shell --command 'cargo build --release'
  else
    echo "error: neither cargo nor nix-shell found" >&2
    exit 1
  fi
fi
[[ -x "$BIN" ]] || { echo "error: $BIN not found (build it first or drop --skip-build)" >&2; exit 1; }

STAGE="${NAME}-${VERSION}-${ARCH}"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "$BIN" "$STAGE/${NAME}"
[[ -f README.md ]]       && cp README.md "$STAGE/"
[[ -f sample_urls.txt ]] && cp sample_urls.txt "$STAGE/"
ICON_SRC=""
[[ -f logo.png ]] && ICON_SRC="logo.png"
[[ -z "$ICON_SRC" && -f assets/scwoll.png ]] && ICON_SRC="assets/scwoll.png"
[[ -n "$ICON_SRC" ]] && cp "$ICON_SRC" "$STAGE/scwoll.png"
chmod +x "$STAGE/${NAME}"

mkdir -p "$OUTPUT_DIR"
TARBALL="${OUTPUT_DIR}/${STAGE}.tar.gz"
tar -czf "$TARBALL" "$STAGE"
rm -rf "$STAGE"

sha256sum "$TARBALL" | tee "${TARBALL}.sha256" | awk '{print $1}' >/dev/null
echo "==> Wrote $TARBALL"
cat "${TARBALL}.sha256"
tar -tzf "$TARBALL"
