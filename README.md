# 📼 Scwoll — native YouTube-style media player in Rust

100% native desktop app. No browser, no server. Built with `egui`/`eframe`, playback via `mpv`.

## Features

- **YouTube-style UI** — top search bar, sidebar (Home / YouTube / Local / Direct), category chips, video-card grid, theater view + Up Next
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
  - **Presets** — ✨ Liquid Glass, Frosted, Neon, OLED, Minimal
  - **Liquid glass** toggle with panel opacity, corner radius, card size,
    shadow/depth and accent glow
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
- **Persistence** — `data/library.json` + `data/theme.json` (3 demo videos seeded on first run)

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

## Layout

```
Cargo.toml        # eframe/egui + rfd dialogs + ureq/image thumbs
src/
  main.rs         # egui app: library, theater, imports, themes, mpv player
  model.rs        # Video/Theme types, YouTube-id parsing, title guessing (+ unit tests)
sample_urls.txt   # sample bulk-import list
shell.nix         # cargo + eframe system libs (GL/wayland/xkb/gtk)
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
- `egui` can't do real backdrop blur, so Liquid Glass uses translucent panels
  over a soft animated aurora backdrop, accent rims and layered shadows to
  simulate frosting. The **Frost / blur** slider controls how wide and soft
  that simulated frosting looks.
