//! Scwoll — native YouTube-style media player in Rust. No browser, no server.
//! Library (URLs / .txt lists / folders) + categories + glass/minimal themes.
//! Playback is handled by mpv (must be installed), controlled from this UI.

mod model;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant},
};

use eframe::egui;
use model::*;

// ------------------------------------------------------------ persistence --
fn data_dir() -> PathBuf {
    PathBuf::from("data")
}
fn lib_path() -> PathBuf {
    data_dir().join("library.json")
}
fn theme_path() -> PathBuf {
    data_dir().join("theme.json")
}

fn load_videos() -> Vec<Video> {
    std::fs::create_dir_all(data_dir()).ok();
    if let Ok(s) = std::fs::read_to_string(lib_path()) {
        if let Ok(lib) = serde_json::from_str::<Library>(&s) {
            return lib.videos;
        }
    }
    let demo = demo_videos();
    save_videos(&demo);
    demo
}
fn save_videos(videos: &[Video]) {
    let lib = Library { videos: videos.to_vec() };
    if let Ok(s) = serde_json::to_string_pretty(&lib) {
        std::fs::write(lib_path(), s).ok();
    }
}
fn load_theme() -> Theme {
    std::fs::read_to_string(theme_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Theme>(&s).ok())
        .unwrap_or_default()
}
fn save_theme(theme: &Theme) {
    if let Ok(s) = serde_json::to_string_pretty(theme) {
        std::fs::write(theme_path(), s).ok();
    }
}

// ---------------------------------------------------------------- colors --
fn hex_to_color(hex: &str) -> egui::Color32 {
    let h = hex.trim().trim_start_matches('#');
    let (r, g, b) = match h.len() {
        6 => (
            u8::from_str_radix(&h[0..2], 16).unwrap_or(255),
            u8::from_str_radix(&h[2..4], 16).unwrap_or(255),
            u8::from_str_radix(&h[4..6], 16).unwrap_or(255),
        ),
        3 => {
            let r = u8::from_str_radix(&h[0..1], 16).unwrap_or(15) * 17;
            let g = u8::from_str_radix(&h[1..2], 16).unwrap_or(15) * 17;
            let b = u8::from_str_radix(&h[2..3], 16).unwrap_or(15) * 17;
            (r, g, b)
        }
        _ => (255, 255, 255),
    };
    egui::Color32::from_rgb(r, g, b)
}
fn color_to_hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}
fn with_alpha(c: egui::Color32, a: u8, glass: bool) -> egui::Color32 {
    if glass {
        egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    } else {
        c
    }
}
fn placeholder_color(key: &str) -> egui::Color32 {
    let mut h: u32 = 7;
    for b in key.bytes() {
        h = h.wrapping_mul(31).wrapping_add(b as u32);
    }
    let palette = [
        (178, 62, 62),
        (62, 120, 178),
        (62, 178, 120),
        (150, 110, 200),
        (210, 140, 60),
        (60, 170, 190),
        (200, 80, 140),
        (110, 150, 70),
    ];
    let (r, g, b) = palette[(h as usize) % palette.len()];
    egui::Color32::from_rgb(r, g, b)
}

/// Shorten a title to roughly two lines at the given card width so every
/// box stays the same height and shelf rows line up evenly.
fn shorten_title(title: &str, card_w: f32) -> String {
    let per_line = (card_w / 7.0).floor() as usize;
    let max = (per_line * 2).max(20);
    let chars: Vec<char> = title.chars().collect();
    if chars.len() <= max {
        title.to_string()
    } else {
        chars[..max].iter().collect::<String>() + "\u{2026}"
    }
}


fn apply_visuals(ctx: &egui::Context, theme: &Theme) {
    let glass = theme.style == "glass";
    let accent = hex_to_color(&theme.accent);
    let bg = hex_to_color(&theme.background);
    let surface = hex_to_color(&theme.surface);
    let surface2 = hex_to_color(&theme.surface2);
    let text = hex_to_color(&theme.text);
    let muted = hex_to_color(&theme.muted);
    let sidebar = hex_to_color(&theme.sidebar);

    let bg_fill = with_alpha(bg, 235, glass);
    let panel = with_alpha(surface, if glass { 205 } else { 255 }, glass);
    let side = with_alpha(sidebar, if glass { 215 } else { 255 }, glass);
    let _input = with_alpha(surface2, if glass { 220 } else { 255 }, glass);
    let r = egui::Rounding::same(theme.radius.min(24) as f32);

    let mut vis = egui::Visuals::dark();
    vis.dark_mode = true;
    vis.override_text_color = Some(text);
    vis.window_fill = panel;
    vis.panel_fill = bg_fill;
    vis.extreme_bg_color = with_alpha(surface2, 255, false);
    vis.code_bg_color = with_alpha(surface2, 255, false);
    vis.faint_bg_color = with_alpha(surface2, 255, false);
    vis.window_stroke = egui::Stroke::new(1.0, muted.gamma_multiply(0.45));
    vis.widgets.noninteractive.bg_fill = panel;
    vis.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text);
    vis.widgets.inactive.bg_fill = with_alpha(surface2, 255, false);
    vis.widgets.hovered.bg_fill = accent.gamma_multiply(0.35);
    vis.widgets.active.bg_fill = accent.gamma_multiply(0.55);
    vis.selection.bg_fill = accent.gamma_multiply(0.85);
    vis.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    vis.hyperlink_color = accent;
    vis.window_rounding = r;
    vis.menu_rounding = r;
    vis.widgets.noninteractive.rounding = r;
    vis.widgets.inactive.rounding = r;
    vis.widgets.hovered.rounding = r;
    vis.widgets.active.rounding = r;
    vis.widgets.open.rounding = r;
    vis.window_shadow = if glass {
        egui::Shadow {
            offset: egui::vec2(0.0, 12.0),
            blur: 40.0,
            spread: 0.0,
            color: egui::Color32::from_black_alpha(120),
        }
    } else {
        egui::Shadow::NONE
    };
    let _ = side;
    ctx.set_visuals(vis);
}

// ---------------------------------------------------------------- player ---
struct Player {
    child: Option<Child>,
    current_id: Option<String>,
    status: String,
}
impl Player {
    fn new() -> Self {
        Self { child: None, current_id: None, status: "Idle — pick a video and press Play.".into() }
    }
    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            c.kill().ok();
            c.wait().ok();
        }
        self.current_id = None;
    }
    fn is_playing(&mut self, id: &str) -> bool {
        if self.current_id.as_deref() != Some(id) {
            return false;
        }
        match self.child.as_mut() {
            Some(c) => match c.try_wait() {
                Ok(None) => true, // still running
                _ => {
                    self.child.take();
                    false
                }
            },
            None => false,
        }
    }
    fn play(&mut self, v: &Video) {
        self.stop();
        let target = v.file_path.clone().unwrap_or_else(|| v.url.clone());
        if target.is_empty() {
            self.status = "Nothing to play (empty path/URL).".into();
            return;
        }
        if v.kind == VideoKind::Local && !Path::new(&target).exists() {
            self.status = format!("File missing on disk:\n{target}");
            return;
        }
        match Command::new("mpv")
            .args(["--no-terminal", "--force-window=yes", "--title=scwoll", &target])
            .spawn()
        {
            Ok(c) => {
                self.child = Some(c);
                self.current_id = Some(v.id.clone());
                self.status = format!("▶ Playing in mpv:\n{}", v.title);
            }
            Err(e) => {
                self.status = format!("Could not launch mpv ({e}).\nInstall mpv to play videos.");
            }
        }
    }
}

// -------------------------------------------------------------- thumbnails --
// Priority: disk cache (data/thumbs/<id>.jpg) -> YouTube thumbnail image ->
// 6-frame mosaic spread across the whole video (the only fallback).
// Anything that still has nothing keeps the letter tile.
enum ThumbMsg {
    Done { key: String, image: egui::ColorImage },
    Failed { key: String },
}

fn thumb_cache_file(key: &str) -> PathBuf {
    data_dir().join("thumbs").join(format!("{key}.jpg"))
}

fn decode_thumb_bytes(bytes: &[u8]) -> Option<egui::ColorImage> {
    if bytes.is_empty() || bytes.len() > 8_000_000 {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    // Never crop: keep aspect (mosaics are 3:2, artwork is 16:9/4:3).
    // Only downscale absurdly large images; the card letterboxes the rest.
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let rgba = if (w as u64) * (h as u64) > 1280 * 720 {
        img.resize(1280, 720, image::imageops::FilterType::Triangle).to_rgba8()
    } else {
        rgba
    };
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()))
}

fn download_bytes(url: &str) -> Option<Vec<u8>> {
    let resp = ureq::get(url).call().ok()?;
    let len: usize = resp
        .header("Content-Length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(300_000);
    if len > 4_000_000 {
        return None;
    }
    let mut bytes = Vec::with_capacity(len.min(1_000_000));
    resp.into_reader().read_to_end(&mut bytes).ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some(bytes)
}

/// Duration in seconds via ffprobe (None for streams it can't probe,
/// e.g. YouTube URLs without yt-dlp handling).
fn video_duration(input: &str) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            input,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|d| *d > 0.0)
}

/// 6 frames equally spaced across the whole video, square-cropped and
/// tiled 3x2 into one 480x320 mosaic JPEG.
/// Engine 1: ffprobe duration + ffmpeg seeks (fast, precise).
/// Engine 2: mpv percent-seeks (needs only mpv — works outside nix-shell
/// and for anything mpv can open, incl. YouTube URLs).
fn grab_mosaic_bytes(input: &str) -> Option<Vec<u8>> {
    if let Some(b) = grab_mosaic_ffmpeg(input) {
        return Some(b);
    }
    eprintln!("[scwoll] thumb: ffmpeg mosaic failed for {input}, trying mpv");
    grab_mosaic_mpv(input)
}

fn grab_mosaic_ffmpeg(input: &str) -> Option<Vec<u8>> {
    let dur = video_duration(input)?;
    let dir = std::env::temp_dir().join(format!("scwoll-mosaic-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok()?;
    let mut cells: Vec<PathBuf> = vec![];
    for i in 0..6 {
        let t = dur * (i as f64 + 0.5) / 6.0;
        let out = dir.join(format!("c{i}.jpg"));
        let ok = Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-ss",
                &format!("{t:.2}"),
                "-i",
                input,
                "-vframes",
                "1",
                "-vf",
                "scale=160:160:force_original_aspect_ratio=increase,crop=160:160",
                "-q:v",
                "5",
            ])
            .arg(&out)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok || std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) == 0 {
            std::fs::remove_dir_all(&dir).ok();
            return None;
        }
        cells.push(out);
    }
    let mut canvas = image::RgbImage::new(480, 320);
    for (i, p) in cells.iter().enumerate() {
        let cell = image::open(p).ok()?.to_rgb8();
        if cell.width() != 160 || cell.height() != 160 {
            std::fs::remove_dir_all(&dir).ok();
            return None;
        }
        image::imageops::replace(&mut canvas, &cell, ((i % 3) * 160) as i64, ((i / 3) * 160) as i64);
    }
    let mosaic_path = dir.join("mosaic.jpg");
    image::DynamicImage::ImageRgb8(canvas).save(&mosaic_path).ok()?;
    let bytes = std::fs::read(&mosaic_path).ok()?;
    std::fs::remove_dir_all(&dir).ok();
    if bytes.is_empty() {
        return None;
    }
    Some(bytes)
}

/// Mosaic engine 2: 6 mpv screenshots at evenly spaced percents of the
/// video, square-cropped in-process. Needs only mpv (no ffprobe/ffmpeg).
fn grab_mosaic_mpv(input: &str) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("scwoll-mosaic-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok()?;
    let mut cells: Vec<PathBuf> = vec![];
    for i in 0..6 {
        let pct = (i as f64 + 0.5) / 6.0 * 100.0;
        let celldir = dir.join(format!("c{i}"));
        std::fs::create_dir_all(&celldir).ok()?;
        let ok = Command::new("mpv")
            .arg("--no-terminal")
            .arg("--really-quiet")
            .arg("--no-audio")
            .arg("--frames=1")
            .arg("--network-timeout=15")
            .arg(format!("--start={pct:.2}%"))
            .arg("--vo=image")
            .arg("--vo-image-format=jpg")
            .arg("--vo-image-jpeg-quality=75")
            .arg(format!("--vo-image-outdir={}", celldir.display()))
            .arg(input)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let shot = ok
            .then(|| {
                std::fs::read_dir(&celldir).ok()?.flatten().map(|e| e.path()).find(|p| {
                    p.extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x.eq_ignore_ascii_case("jpg"))
                        .unwrap_or(false)
                })
            })
            .flatten();
        match shot {
            // square-crop the screenshot to a 160x160 cell in-process
            Some(p) => {
                let sq = image::open(&p)
                    .ok()?
                    .resize_to_fill(160, 160, image::imageops::FilterType::Triangle);
                let cell = dir.join(format!("cell{i}.jpg"));
                sq.save(&cell).ok()?;
                cells.push(cell);
            }
            None => {
                eprintln!("[scwoll] thumb: mpv shot {i}/6 failed for {input}");
                std::fs::remove_dir_all(&dir).ok();
                return None;
            }
        }
    }
    let mut canvas = image::RgbImage::new(480, 320);
    for (i, p) in cells.iter().enumerate() {
        let cell = image::open(p).ok()?.to_rgb8();
        image::imageops::replace(&mut canvas, &cell, ((i % 3) * 160) as i64, ((i / 3) * 160) as i64);
    }
    let mosaic_path = dir.join("mosaic.jpg");
    image::DynamicImage::ImageRgb8(canvas).save(&mosaic_path).ok()?;
    let bytes = std::fs::read(&mosaic_path).ok()?;
    std::fs::remove_dir_all(&dir).ok();
    if bytes.is_empty() {
        return None;
    }
    Some(bytes)
}

fn load_thumb_bytes(v: &Video) -> Option<Vec<u8>> {
    let cache = thumb_cache_file(&v.id);
    if let Ok(b) = std::fs::read(&cache) {
        if !b.is_empty() {
            return Some(b);
        }
    }
    // YouTube thumbnail image
    if !v.thumbnail.is_empty() {
        if let Some(b) = download_bytes(&v.thumbnail) {
            if let Some(parent) = cache.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&cache, &b).ok();
            return Some(b);
        }
    }
    // 6-frame mosaic spread across the whole video — the only fallback.
    // Anything it can't handle keeps the letter tile.
    let input = v.file_path.clone().unwrap_or_else(|| v.url.clone());
    if input.trim().is_empty() {
        return None;
    }
    if v.kind == VideoKind::Local && !Path::new(&input).exists() {
        return None;
    }
    if let Some(b) = grab_mosaic_bytes(&input) {
        if let Some(parent) = cache.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&cache, &b).ok();
        return Some(b);
    }
    eprintln!("[scwoll] thumb: no artwork and no mosaic for {input} (keeps letter tile)");
    None
}

// use Read trait in fetch
use std::io::Read as _;

// --------------------------------------------------------------- app -------
#[derive(Debug, Clone, Copy, PartialEq)]
enum SourceFilter {
    All,
    Youtube,
    Local,
    Direct,
}
impl SourceFilter {
    fn label(self) -> &'static str {
        match self {
            SourceFilter::All => "Home",
            SourceFilter::Youtube => "YouTube",
            SourceFilter::Local => "Local files",
            SourceFilter::Direct => "Direct links",
        }
    }
    fn matches(self, k: &VideoKind) -> bool {
        match self {
            SourceFilter::All => true,
            SourceFilter::Youtube => *k == VideoKind::Youtube,
            SourceFilter::Local => *k == VideoKind::Local,
            SourceFilter::Direct => *k == VideoKind::Direct,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ImportTab {
    Url,
    List,
    Folder,
}

struct ScwollApp {
    videos: Vec<Video>,
    theme: Theme,
    active_category: String,
    source: SourceFilter,
    search: String,
    selected: Option<String>,
    show_import: bool,
    show_theme: bool,
    import_tab: ImportTab,
    // import buffers
    in_url: String,
    in_url_cat: String,
    in_url_title: String,
    in_bulk: String,
    in_bulk_cat: String,
    in_folder: String,
    in_folder_cat: String,
    in_recursive: bool,
    // theater edit buffers
    edit_title: String,
    edit_cat: String,
    player: Player,
    toast: Option<(String, Instant)>,
    // thumbnails
    thumb_tx: Sender<ThumbMsg>,
    thumb_rx: Receiver<ThumbMsg>,
    thumb_cache: HashMap<String, egui::TextureHandle>,
    thumb_pending: HashSet<String>,
}

impl ScwollApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        Self {
            videos: load_videos(),
            theme: load_theme(),
            active_category: "All".into(),
            source: SourceFilter::All,
            search: String::new(),
            selected: None,
            show_import: false,
            show_theme: false,
            import_tab: ImportTab::Url,
            in_url: String::new(),
            in_url_cat: String::new(),
            in_url_title: String::new(),
            in_bulk: String::new(),
            in_bulk_cat: String::new(),
            in_folder: String::new(),
            in_folder_cat: String::new(),
            in_recursive: true,
            edit_title: String::new(),
            edit_cat: String::new(),
            player: Player::new(),
            toast: None,
            thumb_tx: tx,
            thumb_rx: rx,
            thumb_cache: HashMap::new(),
            thumb_pending: HashSet::new(),
        }
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }
    fn persist(&self) {
        save_videos(&self.videos);
    }

    fn categories(&self) -> Vec<(String, usize)> {
        let mut map: BTreeMap<String, usize> = BTreeMap::new();
        for v in &self.videos {
            *map.entry(v.category.clone()).or_default() += 1;
        }
        map.into_iter().collect()
    }

    fn filtered(&self) -> Vec<&Video> {
        let q = self.search.trim().to_lowercase();
        let mut out: Vec<&Video> = self
            .videos
            .iter()
            .filter(|v| {
                if self.active_category != "All" && v.category != self.active_category {
                    return false;
                }
                if !self.source.matches(&v.kind) {
                    return false;
                }
                if !q.is_empty() {
                    let hay = format!("{} {} {}", v.title, v.category, v.url).to_lowercase();
                    if !hay.contains(&q) {
                        return false;
                    }
                }
                true
            })
            .collect();
        out.sort_by(|a, b| b.added_at.cmp(&a.added_at));
        out
    }

    fn by_id(&self, id: &str) -> Option<Video> {
        self.videos.iter().find(|v| v.id == id).cloned()
    }

    fn pump_thumbs(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.thumb_rx.try_recv() {
            match msg {
                ThumbMsg::Done { key, image } => {
                    self.thumb_pending.remove(&key);
                    let tex =
                        ctx.load_texture(format!("thumb-{key}"), image, egui::TextureOptions::LINEAR);
                    self.thumb_cache.insert(key, tex);
                }
                ThumbMsg::Failed { key } => {
                    // tried once (disk -> page thumb -> first frame); keep letter tile.
                    self.thumb_pending.remove(&key);
                }
            }
        }
    }

    fn request_thumb(&mut self, v: &Video) {
        if self.thumb_cache.contains_key(&v.id) || self.thumb_pending.contains(&v.id) {
            return;
        }
        self.thumb_pending.insert(v.id.clone());
        let tx = self.thumb_tx.clone();
        let vid = v.clone();
        std::thread::spawn(move || {
            let msg = match load_thumb_bytes(&vid).and_then(|b| decode_thumb_bytes(&b)) {
                Some(image) => ThumbMsg::Done { key: vid.id, image },
                None => ThumbMsg::Failed { key: vid.id },
            };
            tx.send(msg).ok();
        });
    }

    // ---------------- imports ----------------
    fn import_url(&mut self, url: &str, cat: &str, title: &str) -> Result<(), String> {
        let url = url.trim();
        if url.is_empty() {
            return Err("Paste a URL first.".into());
        }
        if self.videos.iter().any(|x| x.url == url) {
            return Err("Already in library.".into());
        }
        let mut v = make_video_from_url(url, normalize_category(Some(cat.to_string())));
        if !title.trim().is_empty() {
            v.title = title.trim().to_string();
        }
        self.videos.push(v);
        self.persist();
        Ok(())
    }

    fn import_bulk(&mut self, text: &str, cat: &str) -> (usize, usize) {
        let cat = normalize_category(Some(cat.to_string()));
        let mut added = 0;
        let mut skipped = 0;
        for u in split_url_list(text) {
            if self.videos.iter().any(|x| x.url == u) {
                skipped += 1;
                continue;
            }
            self.videos.push(make_video_from_url(&u, cat.clone()));
            added += 1;
        }
        self.persist();
        (added, skipped)
    }

    fn import_folder(&mut self, path: &str, cat: &str, recursive: bool) -> Result<usize, String> {
        let root = PathBuf::from(path.trim());
        if !root.exists() {
            return Err("Folder does not exist.".into());
        }
        let cat = if cat.trim().is_empty() {
            root.file_name().and_then(|x| x.to_str()).unwrap_or("Local").to_string()
        } else {
            cat.trim().to_string()
        };
        let files = scan_folder(&root, recursive);
        if files.is_empty() {
            return Err("No video files found (mp4/mkv/webm/mov/avi…).".into());
        }
        let mut added = 0;
        for f in files {
            let abs = f.to_string_lossy().to_string();
            if self.videos.iter().any(|x| x.file_path.as_deref() == Some(abs.as_str())) {
                continue;
            }
            self.videos.push(make_video_from_file(&f, cat.clone()));
            added += 1;
        }
        self.persist();
        Ok(added)
    }
}

fn scan_folder(root: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if recursive {
                    stack.push(p);
                }
                continue;
            }
            if let Some(ext) = p.extension().and_then(|x| x.to_str()) {
                if VIDEO_EXTS.contains(&ext.to_lowercase().as_str()) {
                    out.push(p);
                }
            }
        }
        if !recursive {
            break;
        }
    }
    out.sort();
    out
}

// --------------------------------------------------------------- eframe ----
impl eframe::App for ScwollApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        apply_visuals(ctx, &self.theme);
        self.pump_thumbs(ctx);
        ctx.request_repaint_after(Duration::from_millis(500));

        let glass = self.theme.style == "glass";
        let accent = hex_to_color(&self.theme.accent);
        let muted = hex_to_color(&self.theme.muted);
        let sidebar_fill = with_alpha(hex_to_color(&self.theme.sidebar), if glass { 215 } else { 255 }, glass);

        // ---------------- top bar ----------------
        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("▶ Scwoll").color(accent).strong());
                ui.label(egui::RichText::new("tube").color(muted));
                ui.add_space(8.0);
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search videos, categories, urls…")
                        .desired_width(320.0),
                );
                if resp.changed() {}
                if ui.button("✕").on_hover_text("Clear search").clicked() {
                    self.search.clear();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🎨 Theme").clicked() {
                        self.show_theme = true;
                    }
                    if ui.button("＋ Import").clicked() {
                        self.show_import = true;
                    }
                });
            });
        });

        // ---------------- sidebar ----------------
        egui::SidePanel::left("sidebar")
            .exact_width(228.0)
            .frame(egui::Frame::side_top_panel(&ctx.style()).fill(sidebar_fill))
            .show(ctx, |ui| {
                ui.heading("Library");
                for s in [SourceFilter::All, SourceFilter::Youtube, SourceFilter::Local, SourceFilter::Direct] {
                    let label = match s {
                        SourceFilter::All => format!("🏠 {}", s.label()),
                        SourceFilter::Youtube => format!("▶ {}", s.label()),
                        SourceFilter::Local => format!("🗂 {}", s.label()),
                        SourceFilter::Direct => format!("🔗 {}", s.label()),
                    };
                    if ui.selectable_label(self.source == s, label).clicked() {
                        self.source = s;
                        self.selected = None;
                    }
                }
                ui.separator();
                ui.heading("Categories");
                let cats = self.categories();
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    if ui.selectable_label(self.active_category == "All", format!("All ({})", self.videos.len())).clicked() {
                        self.active_category = "All".into();
                    }
                    for (name, count) in &cats {
                        if ui.selectable_label(self.active_category == *name, format!("{name} ({count})")).clicked() {
                            self.active_category = name.clone();
                        }
                    }
                    if cats.is_empty() {
                        ui.label(egui::RichText::new("No categories yet — import something!").color(muted).small());
                    }
                });
                ui.separator();
                let (yt, lo, di) = (
                    self.videos.iter().filter(|v| v.kind == VideoKind::Youtube).count(),
                    self.videos.iter().filter(|v| v.kind == VideoKind::Local).count(),
                    self.videos.iter().filter(|v| v.kind == VideoKind::Direct).count(),
                );
                ui.small(format!("📼 {} total · ▶ {yt} · 🗂 {lo} · 🔗 {di}", self.videos.len()));
                ui.small(format!("Player: {}", if self.player.child.is_some() { "busy" } else { "idle" }));
            });

        // ---------------- main ----------------
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(id) = self.selected.clone() {
                self.show_theater(ui, &id, accent, muted);
            } else {
                self.show_grid(ui, accent, muted);
            }
        });

        if self.show_import {
            self.import_window(ctx);
        }
        if self.show_theme {
            self.theme_window(ctx, accent, muted);
        }

        // toast
        if let Some((msg, t)) = self.toast.clone() {
            if t.elapsed() < Duration::from_secs(3) {
                egui::TopBottomPanel::bottom("toast").show(ctx, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(msg).strong());
                    });
                });
            } else {
                self.toast = None;
            }
        }
    }
}

// ----------------------------------------------------------------- views --
impl ScwollApp {
    fn show_grid(&mut self, ui: &mut egui::Ui, accent: egui::Color32, muted: egui::Color32) {
        // chips
        let mut cats: Vec<String> = vec!["All".into()];
        cats.extend(self.categories().into_iter().map(|(n, _)| n));
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for c in &cats {
                    if ui.selectable_label(&self.active_category == c, c).clicked() {
                        self.active_category = c.clone();
                    }
                }
            });
        });
        ui.separator();

        let list: Vec<Video> = self.filtered().into_iter().cloned().collect();
        ui.horizontal(|ui| {
            ui.heading(if self.active_category == "All" { self.source.label() } else { self.active_category.as_str() });
            ui.label(egui::RichText::new(format!("{} videos", list.len())).color(muted));
        });

        if list.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("📼 Nothing here yet");
                ui.label("Import a URL, a .txt list, or a folder of videos.");
                if ui.button("＋ Import videos").clicked() {
                    self.show_import = true;
                }
            });
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // wrapping flow: cards that don't fit flow onto the next row,
            // so nothing ever runs off the window edge.
            let avail = ui.available_width();
            let card_w = (300.0 * self.theme.card_scale).clamp(140.0, avail.max(140.0));
            let thumb_size = egui::vec2(card_w, card_w * 9.0 / 16.0);
            let rounding = egui::Rounding::same(self.theme.radius.min(20) as f32);

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 12.0);
                for v in list.iter() {
                        let vid = v.clone();
                        self.request_thumb(v);
                        let mut opened = false;
                        let mut play = false;
                        let mut delete = false;
                        // ============ video box ============
                        // stock widgets only: no custom painting, nothing can overlap.
                        // top row (source left, category right) / thumbnail /
                        // one horizontal button row / title.
                        egui::Frame::group(ui.style())
                            .rounding(rounding)
                            .inner_margin(egui::Margin::same(6.0))
                            .show(ui, |ui| {
                                ui.set_width(card_w);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                // Force vertical stacking inside the box: the wrapping
                                // row outside hands down a HORIZONTAL layout, which is
                                // what glued the buttons beside the thumbnail and threw
                                // the second box lower (rows center uneven heights).
                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                ui.set_width(card_w);
                                let src = match vid.kind {
                                    VideoKind::Youtube => "\u{25b6} YouTube",
                                    VideoKind::Local => "\u{1F5C2} Local",
                                    VideoKind::Direct => "\u{1F517} Link",
                                };
                                // top row of the box: source + category adjacent, no gap
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(egui::RichText::new(src).color(muted).small());
                                    ui.label(egui::RichText::new("\u{00b7}").color(muted).small());
                                    ui.label(
                                        egui::RichText::new(vid.category.clone())
                                            .color(accent)
                                            .small()
                                            .strong(),
                                    );
                                });
                                ui.add_space(2.0);
                                // thumbnail (click to open); shown aspect-correct so the
                                // 3:2 frame mosaic isn't stretched into the 16:9 slot.
                                let thumb_clicked = if let Some(tex) = self.thumb_cache.get(&vid.id) {
                                    let ts = tex.size();
                                    let ts = egui::vec2(ts[0] as f32, ts[1].max(1) as f32);
                                    let s = (ts.x / ts.y.max(1.0)).clamp(0.2, 4.0);
                                    let slot_s = thumb_size.x / thumb_size.y;
                                    let disp = if s > slot_s {
                                        egui::vec2(thumb_size.x, thumb_size.x / s)
                                    } else {
                                        egui::vec2(thumb_size.y * s, thumb_size.y)
                                    };
                                    ui.allocate_ui_with_layout(
                                        thumb_size,
                                        egui::Layout::top_down(egui::Align::Center),
                                        |ui| {
                                            ui.add(
                                                egui::Image::from_texture(tex)
                                                    .fit_to_exact_size(disp)
                                                    .sense(egui::Sense::click()),
                                            )
                                        },
                                    )
                                    .inner
                                    .clicked()
                                } else {
                                    let letter = vid
                                        .title
                                        .chars()
                                        .next()
                                        .unwrap_or('\u{25b6}')
                                        .to_uppercase()
                                        .to_string();
                                    ui.add_sized(
                                        thumb_size,
                                        egui::Button::new(
                                            egui::RichText::new(letter)
                                                .size(44.0)
                                                .color(egui::Color32::WHITE),
                                        )
                                        .fill(placeholder_color(&vid.id)),
                                    )
                                    .clicked()
                                };
                                if thumb_clicked {
                                    opened = true;
                                }
                                // one horizontal button row directly under the image;
                                // exact thirds of the available width so it always fits.
                                ui.add_space(6.0);
                                let gap = ui.spacing().item_spacing.x;
                                let bw = (ui.available_width() - gap * 2.0) / 3.0;
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_sized(
                                            egui::vec2(bw, 24.0),
                                            egui::Button::new(
                                                egui::RichText::new("\u{25b6} Play")
                                                    .color(egui::Color32::WHITE),
                                            )
                                            .fill(accent),
                                        )
                                        .clicked()
                                    {
                                        play = true;
                                    }
                                    if ui
                                        .add_sized(egui::vec2(bw, 24.0), egui::Button::new("Open"))
                                        .clicked()
                                    {
                                        opened = true;
                                    }
                                    if ui
                                        .add_sized(egui::vec2(bw, 24.0), egui::Button::new("\u{1F5D1}"))
                                        .on_hover_text("Delete")
                                        .clicked()
                                    {
                                        delete = true;
                                    }
                                });
                                // title: fixed-height box so every card is exactly
                                // the same height — shelf rows stay level, nothing
                                // shifts down.
                                ui.add_space(4.0);
                                let title_w = ui.available_width().max(50.0);
                                ui.allocate_ui_with_layout(
                                    egui::vec2(title_w, 36.0),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(shorten_title(&vid.title, card_w))
                                                .strong(),
                                        );
                                    },
                                );
                                });
                            });
                        if opened {
                            self.selected = Some(vid.id.clone());
                            self.edit_title = vid.title.clone();
                            self.edit_cat = vid.category.clone();
                        }
                        if play {
                            self.player.play(&vid);
                            self.toast(format!("Playing: {}", vid.title));
                        }
                        if delete {
                            self.videos.retain(|x| x.id != vid.id);
                            self.persist();
                        }
                    }
                });
        });
    }

    fn show_theater(&mut self, ui: &mut egui::Ui, id: &str, accent: egui::Color32, muted: egui::Color32) {
        let Some(v) = self.by_id(id) else {
            self.selected = None;
            return;
        };
        if ui.button("← Back to library").clicked() {
            self.selected = None;
        }
        ui.add_space(6.0);

        // player screen (mpv plays in its own window; this is the controller)
        egui::Frame::group(ui.style()).fill(egui::Color32::BLACK).show(ui, |ui| {
            ui.set_min_height(300.0);
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                let playing = self.player.is_playing(&v.id);
                ui.label(egui::RichText::new(if playing { "▶ NOW PLAYING (mpv window)" } else { "🎬 READY" }).color(egui::Color32::WHITE).heading());
                ui.label(egui::RichText::new(v.title.clone()).color(egui::Color32::WHITE).strong());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(if playing { "⏸ Stop" } else { "▶ Play in mpv" }).clicked() {
                        if playing {
                            self.player.stop();
                            self.player.status = "Stopped.".into();
                        } else {
                            self.player.play(&v);
                        }
                    }
                    if ui.button("↗ Reveal / copy URL").clicked() {
                        if v.kind == VideoKind::Local {
                            if let Some(p) = v.file_path.clone() {
                                open::that(Path::new(&p).parent().map(|x| x.to_path_buf()).unwrap_or(PathBuf::from(&p))).ok();
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.label(egui::RichText::new(self.player.status.clone()).color(egui::Color32::LIGHT_GRAY).small());
            });
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(v.category.clone()).color(accent).strong());
            ui.label(egui::RichText::new(match v.kind {
                VideoKind::Youtube => "▶ YouTube",
                VideoKind::Local => "🗂 Local file",
                VideoKind::Direct => "🔗 Direct link",
            }).color(muted));
        });
        ui.add(egui::TextEdit::singleline(&mut self.edit_title).hint_text("Title").desired_width(f32::INFINITY));
        ui.horizontal(|ui| {
            ui.label("Category:");
            ui.add(egui::TextEdit::singleline(&mut self.edit_cat).hint_text("e.g. Music").desired_width(200.0));
            if ui.button("Save").clicked() {
                if let Some(slot) = self.videos.iter_mut().find(|x| x.id == v.id) {
                    if !self.edit_title.trim().is_empty() {
                        slot.title = self.edit_title.trim().to_string();
                    }
                    slot.category = normalize_category(Some(self.edit_cat.clone()));
                    self.edit_cat = slot.category.clone();
                }
                self.persist();
                self.toast("Saved ✓");
            }
            if ui.button("🗑 Delete").clicked() {
                self.videos.retain(|x| x.id != v.id);
                self.player.stop();
                self.persist();
                self.selected = None;
                return;
            }
        });
        ui.small(format!("Source: {}", if v.kind == VideoKind::Local { v.file_path.clone().unwrap_or(v.url) } else { v.url }));
        ui.separator();
        ui.heading("Up next");
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                let rest: Vec<Video> = self.videos.iter().filter(|x| x.id != v.id).take(8).cloned().collect();
                for x in rest {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(200.0);
                        ui.label(egui::RichText::new(x.title.clone()).strong());
                        ui.small(x.category.clone());
                        if ui.small_button("Open").clicked() {
                            self.selected = Some(x.id.clone());
                            self.edit_title = x.title.clone();
                            self.edit_cat = x.category.clone();
                        }
                    });
                }
            });
        });
    }

    // ------------------------------------------------------------ windows --
    fn import_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_import;
        egui::Window::new("＋ Import videos")
            .open(&mut open)
            .resizable(true)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.import_tab, ImportTab::Url, "Single URL");
                    ui.selectable_value(&mut self.import_tab, ImportTab::List, "List / .txt");
                    ui.selectable_value(&mut self.import_tab, ImportTab::Folder, "Folder");
                });
                ui.separator();
                match self.import_tab {
                    ImportTab::Url => {
                        ui.label("Video URL (YouTube, mp4, …)");
                        ui.add(egui::TextEdit::singleline(&mut self.in_url).hint_text("https://www.youtube.com/watch?v=…").desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {
                            ui.label("Category:");
                            ui.add(egui::TextEdit::singleline(&mut self.in_url_cat).hint_text("General").desired_width(140.0));
                        });
                        ui.label("Title override (optional):");
                        ui.add(egui::TextEdit::singleline(&mut self.in_url_title).hint_text("blank = auto").desired_width(f32::INFINITY));
                        if ui.button("Add video").clicked() {
                            match self.import_url(&self.in_url.clone(), &self.in_url_cat.clone(), &self.in_url_title.clone()) {
                                Ok(()) => {
                                    self.toast("Added ✓");
                                    self.in_url.clear();
                                    self.in_url_title.clear();
                                }
                                Err(e) => self.toast(e),
                            }
                        }
                    }
                    ImportTab::List => {
                        ui.label("Paste URLs (one per line) or load a .txt file. # = comment.");
                        ui.add(egui::TextEdit::multiline(&mut self.in_bulk).hint_text("https://…\nhttps://…").desired_rows(6).desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {
                            ui.label("Category:");
                            ui.add(egui::TextEdit::singleline(&mut self.in_bulk_cat).hint_text("General").desired_width(140.0));
                            if ui.button("📄 Choose .txt…").clicked() {
                                if let Some(path) = rfd::FileDialog::new().add_filter("text", &["txt", "m3u", "m3u8", "csv"]).pick_file() {
                                    if let Ok(content) = std::fs::read_to_string(&path) {
                                        if !self.in_bulk.is_empty() {
                                            self.in_bulk.push('\n');
                                        }
                                        self.in_bulk.push_str(&content);
                                        self.toast(format!("Loaded {}", path.display()));
                                    }
                                }
                            }
                        });
                        if ui.button("Import list").clicked() {
                            let (a, s) = self.import_bulk(&self.in_bulk.clone(), &self.in_bulk_cat.clone());
                            self.toast(format!("Imported {a}, skipped {s}."));
                            self.in_bulk.clear();
                        }
                    }
                    ImportTab::Folder => {
                        ui.label("Scan a folder containing video files:");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.in_folder).hint_text("/home/you/Videos").desired_width(320.0));
                            if ui.button("📁 Browse…").clicked() {
                                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                    self.in_folder = dir.to_string_lossy().to_string();
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Category (blank = folder name):");
                            ui.add(egui::TextEdit::singleline(&mut self.in_folder_cat).hint_text("Movies").desired_width(140.0));
                            ui.checkbox(&mut self.in_recursive, "Subfolders");
                        });
                        if ui.button("Scan folder").clicked() {
                            match self.import_folder(&self.in_folder.clone(), &self.in_folder_cat.clone(), self.in_recursive) {
                                Ok(n) => self.toast(format!("Added {n} videos ✓")),
                                Err(e) => self.toast(e),
                            }
                        }
                    }
                }
            });
        self.show_import = open;
    }

    fn theme_window(&mut self, ctx: &egui::Context, accent: egui::Color32, _muted: egui::Color32) {
        let mut open = self.show_theme;
        egui::Window::new("🎨 Theme & colors")
            .open(&mut open)
            .resizable(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.theme.style == "glass", "✨ Liquid Glass").clicked() {
                        self.theme.style = "glass".into();
                    }
                    if ui.selectable_label(self.theme.style == "minimal", "◻ Minimal").clicked() {
                        self.theme.style = "minimal".into();
                    }
                });
                ui.small("Glass = translucent panels + soft shadows. Minimal = opaque + sharp.");
                ui.separator();
                // color rows
                let fields: [(&str, &str); 7] = [
                    ("accent", "Accent"),
                    ("background", "Background"),
                    ("surface", "Cards / windows"),
                    ("surface2", "Inputs"),
                    ("text", "Text"),
                    ("muted", "Muted"),
                    ("sidebar", "Sidebar"),
                ];
                for (key, label) in fields {
                    ui.horizontal(|ui| {
                        ui.label(format!("{label}:"));
                        let hex = match key {
                            "accent" => &mut self.theme.accent,
                            "background" => &mut self.theme.background,
                            "surface" => &mut self.theme.surface,
                            "surface2" => &mut self.theme.surface2,
                            "text" => &mut self.theme.text,
                            "muted" => &mut self.theme.muted,
                            "sidebar" => &mut self.theme.sidebar,
                            _ => &mut self.theme.accent,
                        };
                        let mut col = hex_to_color(hex);
                        let mut rgb = [col.r(), col.g(), col.b()];
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            *hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
                        }
                        let _ = col;
                        ui.monospace(hex.clone());
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Corner radius:");
                    ui.add(egui::Slider::new(&mut self.theme.radius, 0..=24));
                });
                ui.horizontal(|ui| {
                    ui.label("Glass glow:");
                    ui.add(egui::Slider::new(&mut self.theme.blur, 0..=40));
                });
                ui.horizontal(|ui| {
                    ui.label("Box size:");
                    ui.add(
                        egui::Slider::new(&mut self.theme.card_scale, 0.6..=1.6)
                            .suffix("\u{d7}")
                            .show_value(true),
                    );
                });
                let _ = accent;
                ui.horizontal(|ui| {
                    if ui.button("Reset defaults").clicked() {
                        self.theme = Theme::default();
                    }
                    if ui.button("💾 Save theme").clicked() {
                        save_theme(&self.theme);
                        self.toast("Theme saved ✓");
                    }
                });
                ui.small("Saved to data/theme.json");
            });
        self.show_theme = open;
        // autosave on close/change? save on each change is cheap:
        save_theme(&self.theme);
    }
}

// ------------------------------------------------------------------ main --
fn main() -> eframe::Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native(
        "Scwoll — media player",
        opts,
        Box::new(|cc| Ok(Box::new(ScwollApp::new(cc)))),
    )
}

#[cfg(test)]
mod thumb_tests {
    use super::*;

    #[test]
    fn mosaic_six_frames() {
        // 12s video: six 2s solid-color segments, so each mosaic cell
        // must land on a different color.
        let vid = std::env::temp_dir().join("scwoll-test-vid.mp4");
        let cols = ["red", "green", "blue", "yellow", "magenta", "cyan"];
        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y").arg("-v").arg("error");
        for c in cols {
            cmd.args(["-f", "lavfi", "-i", &format!("color=c={c}:s=640x360:d=2:r=30")]);
        }
        cmd.args([
            "-filter_complex",
            "[0][1][2][3][4][5]concat=n=6:v=1:a=0",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&vid);
        assert!(cmd.status().expect("ffmpeg missing").success());
        let dur = video_duration(vid.to_str().unwrap()).expect("no duration");
        assert!((dur - 12.0).abs() < 1.0, "dur={dur}");
        let bytes = grab_mosaic_bytes(vid.to_str().unwrap()).expect("no mosaic");
        let img = image::load_from_memory(&bytes).expect("bad jpeg");
        assert_eq!((img.width(), img.height()), (480, 320));
        // decode must preserve the full mosaic (no 16:9 crop eating rows)
        let decoded = decode_thumb_bytes(&bytes).expect("no decode");
        assert_eq!(decoded.size, [480, 320]);
        // the 6 cells must actually differ (frames spread across time)
        let px = |cx: usize, cy: usize| {
            let c = decoded.pixels[cy * 480 + cx];
            (c.r(), c.g(), c.b())
        };
        let mut cells = std::collections::HashSet::new();
        for c in 0..6 {
            cells.insert(px((c % 3) * 160 + 80, (c / 3) * 160 + 80));
        }
        assert!(cells.len() >= 5, "cells not spread across video: {cells:?}");
        std::fs::remove_file(&vid).ok();
    }
}
