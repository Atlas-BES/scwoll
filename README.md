# 📼 Scwoll — native YouTube-style media player in Rust

100% native desktop app. No browser, no server. Built with `egui`/`eframe`, playback via `mpv`.

## Features

- **YouTube-style UI** — top search bar, sidebar (Home / YouTube / Local / Direct), category chips, thumbnail-first video grid (avatar, title, channel/category, age, right-click actions), theater view + Up Next
- **3 import paths** (`＋ Import`)
  - **Single URL** — YouTube (`watch`, `youtu.be`, `embed`, `shorts`, `live`) or direct `mp4/webm/mkv/mov…` links, optional category + title override
  - **List / .txt** — paste many URLs or `📄 Choose .txt…` (`.txt`/`.m3u`/`.csv`, `#` comments ignored, see `sample_urls.txt`)
  - **Folder** — picker or typed path, optional subfolders; category defaults to folder name. Scans `mp4 mkv webm mov m4v avi ogv flv wmv ts m2ts mpg mpeg`
- **Categories** — auto-created on import, chip filter, sidebar with counts, per-video rename in theater view
- **Playback** — `▶ Play` spawns `mpv --no-terminal <file|url>` (YouTube URLs play through mpv's ytdl support). Stop/switch kills the previous instance. Status line shows what's playing. Requires `mpv` installed (you have 0.41.0 ✓)
- **Thumbnails** — YouTube artwork fetched in background threads; any video
  without one (local files, direct links, missing artwork) falls back to a
  **6-frame mosaic** — 6 frames evenly spaced across the full duration,
  square-cropped and tiled 3×2. Two engines: `ffprobe`+`ffmpeg` (fast, precise)
  then pure-`mpv` percent-seeks (works anywhere `mpv` exists, incl. YouTube
  links). Cached in `~/.config/scwoll/thumbs/`; last resort is a letter tile
- **Themes & visual effects** (`⚙ Settings`)
  - **System font picker** (`🔤 Font`) — search and apply any font family
    installed via fontconfig; missing glyphs/emoji fall back to the bundled
    fonts and the choice is saved in `~/.config/scwoll/theme.json`
  - **Custom background image** (`🧊 Glass & motion`) — pick a PNG/JPEG,
    center-cropped to fill the window, with an image-dim slider and path
    saved in `~/.config/scwoll/theme.json`
  - **Presets** — ✨ Liquid Glass, Frosted, Neon, OLED, Minimal
  - **Liquid glass** toggle with panel opacity, corner radius, card size,
    shadow/depth and accent glow
  - **System blur** toggle + intensity — asks the compositor to blur the
    desktop behind the window (KWin/Wayland `ext-background-effect-v1`, or
    X11 `_KDE_NET_WM_BLUR_BEHIND_REGION`) and fades Scwoll's own backdrop so
    the real blur shows through
  - **Minimal top-bar dropdowns** — Settings and Import open right under
    their toolbar buttons as flat, square extensions of the top panel
    (no extra rounding, border, shadow or transparency), close on
    outside-click or Esc, and stay compact with tabs, presets, colour
    pickers and scrollable content
  - **Frost / blur** slider (simulated translucent frosting), **background
    transparency** (default 5%, lets the desktop show through), animated
    **aurora background** with intensity + motion toggle, film grain and
    vignette
  - Hover rim-light on cards, accent gradient panel edges, glass cards and
    rounded thumbnails with a play overlay; all buttons animate their fill,
    border, glow and expansion on hover
  - Fully customizable colors: accent, accent 2 / aurora, background,
    cards/windows, inputs, text, muted text and sidebar. Saved to
    `~/.config/scwoll/theme.json`
- **Persistence** — `~/.config/scwoll/library.json` + `theme.json` (3 demo videos seeded on first run); the directory follows `$XDG_CONFIG_HOME` when set. **⚙ Settings → 📁 Data folder** opens it in your file manager. An existing CWD-relative `data/` folder is copied over automatically on first launch

## Run

> ⚠️ You **must** launch from inside `nix-shell` — otherwise winit can't
> find `libwayland-client.so` and dies with
> `WaylandError(Connection(NoWaylandLib))` (exactly your error).
>
> ```bash
> cd ~/Projects/amedia
> ./run.sh            # Wayland (preferred)
> ./run-x11.sh        # X11/XWayland fallback
> ```
>
> Manual equivalents:
>
> ```bash
> nix-shell --command 'cargo run --release'
> WINIT_UNIX_BACKEND=x11 nix-shell --keep WINIT_UNIX_BACKEND --command 'cargo run --release'
> ```

Requires a graphical session (Wayland/X11) + `mpv` + `ffmpeg` (all provided
inside `nix-shell` via `shell.nix`):
```bash
which mpv   # /etc/profiles/per-user/atlas/bin/mpv ✓
```

## Android (arm64 APK)

Scwoll can be compiled as a standalone Android APK for `aarch64-linux-android`
(arm64) and run on Android phones/tablets. It runs as a `NativeActivity` and
switches to a **Material You (Material 3) phone UI**: top app bar, horizontally
scrollable category chips, a vertical feed of media cards, a bottom navigation
bar (Home / YouTube / Local / Sources) and full-screen Settings / Import pages.
The colours are derived from the app theme's accent, so the existing theme
picker also recolours the phone UI.

Build with `cargo-apk` (configured via `[package.metadata.android]` in
`Cargo.toml`, launcher icons in `android/res/mipmap-*/ic_launcher.png`).
The `scwoll` package has both a `[[bin]]` (desktop) and a `cdylib` `[lib]`
(Android), so the build must pass `--lib` — `./build-apk.sh` does that for you.

```bash
# 1. Nix-provided Android SDK + NDK + rustup + adb + JDK + host `cc`
#    (one-time; accepts the unfree Android SDK license, downloads several GB):
export NIXPKGS_ALLOW_UNFREE=1
export NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE=1
nix-shell shell-android.nix

# 2. Inside the shell (one-time): install the Rust toolchain + arm64 target,
#    then build (installs cargo-apk automatically if needed):
rustup default stable
./build-apk.sh

# 3. Install & launch on a connected device:
adb install -r target/release/apk/scwoll.apk
adb shell am start -n dev.scwoll.app/android.app.NativeActivity

# 4. Logs (stdout/stderr are forwarded to logcat):
adb logcat -d -s RustStdoutStderr:V AndroidRuntime:E DEBUG:E libc:F
```

If you prefer Android Studio's SDK/NDK instead of the Nix ones, skip
`shell-android.nix` and export the usual paths before running `./build-apk.sh`:

```bash
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/<version>
export ANDROID_HOME=$HOME/Android/Sdk
nix-shell -p rustup --run 'rustup default stable'
./build-apk.sh
```

Notes for Android:

- `./build-apk.sh` checks the prerequisites, installs `cargo-apk` if needed,
  and runs `cargo apk build --profile release --lib` for
  `aarch64-linux-android` only (`--lib` avoids a `Bin is not compatible with
  Cdylib` panic from cargo-apk).
- It builds a **release** APK (~4 MB) signed with a locally generated key at
  `~/.android/scwoll-release.keystore` (password `android`; override with
  `CARGO_APK_RELEASE_KEYSTORE` / `CARGO_APK_RELEASE_KEYSTORE_PASSWORD`).
- **Why release by default:** `android-activity` 0.5.2 (pinned by winit 0.29)
  calls `slice::from_raw_parts(null, 0)` for the absent saved instance state.
  rustc's debug-only UB precondition checks panic on that inside
  `ANativeActivity_onCreate`, so a `--profile dev` build aborts at startup
  before `android_main` runs. Release builds compile those checks out; the
  `[profile.android-dev]` entry in `Cargo.toml` keeps the same guarantee for
  debug builds (`PROFILE=dev ./build-apk.sh`, output in
  `target/android-dev/apk/`).
- The system status and navigation bars stay visible; the phone UI reads
  their insets (`WindowInsets` via JNI, re-checked about once a second) and
  pads the top app bar and bottom navigation so nothing sits underneath them.
- The desktop (glass, multi-pane) UI is still there: the Material 3 layout is
  only used on Android. On desktop nothing changes, and the same code paths are
  compiled with `#[cfg(target_os = "android")]`.
- Persistence (`library.json`, `theme.json`, thumbnails) lives in the app's
  private data directory (`XDG_CONFIG_HOME` is pointed at the Android internal
  data dir on startup).
- File dialogs (`.txt` import, folder scan, background image) have no Android
  backend, so those picker buttons are no-ops on Android; URLs and paths can
  still be pasted/typed directly.
- External `mpv`/`ffmpeg` are not present on Android, so the desktop playback
  and thumbnail-mosaic code is bypassed:
  - **Playback** fires an `android.intent.action.VIEW` intent, so videos open in
    the **system video player** (VLC, MX Player, …) and YouTube links open in
    the YouTube app / browser. Because a gallery app with no
    `android.permission.INTERNET` cannot stream a remote URL, Scwoll checks the
    installed `video/*` handlers: if the default one cannot reach the network it
    shows a chooser (when another player can) or hands the URL to the browser,
    which plays it inline.
  - **Thumbnails** for direct links come from `MediaMetadataRetriever`
    (embedded artwork, else a frame ~10% in), so the media cards still get a
    real preview. YouTube artwork keeps using the online thumbnail.
- Navigation is in-app: tap a card for its detail page (`← Back` returns to the
  feed), the bottom bar switches destination. The Android hardware back key is
  not intercepted (it is swallowed by the native-activity glue).

## Nix flake

This repository is a flake, so it can be used directly or imported into a
NixOS / home-manager configuration.

```bash
nix run .              # build & launch (from a checkout)
nix run github:Atlas-BES/scwoll
nix build .#scwoll     # ./result/bin/scwoll
nix develop            # dev shell (cargo, rustc, ffmpeg, mpv, GUI libs)
```

The packaged binary is wrapped with `mpv`, `ffmpeg`, `yt-dlp` and
`xdg-utils` on `PATH`, and with the GUI libraries (GL / Wayland / X11 /
fontconfig / dbus) on `LD_LIBRARY_PATH`, so winit can `dlopen` them at
runtime without any shell setup.

**As a flake input** (NixOS):

```nix
{
  inputs.scwoll.url = "github:Atlas-BES/scwoll";

  outputs = { self, nixpkgs, scwoll, ... }: {
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ({ pkgs, ... }: {
          environment.systemPackages = [
            scwoll.packages.${pkgs.stdenv.hostPlatform.system}.default
            # or: pkgs.scwoll  (with the overlay below)
          ];
        })
        # optional overlay: adds pkgs.scwoll
        { nixpkgs.overlays = [ scwoll.overlays.default ]; }
      ];
    };
  };
}
```

For home-manager, import the module and enable the option:

```nix
{
  inputs.scwoll.url = "github:Atlas-BES/scwoll";

  outputs = { self, nixpkgs, home-manager, scwoll, ... }: {
    homeConfigurations.me = home-manager.lib.homeManagerConfiguration {
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      modules = [
        scwoll.homeManagerModules.default
        ({ pkgs, ... }: {
          programs.scwoll = {
            enable = true;
            # Optional: use the flake's pinned/cached build instead of
            # rebuilding with your own nixpkgs.
            # package = scwoll.packages.${pkgs.stdenv.hostPlatform.system}.default;
          };
        })
      ];
    };
  };
}
```

The module provides:

- `programs.scwoll.enable` — install Scwoll (adds `programs.scwoll.package` to `home.packages`)
- `programs.scwoll.package` — package to install, defaulting to a build from the
  flake source with your own `pkgs`

The package ships a `.desktop` entry and a hicolor icon, so enabling the
module also adds Scwoll to your application launcher.

> Flakes only see files tracked by Git. If `nix build` complains about a
> missing module or source file, `git add` it first (or commit before
> pushing).

## Layout

```
Cargo.toml        # eframe/egui + rfd dialogs + ureq/image thumbs + Android metadata
src/
  main.rs         # egui app: library, theater, imports, themes, mpv player
  lib.rs          # Android cdylib entry (includes main.rs as a module)
  android.rs      # Android JNI: ACTION_VIEW playback + MediaMetadataRetriever thumbs
  mobile.rs       # Material You phone UI (Android): app bar, chips, cards, bottom nav, sheets
  model.rs        # Video/Theme types, YouTube-id parsing, title guessing (+ unit tests)
  system_blur.rs  # compositor blur request (Wayland ext-background-effect / X11)
android/res/      # Android launcher icons (mipmap-*)
build-apk.sh      # cargo-apk arm64 APK build helper
shell-android.nix # Android SDK + NDK + rustup + adb dev shell
sample_urls.txt   # sample bulk-import list
shell.nix         # cargo + eframe system libs (GL/wayland/xkb/gtk)
flake.nix         # flake outputs: packages / apps / devShells / overlay / HM module
nix/package.nix   # buildRustPackage derivation (wrapped runtime PATH + libs + desktop entry)
nix/devshell.nix  # `nix develop` environment
nix/hm-module.nix # home-manager module (`programs.scwoll.enable`)
~/.config/scwoll/ # library.json + theme.json + thumbs (created on first run)
```

## Tests

```bash
nix-shell --command "cargo test"
# youtube_ids · url_list_split · video_kinds — all ok
```

## Notes

- Folder import paths are local paths on the machine running the app (native file picker included).
- YouTube titles import as `YouTube • <id>` — rename them in the theater view.
- `egui` can't sample the desktop itself, so Liquid Glass normally uses
  translucent panels over a soft animated aurora backdrop, accent rims and
  layered shadows to simulate frosting. Enable **System blur** in
  `⚙ Settings → 🧊 Glass & motion` to ask the compositor for real backdrop
  blur; the **Blur intensity** slider controls how much of Scwoll's own
  backdrop is faded out to reveal it. Settings and Import are compact,
  flat top-bar dropdowns that use the same surface as the toolbar, so they
  read as an extension of the top panel. The blur radius itself is a desktop
  effect setting (e.g. KWin's Blur effect), not something an app can set.
