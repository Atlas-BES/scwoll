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
  links). Cached in `data/thumbs/`; last resort is a letter tile
- **Themes & visual effects** (`⚙ Settings`)
  - **System font picker** (`🔤 Font`) — search and apply any font family
    installed via fontconfig; missing glyphs/emoji fall back to the bundled
    fonts and the choice is saved in `data/theme.json`
  - **Custom background image** (`🧊 Glass & motion`) — pick a PNG/JPEG,
    center-cropped to fill the window, with an image-dim slider and path
    saved in `data/theme.json`
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
    `data/theme.json`
- **Persistence** — `data/library.json` + `data/theme.json` (3 demo videos seeded on first run); **⚙ Settings → 📁 Data folder** opens the data directory in your file manager

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
Cargo.toml        # eframe/egui + rfd dialogs + ureq/image thumbs
src/
  main.rs         # egui app: library, theater, imports, themes, mpv player
  model.rs        # Video/Theme types, YouTube-id parsing, title guessing (+ unit tests)
  system_blur.rs  # compositor blur request (Wayland ext-background-effect / X11)
sample_urls.txt   # sample bulk-import list
shell.nix         # cargo + eframe system libs (GL/wayland/xkb/gtk)
flake.nix         # flake outputs: packages / apps / devShells / overlay / HM module
nix/package.nix   # buildRustPackage derivation (wrapped runtime PATH + libs + desktop entry)
nix/devshell.nix  # `nix develop` environment
nix/hm-module.nix # home-manager module (`programs.scwoll.enable`)
data/             # library.json + theme.json (created on first run)
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
