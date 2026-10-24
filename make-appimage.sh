#!/usr/bin/env bash
# make-appimage.sh — build a release binary and package it as an AppImage
#
# Usage:
#   ./make-appimage.sh [--skip-build] [--appdir-only]
#                      [--appimagetool PATH] [--output-dir DIR]
#
# Output:
#   dist/Scwoll-<version>-<arch>.AppImage  (+ .sha256)
#
# How it works:
#   1. cargo build --release (via nix-shell if cargo isn't on PATH)
#   2. stages Scwoll.AppDir/ (AppRun, *.desktop, icon, usr/bin/scwoll)
#   3. runs appimagetool (uses --appimagetool, or one on PATH, or downloads
#      the official static build from GitHub)
#
# Notes:
#   - eframe/egui system libs (wayland, GL, xkb…) are NOT bundled — the
#     AppImage targets modern desktop distros that ship them. For a fully
#     hermetic bundle use linuxdeploy with its plugins instead.
#   - Runtime still requires mpv (+ ffmpeg for thumbnail mosaics) installed.
#   - --appdir-only stops after step 2 (handy for testing or manual runs of
#     appimagetool / linuxdeploy).
set -euo pipefail

SKIP_BUILD=0
APPDIR_ONLY=0
APPIMAGETOOL=""
OUTPUT_DIR="dist"

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//'
  echo "Options:"
  echo "  --skip-build          reuse existing target/release/scwoll"
  echo "  --appdir-only         only stage Scwoll.AppDir, skip appimagetool"
  echo "  --appimagetool PATH   use this appimagetool binary/AppImage"
  echo "  --output-dir DIR      where to write the AppImage (default: dist)"
  echo "  -h, --help            show this help"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --appdir-only) APPDIR_ONLY=1; shift ;;
    --appimagetool) APPIMAGETOOL="$2"; shift 2 ;;
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
APPIMAGE_ARCH="$ARCH"
[[ "$ARCH" == "x86_64" ]] && APPIMAGE_ARCH="x86_64"
[[ "$ARCH" == "aarch64" ]] && APPIMAGE_ARCH="aarch64"

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

# ---- icon: prefer checked-in PNG, else SVG, else draw a fallback ----
ICON_SRC=""
[[ -f assets/scwoll.png ]] && ICON_SRC="assets/scwoll.png"
[[ -z "$ICON_SRC" && -f assets/scwoll.svg ]] && ICON_SRC="assets/scwoll.svg"

# ---- stage AppDir ----
APPDIR="Scwoll.AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
         "$APPDIR/usr/share/doc/scwoll"

cp "$BIN" "$APPDIR/usr/bin/${NAME}"
chmod +x "$APPDIR/usr/bin/${NAME}"

if [[ -n "$ICON_SRC" ]]; then
  if [[ "$ICON_SRC" == *.svg ]]; then
    if command -v rsvg-convert >/dev/null 2>&1; then
      rsvg-convert -w 256 -h 256 "$ICON_SRC" -o "$APPDIR/scwoll.png"
    elif command -v magick >/dev/null 2>&1; then
      magick -background none "$ICON_SRC" -resize 256x256 "$APPDIR/scwoll.png"
    elif command -v convert >/dev/null 2>&1; then
      convert -background none "$ICON_SRC" -resize 256x256 "$APPDIR/scwoll.png"
    else
      echo "warning: no SVG converter found; AppDir will have no icon" >&2
    fi
  else
    cp "$ICON_SRC" "$APPDIR/scwoll.png"
  fi
fi
# Ultimate fallback: paint a minimal play-button PNG with ImageMagick.
if [[ ! -f "$APPDIR/scwoll.png" ]] && command -v convert >/dev/null 2>&1; then
  convert -size 256x256 xc:none \
    -fill '#141a28' -draw 'roundrectangle 8,8 248,248 56,56' \
    -fill '#38bdf8' -draw 'polygon 104,76 184,128 104,180' \
    "$APPDIR/scwoll.png"
fi
[[ -f "$APPDIR/scwoll.png" ]] || echo "warning: no icon bundled (install rsvg-convert or ImageMagick)" >&2
[[ -f "$APPDIR/scwoll.png" ]] && cp "$APPDIR/scwoll.png" \
  "$APPDIR/usr/share/icons/hicolor/256x256/apps/scwoll.png"

cat > "$APPDIR/scwoll.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Scwoll
Comment=Native YouTube-style media player (Rust, mpv playback)
Exec=scwoll
Icon=scwoll
Categories=AudioVideo;Video;Player;
Terminal=false
StartupWMClass=scwoll
EOF
cp "$APPDIR/scwoll.desktop" "$APPDIR/usr/share/applications/"

[[ -f README.md ]]       && cp README.md "$APPDIR/usr/share/doc/scwoll/"
[[ -f sample_urls.txt ]] && cp sample_urls.txt "$APPDIR/usr/share/doc/scwoll/"

cat > "$APPDIR/AppRun" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
HERE="$(dirname "$(readlink -f "$0")")"
export APPDIR="$HERE"
export PATH="$HERE/usr/bin:$PATH"
for missing in mpv; do
  command -v "$missing" >/dev/null 2>&1 || \
    echo "Scwoll: optional runtime dependency '$missing' not found on PATH" >&2
done
exec "$HERE/usr/bin/scwoll" "$@"
EOF
chmod +x "$APPDIR/AppRun"

command -v desktop-file-validate >/dev/null 2>&1 \
  && desktop-file-validate "$APPDIR/scwoll.desktop" || true

echo "==> Staged $APPDIR:"
find "$APPDIR" -maxdepth 3 | sort

if [[ "$APPDIR_ONLY" -eq 1 ]]; then
  echo "==> --appdir-only: stopping before appimagetool."
  exit 0
fi

# ---- find or fetch appimagetool ----
TOOL="$APPIMAGETOOL"
[[ -z "$TOOL" ]] && command -v appimagetool >/dev/null 2>&1 && TOOL="$(command -v appimagetool)"
if [[ -z "$TOOL" ]]; then
  TOOL="dist/appimagetool-${APPIMAGE_ARCH}.AppImage"
  if [[ ! -x "$TOOL" ]]; then
    echo "==> Downloading appimagetool…"
    mkdir -p dist
    URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${APPIMAGE_ARCH}.AppImage"
    if command -v curl >/dev/null 2>&1; then
      curl -fSL -o "$TOOL" "$URL"
    elif command -v wget >/dev/null 2>&1; then
      wget -O "$TOOL" "$URL"
    else
      echo "error: need curl or wget to fetch appimagetool (or pass --appimagetool PATH)" >&2
      exit 1
    fi
    chmod +x "$TOOL"
  fi
fi

mkdir -p "$OUTPUT_DIR"
OUT="${OUTPUT_DIR}/Scwoll-${VERSION}-${APPIMAGE_ARCH}.AppImage"
rm -f "$OUT"

echo "==> Running appimagetool…"
# appimagetool hard-requires the `file(1)` utility (and sha256sum for hashing
# below). Fail fast with an actionable hint instead of its terse
# "file command is missing but required" error.
if ! command -v file >/dev/null 2>&1; then
  echo "error: 'file' command is missing but required by appimagetool." >&2
  echo "install it and re-run:" >&2
  echo "  NixOS / nix-shell:  nix-shell -p file --run ./make-appimage.sh" >&2
  echo "                      (or enter a shell that provides it: nix-shell -p file)" >&2
  echo "  Debian/Ubuntu:      sudo apt install file" >&2
  echo "  Fedora:             sudo dnf install file" >&2
  echo "  Arch:               sudo pacman -S file" >&2
  exit 1
fi
if [[ "$TOOL" == *.AppImage ]]; then
  # extract-and-run avoids needing FUSE to execute the tool itself.
  ARCH="$APPIMAGE_ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" \
    "$APPDIR" "$OUT"
else
  ARCH="$APPIMAGE_ARCH" "$TOOL" "$APPDIR" "$OUT"
fi
chmod +x "$OUT"

sha256sum "$OUT" | tee "${OUT}.sha256" | awk '{print $1}' >/dev/null
echo "==> Wrote $OUT"
cat "${OUT}.sha256"
