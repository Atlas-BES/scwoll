{ pkgs ? import <nixpkgs> {} }:
let
  guiLibs = with pkgs; [
    libGL
    wayland
    wayland-protocols
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
    openssl
  ];
in
pkgs.mkShell {
  buildInputs = with pkgs; [
    cargo rustc gcc pkg-config openssl ffmpeg
    wayland libxkbcommon libGL
    libx11 libxcursor libxrandr libxi
    fontconfig freetype
    gtk3 glib dbus
    chromium # headless JS rendering for Sources pages (--dump-dom snapshots)
    file # required by appimagetool when building the AppImage
    desktop-file-utils # desktop-file-validate for make-appimage.sh (optional)
  ];

  # winit/glutin dlopen these at RUNTIME, so they must be on LD_LIBRARY_PATH.
  # Always launch the app from inside `nix-shell` (or `nix develop`).
  LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath guiLibs}";
  PKG_CONFIG_PATH = "${pkgs.lib.makeSearchPath "lib/pkgconfig" guiLibs}";

  shellHook = ''
    export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath guiLibs}:$LD_LIBRARY_PATH
    echo "📼 scwoll native shell — run: cargo run --release"
    echo "   If Wayland fails, force X11: WINIT_UNIX_BACKEND=x11 cargo run --release"
  '';
}
