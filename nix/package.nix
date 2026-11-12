{
  lib,
  rustPlatform,
  pkg-config,
  makeWrapper,
  makeDesktopItem,
  mpv,
  ffmpeg,
  yt-dlp,
  xdg-utils,
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
  dbus,
}:

let
  # winit/glutin (and the Wayland/X11 backends) dlopen these at runtime, so
  # they have to be on LD_LIBRARY_PATH even though they are linked at build
  # time as well.
  runtimeLibs = [
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
    dbus
  ];

  runtimeTools = [
    mpv
    ffmpeg
    yt-dlp
    xdg-utils
  ];
in
rustPlatform.buildRustPackage rec {
  pname = "scwoll";
  version = "0.1.0";

  # Only ship what the build actually needs; target/, dist/, data/ and the
  # AppDir are not part of the source derivation.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../logo.png
      ../assets/scwoll.png
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = runtimeLibs ++ [
    wayland-protocols
  ];

  # The mosaic/thumbnail tests shell out to ffprobe/ffmpeg, and fall back
  # to mpv when the ffmpeg engine cannot seek.
  nativeCheckInputs = [
    ffmpeg
    mpv
  ];

  postFixup = ''
    wrapProgram $out/bin/scwoll \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs} \
      --prefix PATH : ${lib.makeBinPath runtimeTools}
  '';

  desktopItem = makeDesktopItem {
    name = "scwoll";
    desktopName = "Scwoll";
    genericName = "Media Player";
    comment = "Native YouTube-style media player (Rust, mpv playback)";
    exec = "scwoll";
    icon = "scwoll";
    categories = [
      "AudioVideo"
      "Video"
      "Player"
    ];
    keywords = [
      "video"
      "player"
      "youtube"
      "mpv"
    ];
    startupWMClass = "scwoll";
  };

  postInstall = ''
    install -Dm644 ${desktopItem}/share/applications/scwoll.desktop \
      $out/share/applications/scwoll.desktop
    install -Dm644 ${../assets/scwoll.png} \
      $out/share/icons/hicolor/512x512/apps/scwoll.png
  '';

  meta = with lib; {
    description = "Native YouTube-style media player built with egui and mpv";
    homepage = "https://github.com/Atlas-BES/scwoll";
    mainProgram = "scwoll";
    platforms = platforms.linux;
  };
}
