{
  lib,
  mkShell,
  rustc,
  cargo,
  rustfmt,
  clippy,
  pkg-config,
  stdenv,
  ffmpeg,
  mpv,
  yt-dlp,
  libGL,
  wayland,
  wayland-protocols,
  libxkbcommon,
  libx11,
  libxcursor,
  libxrandr,
  libxi,
  libXScrnSaver,
  fontconfig,
  freetype,
  gtk3,
  glib,
  dbus,
  chromium,
  file,
  desktop-file-utils,
}:

let
  guiLibs = [
    libGL
    wayland
    libxkbcommon
    libx11
    libxcursor
    libxrandr
    libxi
    libXScrnSaver
    fontconfig
    freetype
    gtk3
    glib
    dbus
  ];
in
mkShell {
  buildInputs = [
    rustc
    cargo
    rustfmt
    clippy
    pkg-config
    stdenv.cc
    ffmpeg
    mpv
    yt-dlp
    fontconfig
    freetype
    wayland
    wayland-protocols
    libxkbcommon
    libGL
    libx11
    libxcursor
    libxrandr
    libxi
    gtk3
    glib
    dbus
    chromium # headless JS rendering for Sources pages (--dump-dom snapshots)
    file # required by appimagetool when building the AppImage
    desktop-file-utils # desktop-file-validate for make-appimage.sh (optional)
  ];

  # winit/glutin dlopen these at RUNTIME, so they must be on LD_LIBRARY_PATH.
  LD_LIBRARY_PATH = lib.makeLibraryPath guiLibs;
  PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" guiLibs;

  shellHook = ''
    export LD_LIBRARY_PATH=${lib.makeLibraryPath guiLibs}:$LD_LIBRARY_PATH
    echo "📼 scwoll dev shell — run: cargo run --release"
    echo "   If Wayland fails, force X11: WINIT_UNIX_BACKEND=x11 cargo run --release"
  '';
}
