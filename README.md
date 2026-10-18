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
- **Themes** (`🎨 Theme`)
  - ✨ **Liquid Glass** — translucent panels, soft shadows, rounded corners
  - ◻ **Minimal** — opaque, sharp, flat
  - Fully customizable: accent, background, cards, inputs, text, muted, sidebar + corner radius + glass glow. Saved to `data/theme.json`
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
- `egui` can't do real backdrop blur, so Liquid Glass is simulated with translucency + shadows on a transparent viewport.
