use chrono::Utc;
use serde::{Deserialize, Serialize};
use url::Url;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VideoKind {
    Youtube,
    Local,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub url: String,
    pub kind: VideoKind,
    #[serde(default)]
    pub youtube_id: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default = "default_category")]
    pub category: String,
    pub added_at: String,
    pub thumbnail: String,
    #[serde(default)]
    pub favorite: bool,
}

fn default_category() -> String {
    "General".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    #[serde(default = "default_style")]
    pub style: String, // always "glass" (kept for save-file compat)
    #[serde(default = "c_accent")]
    pub accent: String,
    #[serde(default = "c_bg")]
    pub background: String,
    #[serde(default = "c_surface")]
    pub surface: String,
    #[serde(default = "c_surface2")]
    pub surface2: String,
    #[serde(default = "c_text")]
    pub text: String,
    #[serde(default = "c_muted")]
    pub muted: String,
    #[serde(default = "c_sidebar")]
    pub sidebar: String,
    #[serde(default = "default_radius")]
    pub radius: u8,
    /// Liquid-glass mode: translucent panels, soft borders and shadows.
    #[serde(default = "default_glass")]
    pub glass: bool,
    /// Simulated frosting strength (0-40). egui cannot blur the backdrop,
    /// so this drives the softness of the glass tint and glow blobs.
    #[serde(default = "default_blur")]
    pub blur: u8,
    /// Panel opacity in percent (20-100). Ignored when `glass` is off.
    #[serde(default = "default_opacity")]
    pub opacity: u8,
    /// Drop-shadow strength (0-100).
    #[serde(default = "default_shadow")]
    pub shadow: u8,
    /// Accent glow / rim-light strength for cards, panels and focus rings.
    #[serde(default = "default_glow")]
    pub glow: u8,
    /// Animated aurora background strength (0-100).
    #[serde(default = "default_ambient")]
    pub ambient: u8,
    /// Film-grain overlay (0-100).
    #[serde(default = "default_grain")]
    pub grain: u8,
    /// Darkened screen edges (0-100).
    #[serde(default = "default_vignette")]
    pub vignette: u8,
    /// Paint the coloured aurora blobs behind the UI.
    #[serde(default = "default_aurora")]
    pub aurora: bool,
    /// Animate the aurora / background.
    #[serde(default = "default_animate")]
    pub animate: bool,
    /// Highlight cards with an accent rim on hover.
    #[serde(default = "default_hover_glow")]
    pub hover_glow: bool,
    /// Secondary colour used by gradients and the aurora background.
    #[serde(default = "c_accent2")]
    pub accent2: String,
    #[serde(default = "default_card_scale")]
    pub card_scale: f32,
}

fn default_style() -> String {
    "glass".into()
}
fn c_accent() -> String {
    "#007BA7".into() // cerulean blue
}
fn c_bg() -> String {
    "#0b0b10".into()
}
fn c_surface() -> String {
    "#17171f".into()
}
fn c_surface2() -> String {
    "#21212c".into()
}
fn c_text() -> String {
    "#f5f5f7".into()
}
fn c_muted() -> String {
    "#9a9aad".into()
}
fn c_sidebar() -> String {
    "#101018".into()
}
fn default_radius() -> u8 {
    16
}
fn default_glass() -> bool {
    true
}
fn default_blur() -> u8 {
    22
}
fn default_opacity() -> u8 {
    72
}
fn default_shadow() -> u8 {
    55
}
fn default_glow() -> u8 {
    40
}
fn default_ambient() -> u8 {
    55
}
fn default_grain() -> u8 {
    8
}
fn default_vignette() -> u8 {
    30
}
fn default_aurora() -> bool {
    true
}
fn default_animate() -> bool {
    true
}
fn default_hover_glow() -> bool {
    true
}
fn c_accent2() -> String {
    "#7c5cff".into()
}
fn default_card_scale() -> f32 {
    1.0
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            style: default_style(),
            accent: c_accent(),
            background: c_bg(),
            surface: c_surface(),
            surface2: c_surface2(),
            text: c_text(),
            muted: c_muted(),
            sidebar: c_sidebar(),
            radius: default_radius(),
            glass: default_glass(),
            blur: default_blur(),
            opacity: default_opacity(),
            shadow: default_shadow(),
            glow: default_glow(),
            ambient: default_ambient(),
            grain: default_grain(),
            vignette: default_vignette(),
            aurora: default_aurora(),
            animate: default_animate(),
            hover_glow: default_hover_glow(),
            accent2: c_accent2(),
            card_scale: default_card_scale(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Library {
    #[serde(default)]
    pub videos: Vec<Video>,
}

// ---------------------------------------------------------------------------
// Constructors / helpers
// ---------------------------------------------------------------------------

pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "m4v", "avi", "ogv", "flv", "wmv", "ts", "m2ts", "mpg", "mpeg",
];

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

pub fn normalize_category(c: Option<String>) -> String {
    match c {
        Some(s) => {
            let t = s.trim().to_string();
            if t.is_empty() {
                "General".to_string()
            } else {
                t
            }
        }
        None => "General".to_string(),
    }
}

/// Extract a YouTube video id from all common URL shapes.
pub fn parse_youtube_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let parsed = Url::parse(raw).ok()?;
    let host = parsed.host_str().unwrap_or("").to_lowercase();

    // youtu.be/ID
    if host.contains("youtu.be") {
        let id = parsed.path().trim_matches('/').split('/').next()?.to_string();
        if (11..=20).contains(&id.len()) || !id.is_empty() {
            return Some(id);
        }
        return None;
    }

    if host.contains("youtube.com") || host.contains("youtube-nocookie.com") {
        // /watch?v=ID
        if parsed.path() == "/watch" {
            for (k, v) in parsed.query_pairs() {
                if k == "v" && !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        // /embed/ID  /shorts/ID  /live/ID  /v/ID
        let segs: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() >= 2 && ["embed", "shorts", "live", "v"].contains(&segs[0]) {
            return Some(segs[1].to_string());
        }
    }
    None
}

pub fn is_youtube(raw: &str) -> bool {
    parse_youtube_id(raw).is_some()
}

pub fn is_direct_video_url(raw: &str) -> bool {
    let lower = raw.trim().to_lowercase();
    let path_part = lower.split('?').next().unwrap_or("").split('#').next().unwrap_or("");
    VIDEO_EXTS.iter().any(|e| path_part.ends_with(&format!(".{e}")))
}

pub fn guess_title(raw_url: &str) -> String {
    // filename-based guess for local / direct, host-based for pages
    if let Ok(u) = Url::parse(raw_url) {
        if let Some(last) = u.path().rsplit('/').next() {
            let decoded = urlencoding_decode(last);
            let no_ext = strip_video_ext(&decoded);
            let cleaned = no_ext.replace(['_', '+'], " ").replace('.', " ").trim().to_string();
            if !cleaned.is_empty() && cleaned != "/" {
                return titlecase(&cleaned);
            }
        }
        return u.host_str().unwrap_or("Untitled").to_string();
    }
    // local path?
    let p = std::path::Path::new(raw_url);
    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
        return titlecase(&stem.replace(['_', '+'], " ").replace('.', " "));
    }
    "Untitled".to_string()
}

fn urlencoding_decode(s: &str) -> String {
    // tiny %XX decoder, no extra dep
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h: String = chars.by_ref().take(2).collect();
            if let Ok(b) = u8::from_str_radix(&h, 16) {
                out.push(b as char);
            } else {
                out.push('%');
                out.push_str(&h);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn strip_video_ext(name: &str) -> String {
    let lower = name.to_lowercase();
    for e in VIDEO_EXTS {
        if lower.ends_with(&format!(".{e}")) {
            return name[..name.len() - e.len() - 1].to_string();
        }
    }
    name.to_string()
}

fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn make_video_from_url(raw_url: &str, category: String) -> Video {
    let raw_url = raw_url.trim().to_string();
    let category = normalize_category(Some(category));
    if let Some(yid) = parse_youtube_id(&raw_url) {
        return Video {
            id: uuid::Uuid::new_v4().to_string(),
            title: format!("YouTube \u{2022} {yid}"),
            url: raw_url.clone(),
            kind: VideoKind::Youtube,
            youtube_id: Some(yid.clone()),
            file_path: None,
            category,
            added_at: now_iso(),
            thumbnail: format!("https://i.ytimg.com/vi/{yid}/hqdefault.jpg"),
            favorite: false,
        };
    }
    if is_direct_video_url(&raw_url) {
        return Video {
            id: uuid::Uuid::new_v4().to_string(),
            title: guess_title(&raw_url),
            url: raw_url.clone(),
            kind: VideoKind::Direct,
            youtube_id: None,
            file_path: None,
            category,
            added_at: now_iso(),
            thumbnail: String::new(),
            favorite: false,
        };
    }
    // generic page URL — still importable, opens externally / iframe attempt
    Video {
        id: uuid::Uuid::new_v4().to_string(),
        title: guess_title(&raw_url),
        url: raw_url,
        kind: VideoKind::Direct,
        youtube_id: None,
        file_path: None,
        category,
        added_at: now_iso(),
        thumbnail: String::new(),
        favorite: false,
    }
}

pub fn make_video_from_file(path: &std::path::Path, category: String) -> Video {
    let abs = path.to_string_lossy().to_string();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    Video {
        id: uuid::Uuid::new_v4().to_string(),
        title: titlecase(&stem.replace(['_', '-', '+'], " ").replace('.', " ")),
        url: format!("file://{abs}"),
        kind: VideoKind::Local,
        youtube_id: None,
        file_path: Some(abs),
        category: normalize_category(Some(category)),
        added_at: now_iso(),
        thumbnail: String::new(),
        favorite: false,
    }
}

/// Split pasted / file text content into clean URLs (supports # comments).
/// Separators: newlines, spaces, tabs, commas, semicolons, pipes — plus
/// trailing sentence punctuation ("see https://a.com/x.mp4." still parses).
/// Split on `.` itself is deliberately NOT done: dots live inside every URL.
/// Filter keeps anything URL-shaped and drops prose around it.
pub fn split_url_list(text: &str) -> Vec<String> {
    text.split(|c: char| c == ',' || c == ';' || c == '|' || c.is_whitespace())
        .map(|s| {
            s.trim()
                .trim_matches(|c| {
                    c == '"' || c == '\'' || c == '<' || c == '>' || c == '(' || c == ')' || c == '[' || c == ']'
                })
                .trim_end_matches(|c| c == '.' || c == '!' || c == '?')
        })
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .filter(|s| s.contains("://") || s.contains("www.") || s.contains("youtu"))
        .map(|s| s.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Sources — user-saved HTML pages to browse for videos/links.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub added_at: String,
}

/// Normalize a source URL: trim, prepend https:// when the scheme is missing,
/// reject anything that isn't http(s). Returns None for garbage input.
pub fn normalize_source_url(raw: &str) -> Option<String> {
    let t = raw.trim().trim_matches(|c| {
        c == '"' || c == '\'' || c == '<' || c == '>' || c == '(' || c == ')' || c == '[' || c == ']'
    }).trim();
    if t.is_empty() || t.contains(' ') {
        return None;
    }
    let with_scheme = if t.contains("://") {
        t.to_string()
    } else if t.starts_with("www.") || t.contains('.') {
        format!("https://{t}")
    } else {
        return None;
    };
    let u = Url::parse(&with_scheme).ok()?;
    match u.scheme() {
        "http" | "https" => Some(u.to_string()),
        _ => None,
    }
}

pub fn make_source(url: String) -> Source {
    Source {
        id: uuid::Uuid::new_v4().to_string(),
        url,
        title: String::new(),
        added_at: now_iso(),
    }
}

/// Short label for sidebar rows: custom title, else host + trimmed path.
pub fn source_label(s: &Source) -> String {
    if !s.title.trim().is_empty() {
        return s.title.trim().to_string();
    }
    if let Ok(u) = Url::parse(&s.url) {
        let host = u.host_str().unwrap_or("?").to_string();
        let path = u.path();
        if path.len() > 1 {
            let short: String = path.chars().take(22).collect();
            return format!("{host}{}", if path.len() > 22 { format!("{short}…") } else { short });
        }
        return host;
    }
    s.url.clone()
}

pub fn demo_videos() -> Vec<Video> {
    vec![
        Video {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Big Buck Bunny (Demo • Film)".into(),
            url: "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into(),
            kind: VideoKind::Youtube,
            youtube_id: Some("aqz-KE-bpKQ".into()),
            file_path: None,
            category: "Film".into(),
            added_at: now_iso(),
            thumbnail: "https://i.ytimg.com/vi/aqz-KE-bpKQ/hqdefault.jpg".into(),
            favorite: false,
        },
        Video {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Lo-Fi Beats To Relax To (Demo • Music)".into(),
            url: "https://www.youtube.com/watch?v=jfKfPfyJRdk".into(),
            kind: VideoKind::Youtube,
            youtube_id: Some("jfKfPfyJRdk".into()),
            file_path: None,
            category: "Music".into(),
            added_at: now_iso(),
            thumbnail: "https://i.ytimg.com/vi/jfKfPfyJRdk/hqdefault.jpg".into(),
            favorite: false,
        },
        Video {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Sample MP4 Direct Stream (Demo • Tech)".into(),
            url: "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4".into(),
            kind: VideoKind::Direct,
            youtube_id: None,
            file_path: None,
            category: "Tech".into(),
            added_at: now_iso(),
            thumbnail: String::new(),
            favorite: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_ids() {
        assert_eq!(parse_youtube_id("https://www.youtube.com/watch?v=aqz-KE-bpKQ"), Some("aqz-KE-bpKQ".into()));
        assert_eq!(parse_youtube_id("https://youtu.be/dQw4w9WgXcQ"), Some("dQw4w9WgXcQ".into()));
        assert_eq!(parse_youtube_id("https://www.youtube.com/shorts/abc123XYZ12"), Some("abc123XYZ12".into()));
        assert_eq!(parse_youtube_id("https://www.youtube.com/embed/abc123XYZ12"), Some("abc123XYZ12".into()));
        assert_eq!(parse_youtube_id("https://example.com/clip.mp4"), None);
    }

    #[test]
    fn url_list_split() {
        let urls = split_url_list("https://a.com/1.mp4\n# comment\nhttps://youtu.be/dQw4w9WgXcQ");
        assert_eq!(urls.len(), 2);
        // commas, spaces, pipes, trailing periods
        let urls = split_url_list("https://a.com/1.mp4, https://b.com/2.mp4 https://c.com/3.mp4|https://d.com/4.mp4.");
        assert_eq!(urls.len(), 4);
        // sentence-wrapped url
        let urls = split_url_list("watch this: https://youtu.be/dQw4w9WgXcQ. so good!");
        assert_eq!(urls, vec!["https://youtu.be/dQw4w9WgXcQ".to_string()]);
    }

    #[test]
    fn video_kinds() {
        assert_eq!(make_video_from_url("https://youtu.be/dQw4w9WgXcQ", "Music".into()).kind, VideoKind::Youtube);
        assert_eq!(make_video_from_url("https://example.com/clip.mp4", "M".into()).kind, VideoKind::Direct);
    }

    #[test]
    fn source_urls() {
        assert_eq!(normalize_source_url("https://example.com/blog"), Some("https://example.com/blog".into()));
        assert_eq!(normalize_source_url("example.com/blog"), Some("https://example.com/blog".into()));
        assert_eq!(normalize_source_url("www.example.com/"), Some("https://www.example.com/".into()));
        assert!(normalize_source_url("not a url").is_none());
        assert!(normalize_source_url("ftp://example.com/x").is_none());
        let s = make_source("https://example.com/a".into());
        assert_eq!(source_label(&s), "example.com/a");
    }
}
