//! Scwoll — native YouTube-style media player in Rust. No browser, no server.
//! Library (URLs / .txt lists / folders) + categories + liquid glass theme.
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
    let mut theme: Theme = std::fs::read_to_string(theme_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Theme>(&s).ok())
        .unwrap_or_default();
    // Keep the save-file style tag pinned for compatibility and migrate old
    // defaults (old default red accent -> cerulean).
    theme.style = "glass".into();
    if theme.accent.eq_ignore_ascii_case("#ff0033") {
        theme.accent = Theme::default().accent;
    }
    if theme.accent2.trim().is_empty() {
        theme.accent2 = Theme::default().accent2;
    }
    save_theme(&theme);
    theme
}
fn save_theme(theme: &Theme) {
    if let Ok(s) = serde_json::to_string_pretty(theme) {
        std::fs::write(theme_path(), s).ok();
    }
}

// --------------------------------------------------------------- sources --
// Bookmarked HTML pages the user can browse inside the app for videos and
// links. Stored in data/sources.json (separate from library.json so old
// libraries keep loading). Pages are fetched with ureq on worker threads;
// links are extracted with a tiny href scanner (no extra deps).
fn sources_path() -> PathBuf {
    data_dir().join("sources.json")
}
fn load_sources() -> Vec<Source> {
    std::fs::create_dir_all(data_dir()).ok();
    if let Ok(s) = std::fs::read_to_string(sources_path()) {
        if let Ok(v) = serde_json::from_str::<Vec<Source>>(&s) {
            return v;
        }
    }
    vec![]
}
fn save_sources(sources: &[Source]) {
    if let Ok(s) = serde_json::to_string_pretty(sources) {
        std::fs::write(sources_path(), s).ok();
    }
}

#[derive(Debug, Clone)]
struct PageLink {
    url: String,
    label: String,
    is_video: bool,
}

// Lightweight rendered-document model: the fetched HTML is parsed into
// blocks once (worker thread) and then drawn with stock egui widgets —
// headings, paragraphs with bold/italic/code/links, lists, quotes,
// preformatted text, rules and images. No browser engine needed.
#[derive(Debug, Clone, Default)]
struct DocSpan {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    link: Option<String>,
}

#[derive(Debug, Clone)]
enum DocBlock {
    Heading { level: u8, text: String },
    Para(Vec<DocSpan>),
    Bullet(Vec<DocSpan>),
    Numbered(usize, Vec<DocSpan>),
    Image { src: String, alt: String },
    Hr,
    Pre(String),
    Quote(Vec<DocSpan>),
}

#[derive(Debug, Clone, Default)]
struct HtmlDoc {
    blocks: Vec<DocBlock>,
}

#[derive(Debug, Clone)]
struct SourcePage {
    title: String,
    links: Vec<PageLink>,
    doc: HtmlDoc,
    fetched_at: String,
    error: Option<String>,
    /// true when the shown doc came from a post-JS DOM snapshot.
    js_rendered: bool,
}

struct SourceFetchMsg {
    id: String,
    result: Result<(String, Vec<PageLink>, HtmlDoc, bool), String>,
}

struct SourceImgMsg {
    url: String,
    image: Option<egui::ColorImage>,
}

/// Download an HTML page (max ~2MB, 15s timeout). Errors are short strings
/// suitable for display in the UI.
fn fetch_html(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent("scwoll/0.1")
        .build();
    let resp = agent.get(url).call().map_err(|e| format!("request failed: {e}"))?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!("HTTP {}", resp.status()));
    }
    let ctype = resp.header("Content-Type").unwrap_or("").to_lowercase();
    if !ctype.is_empty() && !ctype.contains("html") && !ctype.contains("text") {
        return Err(format!("not an HTML page ({ctype})"));
    }
    let mut buf = String::new();
    resp.into_reader()
        .take(2_000_000)
        .read_to_string(&mut buf)
        .map_err(|e| format!("read failed: {e}"))?;
    if buf.trim().is_empty() {
        return Err("empty page".into());
    }
    Ok(buf)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<title") {
        if let Some(gt) = html[start..].find('>') {
            let rest = &html[start + gt + 1..];
            let rest_lower = rest.to_lowercase();
            if let Some(end) = rest_lower.find("</title>") {
                return strip_tags(rest[..end].trim()).chars().take(120).collect();
            }
        }
    }
    String::new()
}

/// Scan `<a ... href="...">label</a>` tags, resolve against the page URL,
/// dedupe (keep first), cap at 500. Labels are tag-stripped + truncated.
fn extract_links(base_url: &str, html: &str) -> Vec<PageLink> {
    let base = url::Url::parse(base_url).ok();
    let mut out: Vec<PageLink> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() && out.len() < 500 {
        // find next "<a" (case-insensitive)
        let rest = &html[i..];
        let rel = rest.to_lowercase().find("<a");
        let tag_start = match rel {
            Some(r) => i + r,
            None => break,
        };
        // end of opening tag
        let gt = match html[tag_start..].find('>') {
            Some(g) => tag_start + g,
            None => break,
        };
        let open_tag = &html[tag_start..=gt];
        let open_lower = open_tag.to_lowercase();
        // href value
        let mut href: Option<String> = None;
        if let Some(hpos) = open_lower.find("href") {
            let after = &open_tag[hpos + 4..];
            let after = after.trim_start_matches(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '=');
            let mut chars = after.chars();
            if let Some(q) = chars.next() {
                if q == '"' || q == '\'' {
                    if let Some(end) = after[1..].find(q) {
                        href = Some(after[1..1 + end].to_string());
                    }
                } else {
                    let end = after.find(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '>').unwrap_or(after.len());
                    href = Some(after[..end].to_string());
                }
            }
        }
        // label: text until </a>
        let after_gt = &html[gt + 1..];
        let close_rel = after_gt.to_lowercase().find("</a>");
        let (raw_label, next_i) = match close_rel {
            Some(r) => (after_gt[..r.min(400)].to_string(), gt + 1 + r + 4),
            None => (String::new(), gt + 1),
        };
        i = next_i.max(tag_start + 2);
        let mut href = match href {
            Some(h) => h.trim().to_string(),
            None => continue,
        };
        if href.is_empty() || href.starts_with('#') || href.to_lowercase().starts_with("javascript:") || href.to_lowercase().starts_with("mailto:") {
            continue;
        }
        // resolve relative -> absolute http(s) only
        let abs = if let Some(b) = &base {
            match b.join(&href) {
                Ok(u) => u.to_string(),
                Err(_) => continue,
            }
        } else {
            href.clone()
        };
        let parsed = match url::Url::parse(&abs) {
            Ok(u) => u,
            Err(_) => continue,
        };
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            continue;
        }
        let abs = parsed.to_string();
        if !seen.insert(abs.clone()) {
            continue;
        }
        let mut label = strip_tags(&raw_label);
        if label.is_empty() {
            label = abs.clone();
        }
        if label.chars().count() > 90 {
            label = label.chars().take(89).collect::<String>() + "…";
        }
        let is_video = parse_youtube_id(&abs).is_some() || is_direct_video_url(&abs);
        out.push(PageLink { url: abs, label, is_video });
    }
    // videos first, keep discovery order within each group
    out.sort_by_key(|l| !l.is_video);
    out
}

fn fetch_source_page(url: &str, force_js: bool) -> Result<(String, Vec<PageLink>, HtmlDoc, bool), String> {
    let html = fetch_html(url)?;
    let mut title = extract_title(&html);
    let mut links = extract_links(url, &html);
    let mut doc = parse_html_doc(url, &html);
    let mut js = false;
    // Script-rendered shell with (almost) no readable content? Re-fetch the
    // post-JS DOM so the page actually shows something in-app.
    if force_js || looks_like_spa(&html, &doc) {
        match snapshot_dom(url) {
            Ok(dom) => {
                let t2 = extract_title(&dom);
                if !t2.is_empty() {
                    title = t2;
                }
                links = extract_links(url, &dom);
                doc = parse_html_doc(url, &dom);
                js = true;
            }
            Err(e) => {
                if force_js {
                    return Err(format!("JS render failed: {e}"));
                }
                // auto path: keep the raw doc; the UI offers a manual retry.
            }
        }
    }
    Ok((title, links, doc, js))
}

/// Locate a chromium binary for JS rendering: $SCWOLL_CHROMIUM first,
/// then common names on PATH (provided by shell.nix).
fn find_chromium() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SCWOLL_CHROMIUM") {
        let p = PathBuf::from(&p);
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in ["chromium", "chromium-browser", "google-chrome", "google-chrome-stable", "headless_shell"] {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Run page JS in headless chromium and dump the resulting DOM.
/// The virtual-time budget lets SPA boot + data fetching settle without
/// wall-clock waiting. Runs on a worker thread; 30s hard timeout.
fn snapshot_dom(url: &str) -> Result<String, String> {
    let bin = find_chromium().ok_or("no chromium on PATH (launch via nix-shell for JS pages)")?;
    let profile = std::env::temp_dir().join(format!("scwoll-chrome-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&profile).ok();
    // `--headless=new` on recent chromium, plain `--headless` on old builds.
    let mut last_err = String::from("chromium produced no DOM");
    for headless in ["--headless=new", "--headless"] {
        let mut child = Command::new(&bin)
            .args([
                headless,
                "--no-sandbox",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                &format!("--user-data-dir={}", profile.display()),
                "--virtual-time-budget=8000",
                "--dump-dom",
                url,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't launch chromium: {e}"))?;
        match child.wait_timeout(Duration::from_secs(30)).map_err(|e| format!("wait failed: {e}"))? {
            Some(status) => {
                let out = child.wait_with_output().map_err(|e| format!("read failed: {e}"))?;
                let dom = String::from_utf8_lossy(&out.stdout).into_owned();
                if status.success() && dom.trim_start().starts_with('<') && dom.len() > 100 {
                    std::fs::remove_dir_all(&profile).ok();
                    return Ok(dom);
                }
                last_err = format!("chromium exited with {status} / dumped nothing");
            }
            None => {
                child.kill().ok();
                child.wait().ok();
                std::fs::remove_dir_all(&profile).ok();
                return Err("chromium timed out after 30s".into());
            }
        }
    }
    std::fs::remove_dir_all(&profile).ok();
    Err(last_err)
}

/// Readable text volume of a parsed doc, in chars.
fn doc_text_len(doc: &HtmlDoc) -> usize {
    doc.blocks
        .iter()
        .map(|b| match b {
            DocBlock::Heading { text, .. } => text.len(),
            DocBlock::Para(s) | DocBlock::Bullet(s) | DocBlock::Numbered(_, s) | DocBlock::Quote(s) => {
                s.iter().map(|sp| sp.text.len()).sum()
            }
            DocBlock::Pre(t) => t.len(),
            _ => 0,
        })
        .sum()
}

/// Big HTML shell + app-root markers + bundled scripts but (almost) no
/// readable text: almost certainly a script-rendered page.
fn looks_like_spa(html: &str, doc: &HtmlDoc) -> bool {
    if doc_text_len(doc) >= 400 || html.len() < 1500 {
        return false;
    }
    let low = html.to_lowercase();
    [
        "id=\"root\"",
        "id=\"app\"",
        "__next_data__",
        "data-reactroot",
        "ng-app",
        "ng-version",
        "id=\"__nuxt\"",
        "vite",
        "bundle.js",
        "chunk.js",
    ]
    .iter()
    .any(|m| low.contains(m))
}

/// Shown doc has basically nothing in it (used for the JS hint banner).
fn page_looks_empty(page: &SourcePage) -> bool {
    doc_text_len(&page.doc) < 200 && page.links.len() < 3
}

// -------------------------------------------------------- html renderer --
// Tiny HTML->document parser: block tags (h1-h6/p/div/li/img/pre/…),
// inline bold/italic/code/links, skips script/style/comments, decodes
// entities, resolves relative URLs. Caps output so junk pages can't blow up.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let b = s.as_bytes();
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(semi) = s[i..].find(';') {
                if semi <= 10 {
                    let ent = &s[i + 1..i + semi];
                    let rep: Option<String> = match ent {
                        "amp" => Some("&".into()),
                        "lt" => Some("<".into()),
                        "gt" => Some(">".into()),
                        "quot" => Some("\"".into()),
                        "apos" | "#39" => Some("'".into()),
                        "nbsp" => Some(" ".into()),
                        "copy" => Some("©".into()),
                        "mdash" => Some("—".into()),
                        "ndash" => Some("–".into()),
                        _ if ent.starts_with('#') => {
                            let num = &ent[1..];
                            let cp = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                                u32::from_str_radix(hex, 16).ok()
                            } else {
                                num.parse::<u32>().ok()
                            };
                            cp.and_then(char::from_u32).map(|c| c.to_string())
                        }
                        _ => None,
                    };
                    if let Some(r) = rep {
                        out.push_str(&r);
                        i += semi + 1;
                        continue;
                    }
                }
            }
        }
        // push one char (keep it simple: byte-wise for ascii, char-wise else)
        if let Some(c) = s[i..].chars().next() {
            out.push(c);
            i += c.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn tag_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let mut search = 0;
    while let Some(pos) = lower[search..].find(name) {
        let p = search + pos;
        let before = p.checked_sub(1).map(|i| lower.as_bytes()[i] as char);
        if before.is_some_and(|c| !c.is_whitespace()) {
            search = p + name.len();
            continue;
        }
        let after = tag[p + name.len()..].trim_start_matches(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r');
        let val = after.strip_prefix('=')?;
        let val = val.trim_start_matches(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r');
        if let Some(q) = val.chars().next() {
            if q == '"' || q == '\'' {
                return val[1..].find(q).map(|e| val[1..1 + e].to_string());
            }
            let end = val.find(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '>').unwrap_or(val.len());
            return Some(val[..end].to_string());
        }
        return None;
    }
    None
}

fn resolve_http(base: &url::Url, raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let low = t.to_lowercase();
    if low.starts_with("javascript:") || low.starts_with("mailto:") || low.starts_with("data:") {
        return None;
    }
    let abs = base.join(t).ok()?;
    if abs.scheme() != "http" && abs.scheme() != "https" {
        return None;
    }
    Some(abs.to_string())
}

fn parse_html_doc(base_url: &str, html: &str) -> HtmlDoc {
    let base = url::Url::parse(base_url);
    let mut doc = HtmlDoc::default();
    let mut spans: Vec<DocSpan> = vec![];
    let mut bold: i32 = 0;
    let mut italic: i32 = 0;
    let mut code: i32 = 0;
    let mut link_stack: Vec<Option<String>> = vec![];
    let mut skip_depth: i32 = 0; // script/style/title content
    let mut pre_depth: i32 = 0;
    let mut quote_depth: i32 = 0;
    let mut heading: Option<(u8, String)> = None;
    let mut pre_buf = String::new();
    let mut lists: Vec<Option<usize>> = vec![]; // None = bullet, Some(n) = next number
    let mut img_count = 0;

    let push_text = |dst: &mut Vec<DocSpan>, text: String, bold: i32, italic: i32, code: i32, link: Option<String>| {
        if text.is_empty() {
            return;
        }
        let sp = DocSpan { text, bold: bold > 0, italic: italic > 0, code: code > 0, link };
        if let Some(last) = dst.last_mut() {
            if last.bold == sp.bold && last.italic == sp.italic && last.code == sp.code && last.link == sp.link {
                last.text.push_str(&sp.text);
                return;
            }
        }
        if dst.len() < 80 {
            dst.push(sp);
        }
    };

    macro_rules! flush_para {
        () => {{
            if heading.is_some() || pre_depth > 0 {
                // handled by their own closers
            } else if spans.iter().all(|s| s.text.trim().is_empty()) {
                spans.clear();
            } else if !spans.is_empty() && doc.blocks.len() < 600 {
                let cur = std::mem::take(&mut spans);
                if quote_depth > 0 {
                    doc.blocks.push(DocBlock::Quote(cur));
                } else if let Some(top) = lists.last_mut() {
                    match top {
                        None => doc.blocks.push(DocBlock::Bullet(cur)),
                        Some(n) => {
                            doc.blocks.push(DocBlock::Numbered(*n, cur));
                            *n += 1;
                        }
                    }
                } else {
                    doc.blocks.push(DocBlock::Para(cur));
                }
            } else {
                spans.clear();
            }
        }};
    }

    let mut i = 0;
    let n = html.len();
    while i < n && doc.blocks.len() < 600 {
        if html[i..].starts_with("<!--") {
            if let Some(end) = html[i..].find("-->") {
                i += end + 3;
            } else {
                break;
            }
            continue;
        }
        if html.as_bytes()[i] == b'<' {
            if let Some(gt) = html[i..].find('>') {
                let raw = &html[i + 1..i + gt];
                let is_close = raw.trim_start().starts_with('/');
                let name = raw.trim_start().trim_start_matches('/').split(|c| c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '/').next().unwrap_or("").to_lowercase();
                i += gt + 1;
                // skip regions
                if !is_close && (name == "script" || name == "style" || name == "title" || name == "noscript") {
                    skip_depth += 1;
                    continue;
                }
                if is_close && (name == "script" || name == "style" || name == "title" || name == "noscript") {
                    skip_depth = skip_depth.saturating_sub(1);
                    continue;
                }
                if skip_depth > 0 {
                    continue;
                }
                match name.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        if !is_close {
                            flush_para!();
                            heading = Some((name[1..2].parse().unwrap_or(2), String::new()));
                        } else if let Some((lv, txt)) = heading.take() {
                            let t = txt.split_whitespace().collect::<Vec<_>>().join(" ");
                            if !t.is_empty() {
                                doc.blocks.push(DocBlock::Heading { level: lv, text: t.chars().take(200).collect() });
                            }
                        }
                    }
                    "p" | "div" | "article" | "section" | "header" | "footer" | "main" | "table" | "tr" | "figure" | "figcaption" => {
                        flush_para!();
                    }
                    "br" => flush_para!(),
                    "hr" => {
                        flush_para!();
                        doc.blocks.push(DocBlock::Hr);
                    }
                    "ul" | "ol" => {
                        flush_para!();
                        if !is_close {
                            lists.push(if name == "ol" { Some(1) } else { None });
                            if lists.len() > 4 {
                                lists.remove(0);
                            }
                        } else {
                            lists.pop();
                        }
                    }
                    "li" => flush_para!(),
                    "blockquote" => {
                        flush_para!();
                        if !is_close {
                            quote_depth += 1;
                        } else {
                            quote_depth = quote_depth.saturating_sub(1);
                        }
                    }
                    "pre" => {
                        if !is_close {
                            flush_para!();
                            pre_depth += 1;
                            pre_buf.clear();
                        } else {
                            pre_depth = pre_depth.saturating_sub(1);
                            if !pre_buf.trim().is_empty() && doc.blocks.len() < 600 {
                                let t: String = pre_buf.chars().take(2000).collect();
                                doc.blocks.push(DocBlock::Pre(t));
                            }
                            pre_buf.clear();
                        }
                    }
                    "a" => {
                        if !is_close {
                            let href = tag_attr(raw, "href").unwrap_or_default();
                            let abs = base.as_ref().ok().and_then(|b| resolve_http(b, &href));
                            link_stack.push(abs);
                        } else {
                            link_stack.pop();
                        }
                    }
                    "b" | "strong" => {
                        if !is_close {
                            bold += 1;
                        } else {
                            bold = bold.saturating_sub(1);
                        }
                    }
                    "i" | "em" | "cite" => {
                        if !is_close {
                            italic += 1;
                        } else {
                            italic = italic.saturating_sub(1);
                        }
                    }
                    "code" | "tt" | "kbd" => {
                        if !is_close {
                            code += 1;
                        } else {
                            code = code.saturating_sub(1);
                        }
                    }
                    "img" => {
                        if img_count < 60 {
                            if let Some(src_raw) = tag_attr(raw, "src") {
                                if let Some(abs) = base.as_ref().ok().and_then(|b| resolve_http(b, &src_raw)) {
                                    flush_para!();
                                    let alt = tag_attr(raw, "alt").unwrap_or_default().chars().take(120).collect();
                                    doc.blocks.push(DocBlock::Image { src: abs, alt });
                                    img_count += 1;
                                }
                            }
                        }
                    }
                    _ => {}
                }
                continue;
            } else {
                break;
            }
        }
        // text until next '<'
        let next = html[i..].find('<').map(|p| i + p).unwrap_or(n);
        let chunk = &html[i..next];
        i = next;
        if skip_depth > 0 {
            continue;
        }
        let decoded = decode_entities(chunk);
        if pre_depth > 0 {
            if pre_buf.len() < 3000 {
                pre_buf.push_str(&decoded);
            }
            continue;
        }
        if let Some((_, ref mut htxt)) = heading {
            htxt.push_str(&decoded);
            if htxt.len() > 400 {
                htxt.truncate(400);
            }
            continue;
        }
        // collapse whitespace (keep single spaces)
        let collapsed = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            continue;
        }
        let link = link_stack.iter().rev().flatten().next().cloned();
        let piece = if spans.is_empty() { collapsed } else { format!(" {collapsed}") };
        push_text(&mut spans, piece, bold, italic, code, link);
        // auto-flush giant paragraphs
        if spans.iter().map(|s| s.text.len()).sum::<usize>() > 1200 {
            flush_para!();
        }
    }
    flush_para!();
    if let Some((lv, txt)) = heading.take() {
        let t = txt.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            doc.blocks.push(DocBlock::Heading { level: lv, text: t.chars().take(200).collect() });
        }
    }
    doc
}

/// Download + decode a page image into an egui image (worker thread).
fn fetch_page_image(url: &str) -> Option<egui::ColorImage> {
    let bytes = download_bytes(url)?;
    if bytes.is_empty() || bytes.len() > 4_000_000 {
        return None;
    }
    let img = image::load_from_memory(&bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let rgba = if (w as u64) * (h as u64) > 900 * 700 {
        img.resize(900, 700, image::imageops::FilterType::Triangle).to_rgba8()
    } else {
        rgba
    };
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw()))
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
fn with_alpha(c: egui::Color32, a: u8, glass: bool) -> egui::Color32 {
    if glass {
        egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    } else {
        c
    }
}
fn alpha_color(c: egui::Color32, a: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}
fn mix_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

fn panel_alpha(theme: &Theme) -> u8 {
    if theme.glass {
        (theme.opacity.clamp(20, 100) as f32 * 2.35) as u8
    } else {
        255
    }
}
fn card_alpha(theme: &Theme) -> u8 {
    if theme.glass {
        (theme.opacity.clamp(20, 100) as f32 * 2.05 + 18.0) as u8
    } else {
        255
    }
}
fn input_alpha(theme: &Theme) -> u8 {
    if theme.glass {
        ((theme.opacity.clamp(20, 100) as f32 * 1.65) + 65.0).min(248.0) as u8
    } else {
        255
    }
}

fn theme_shadow(theme: &Theme) -> egui::Shadow {
    if theme.shadow == 0 {
        return egui::Shadow::NONE;
    }
    let s = theme.shadow as f32 / 100.0;
    let blur = 8.0 + theme.blur as f32 * 0.95 + s * 26.0;
    let alpha = (s * 170.0 + if theme.glass { 12.0 } else { 0.0 }).min(210.0) as u8;
    egui::Shadow {
        offset: egui::vec2(0.0, 3.0 + s * 11.0),
        blur,
        spread: 0.0,
        color: egui::Color32::from_black_alpha(alpha),
    }
}

/// Curated looks. Presets only touch appearance/effect fields so users keep
/// their accent and category data. The Colors tab is still fully editable.
pub fn apply_theme_preset(theme: &mut Theme, preset: &str) {
    match preset {
        "Liquid Glass" => {
            theme.glass = true;
            theme.opacity = 68;
            theme.blur = 26;
            theme.shadow = 62;
            theme.glow = 48;
            theme.ambient = 60;
            theme.grain = 8;
            theme.vignette = 32;
            theme.aurora = true;
            theme.animate = true;
            theme.hover_glow = true;
            theme.radius = 22;
        }
        "Frosted" => {
            theme.glass = true;
            theme.opacity = 86;
            theme.blur = 40;
            theme.shadow = 38;
            theme.glow = 16;
            theme.ambient = 34;
            theme.grain = 20;
            theme.vignette = 18;
            theme.aurora = true;
            theme.animate = false;
            theme.hover_glow = false;
            theme.radius = 16;
        }
        "Neon" => {
            theme.glass = true;
            theme.opacity = 52;
            theme.blur = 16;
            theme.shadow = 82;
            theme.glow = 95;
            theme.ambient = 90;
            theme.grain = 0;
            theme.vignette = 58;
            theme.aurora = true;
            theme.animate = true;
            theme.hover_glow = true;
            theme.radius = 24;
            theme.accent2 = "#ff2d95".into();
        }
        "OLED" => {
            theme.glass = false;
            theme.opacity = 100;
            theme.blur = 0;
            theme.shadow = 0;
            theme.glow = 0;
            theme.ambient = 0;
            theme.grain = 0;
            theme.vignette = 0;
            theme.aurora = false;
            theme.animate = false;
            theme.hover_glow = false;
            theme.radius = 10;
            theme.background = "#000000".into();
            theme.surface = "#08080c".into();
            theme.surface2 = "#14141a".into();
            theme.sidebar = "#040406".into();
        }
        "Minimal" => {
            theme.glass = false;
            theme.opacity = 100;
            theme.blur = 0;
            theme.shadow = 0;
            theme.glow = 8;
            theme.ambient = 0;
            theme.grain = 0;
            theme.vignette = 0;
            theme.aurora = false;
            theme.animate = false;
            theme.hover_glow = true;
            theme.radius = 8;
        }
        _ => {}
    }
}

fn hash32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// A soft radial blob built from a triangle fan. Two rings give a smoother
/// falloff than a single centre-to-edge gradient, with no extra dependencies.
fn paint_radial_glow(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    inner: egui::Color32,
    outer: egui::Color32,
) {
    if radius <= 1.0 {
        return;
    }
    const SEG: usize = 64;
    let mid = alpha_color(inner, (inner.a() as f32 * 0.42) as u8);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, inner);
    for i in 0..SEG {
        let a = i as f32 / SEG as f32 * std::f32::consts::TAU;
        let p = center + egui::vec2(a.cos(), a.sin()) * (radius * 0.55);
        mesh.colored_vertex(p, mid);
    }
    for i in 0..SEG {
        let a = i as f32 / SEG as f32 * std::f32::consts::TAU;
        let p = center + egui::vec2(a.cos(), a.sin()) * radius;
        mesh.colored_vertex(p, outer);
    }
    for i in 0..SEG {
        let a = 1 + i as u32;
        let b = 1 + ((i + 1) % SEG) as u32;
        let c = 1 + SEG as u32 + i as u32;
        let d = 1 + SEG as u32 + ((i + 1) % SEG) as u32;
        mesh.add_triangle(0, a, b);
        mesh.add_triangle(a, c, b);
        mesh.add_triangle(b, c, d);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Full-window backdrop: a subdued accent gradient, soft animated aurora
/// blobs, a vignette and optional film grain. Painted on egui's background
/// layer so every translucent glass panel picks up the colour behind it.
fn paint_backdrop(ctx: &egui::Context, theme: &Theme) {
    let rect = ctx.screen_rect();
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::background());
    let bg = hex_to_color(&theme.background);
    let accent = hex_to_color(&theme.accent);
    let accent2 = hex_to_color(&theme.accent2);
    let ambient = (theme.ambient as f32 / 100.0).clamp(0.0, 1.0);

    // Base gradient: a touch of both accents at opposite corners keeps the
    // background from looking like a flat slab even with aurora disabled.
    let tint = if theme.aurora { 0.12 } else { 0.05 };
    let tl = mix_color(bg, accent, tint * ambient);
    let tr = mix_color(bg, accent2, tint * ambient * 0.85);
    let bl = mix_color(bg, accent2, tint * ambient * 0.65);
    let br = mix_color(bg, accent, tint * ambient * 0.75);
    let mut base = egui::Mesh::default();
    base.colored_vertex(rect.left_top(), tl);
    base.colored_vertex(rect.right_top(), tr);
    base.colored_vertex(rect.left_bottom(), bl);
    base.colored_vertex(rect.right_bottom(), br);
    base.add_triangle(0, 1, 2);
    base.add_triangle(2, 1, 3);
    painter.add(egui::Shape::mesh(base));

    if theme.aurora && ambient > 0.01 {
        let t = if theme.animate {
            ctx.input(|i| i.time) as f32
        } else {
            0.0
        };
        let maxr = rect.width().max(rect.height());
        let frost = (0.70 + theme.blur as f32 / 50.0).clamp(0.65, 1.9);
        let strength = ((12.0 + ambient * 38.0) * (0.72 + theme.glow as f32 / 140.0))
            .clamp(8.0, 86.0) as u8;
        let blobs = [
            (0.16, 0.24, accent, 0.50, 0.0, 0.10, 0.055),
            (0.82, 0.18, accent2, 0.46, 1.9, 0.085, 0.075),
            (0.68, 0.78, accent, 0.42, 3.4, 0.11, 0.065),
            (0.22, 0.86, accent2, 0.44, 5.1, 0.075, 0.09),
            (0.50, 0.47, mix_color(accent, accent2, 0.5), 0.34, 2.4, 0.06, 0.05),
        ];
        for (fx, fy, color, scale, phase, sx, sy) in blobs {
            let ox = (t * sx + phase).sin() * rect.width() * 0.07;
            let oy = (t * sy + phase * 1.7).cos() * rect.height() * 0.07;
            let center = egui::pos2(
                rect.left() + rect.width() * fx + ox,
                rect.top() + rect.height() * fy + oy,
            );
            let radius = maxr * scale * frost;
            paint_radial_glow(&painter, center, radius, alpha_color(color, strength), alpha_color(color, 0));
        }
    }

    if theme.vignette > 0 {
        let v = theme.vignette as f32 / 100.0;
        let radius = rect.width().max(rect.height()) * 0.78;
        paint_radial_glow(
            &painter,
            rect.center(),
            radius,
            alpha_color(egui::Color32::BLACK, 0),
            egui::Color32::from_black_alpha((v * 125.0) as u8),
        );
    }

    if theme.grain > 0 {
        let amount = theme.grain as f32 / 100.0;
        let count = ((rect.width() * rect.height() / 500.0) * amount) as usize;
        let count = count.clamp(0, 5000);
        let a = (5.0 + amount * 30.0) as u8;
        let mut grain = egui::Mesh::default();
        for i in 0..count {
            let h = hash32(i as u32 ^ 0x9e37_79b9);
            let x = rect.left() + (h & 0xffff) as f32 / 65535.0 * rect.width();
            let y = rect.top() + ((h >> 16) & 0xffff) as f32 / 65535.0 * rect.height();
            grain.add_colored_rect(
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(1.0, 1.0)),
                alpha_color(egui::Color32::WHITE, a),
            );
        }
        painter.add(egui::Shape::mesh(grain));
    }
}

/// Thin one-pixel accent gradient along a panel edge. Kept in a middle
/// layer so it sits above the panel fill but below floating windows.
fn paint_panel_edge(
    ctx: &egui::Context,
    rect: egui::Rect,
    horizontal: bool,
    c1: egui::Color32,
    c2: egui::Color32,
) {
    if rect.width() < 2.0 || rect.height() < 2.0 {
        return;
    }
    let id = egui::Id::new(if horizontal { "panel_edge_h" } else { "panel_edge_v" });
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, id));
    let edge = if horizontal {
        egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - 1.0),
            rect.right_bottom(),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(rect.right() - 1.0, rect.top()),
            rect.right_bottom(),
        )
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(edge.left_top(), c1);
    mesh.colored_vertex(edge.right_top(), if horizontal { c2 } else { c1 });
    mesh.colored_vertex(edge.left_bottom(), if horizontal { c1 } else { c2 });
    mesh.colored_vertex(edge.right_bottom(), c2);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    painter.add(egui::Shape::mesh(mesh));
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

/// Shorten a title to a single line at the given content width so cards
/// stay compact and shelf rows line up evenly.
fn shorten_title(title: &str, content_w: f32) -> String {
    let max = ((content_w / 7.0).floor() as usize).max(16);
    let chars: Vec<char> = title.chars().collect();
    if chars.len() <= max {
        title.to_string()
    } else {
        chars[..max].iter().collect::<String>() + "\u{2026}"
    }
}


fn apply_visuals(ctx: &egui::Context, theme: &Theme) {
    let glass = theme.glass;
    let accent = hex_to_color(&theme.accent);
    let surface = hex_to_color(&theme.surface);
    let surface2 = hex_to_color(&theme.surface2);
    let text = hex_to_color(&theme.text);
    let muted = hex_to_color(&theme.muted);

    let panel_a = panel_alpha(theme);
    let card_a = card_alpha(theme);
    let input_a = input_alpha(theme);
    let r = egui::Rounding::same(theme.radius.min(26) as f32);
    let r_small = egui::Rounding::same((theme.radius.min(16) as f32).max(4.0));
    let rim = mix_color(muted, accent, 0.38);
    let rim_a = (24.0 + theme.glow as f32 * 1.45).min(155.0) as u8;
    let border = alpha_color(rim, if glass { rim_a } else { 72 });
    let shadow = theme_shadow(theme);

    let mut vis = egui::Visuals::dark();
    vis.dark_mode = true;
    vis.override_text_color = Some(text);
    vis.window_fill = with_alpha(surface, panel_a, glass);
    vis.panel_fill = egui::Color32::TRANSPARENT; // backdrop shows through
    vis.extreme_bg_color = with_alpha(surface2, input_a, glass);
    vis.code_bg_color = alpha_color(surface2, input_a.max(150));
    vis.faint_bg_color = alpha_color(surface2, input_a.saturating_sub(35));
    vis.window_stroke = egui::Stroke::new(1.0_f32, border);
    vis.window_rounding = r;
    vis.menu_rounding = r;
    vis.window_shadow = shadow;
    vis.popup_shadow = shadow;
    vis.window_highlight_topmost = false;
    vis.hyperlink_color = accent;
    vis.selection.bg_fill = accent.gamma_multiply(0.82);
    vis.selection.stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
    vis.slider_trailing_fill = true;
    vis.interact_cursor = Some(egui::CursorIcon::PointingHand);
    vis.button_frame = true;
    vis.collapsing_header_frame = false;
    vis.striped = false;

    vis.widgets.noninteractive.bg_fill = with_alpha(surface, card_a, glass);
    vis.widgets.noninteractive.weak_bg_fill = with_alpha(surface, card_a, glass);
    vis.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, border);
    vis.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, text);
    vis.widgets.noninteractive.rounding = r;

    vis.widgets.inactive.bg_fill = with_alpha(surface2, input_a, glass);
    vis.widgets.inactive.weak_bg_fill = with_alpha(surface2, input_a, glass);
    vis.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, border);
    vis.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, text);
    vis.widgets.inactive.rounding = r_small;
    vis.widgets.inactive.expansion = 0.0;

    vis.widgets.hovered.bg_fill = alpha_color(
        mix_color(surface2, accent, 0.30),
        (input_a as f32 + 22.0).min(248.0) as u8,
    );
    vis.widgets.hovered.weak_bg_fill = vis.widgets.hovered.bg_fill;
    vis.widgets.hovered.bg_stroke = egui::Stroke::new(
        1.0 + theme.glow as f32 / 110.0,
        alpha_color(accent, (70.0 + theme.glow as f32 * 1.4).min(235.0) as u8),
    );
    vis.widgets.hovered.fg_stroke = egui::Stroke::new(1.5_f32, text);
    vis.widgets.hovered.rounding = r_small;
    vis.widgets.hovered.expansion = 1.0;

    vis.widgets.active.bg_fill = alpha_color(
        mix_color(surface2, accent, 0.50),
        (input_a as f32 + 12.0).min(250.0) as u8,
    );
    vis.widgets.active.weak_bg_fill = vis.widgets.active.bg_fill;
    vis.widgets.active.bg_stroke = egui::Stroke::new(1.5_f32, accent);
    vis.widgets.active.fg_stroke = egui::Stroke::new(1.5_f32, egui::Color32::WHITE);
    vis.widgets.active.rounding = r_small;
    vis.widgets.active.expansion = 0.0;

    vis.widgets.open = vis.widgets.hovered;
    vis.widgets.open.rounding = r_small;

    ctx.set_visuals(vis);
    ctx.style_mut(|style| {
        use egui::{FontFamily, FontId, TextStyle};
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::new(21.0, FontFamily::Proportional));
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(14.5, FontFamily::Proportional));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::new(11.5, FontFamily::Proportional));
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 24.0;
        style.spacing.slider_width = 165.0;
        style.spacing.window_margin = egui::Margin::same(12.0);
        style.spacing.menu_margin = egui::Margin::same(8.0);
        style.spacing.scroll.floating = true;
        style.spacing.scroll.bar_width = 9.0;
        style.spacing.scroll.floating_width = 5.0;
        style.spacing.scroll.handle_min_length = 28.0;
        style.spacing.scroll.dormant_handle_opacity = 0.32;
        style.spacing.scroll.active_handle_opacity = 0.55;
        style.spacing.scroll.interact_handle_opacity = 0.92;
        style.spacing.scroll.foreground_color = false;
    });
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
/// e.g. YouTube URLs without yt-dlp handling). Short timeout so dead
/// streams fail fast instead of hanging the queue.
fn video_duration(input: &str) -> Option<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-timeout",
            "10000000",
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

/// Mosaic geometry: 120px cells tile to a 360x240 JPEG (~15-25KB at
/// quality 62) — still sharp in the ~300px card slot, ~half the bytes
/// of the old 160px / 480x320 layout.
const MOSAIC_CELL: u32 = 120;
const MOSAIC_COLS: usize = 3;
const MOSAIC_JPEG_QUALITY: u8 = 62;

fn mosaic_black_cell() -> image::RgbImage {
    image::RgbImage::from_pixel(
        MOSAIC_CELL,
        MOSAIC_CELL,
        image::Rgb([10, 10, 16]),
    )
}

/// Tile `cells` (each MOSAIC_CELL², missing slots black-filled) 3x2 and
/// encode straight to memory — no intermediate mosaic file on disk.
fn encode_mosaic_jpeg(cells: &[image::RgbImage]) -> Option<Vec<u8>> {
    let (cw, rows) = (MOSAIC_CELL, cells.len().div_ceil(MOSAIC_COLS).max(1) as u32);
    let mut canvas = image::RgbImage::new(cw * MOSAIC_COLS as u32, cw * rows);
    // paint background so short/partial mosaics have no garbage pixels
    for p in canvas.pixels_mut() {
        *p = image::Rgb([10, 10, 16]);
    }
    for (i, cell) in cells.iter().enumerate() {
        if cell.width() != MOSAIC_CELL || cell.height() != MOSAIC_CELL {
            continue;
        }
        image::imageops::replace(
            &mut canvas,
            cell,
            ((i % MOSAIC_COLS) * MOSAIC_CELL as usize) as i64,
            ((i / MOSAIC_COLS) * MOSAIC_CELL as usize) as i64,
        );
    }
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, MOSAIC_JPEG_QUALITY)
        .encode_image(&image::DynamicImage::ImageRgb8(canvas))
        .ok()?;
    if buf.is_empty() {
        return None;
    }
    Some(buf)
}
/// 6 frames equally spaced across the whole video, square-cropped and
/// tiled 3x2 into one 360x240 mosaic JPEG.
/// Engine 1: ffprobe duration + ffmpeg seeks (fast, precise).
/// Engine 2: mpv percent-seeks (needs only mpv — works outside nix-shell
/// and for anything mpv can open, incl. YouTube URLs).
/// Both engines are tolerant: a failed frame reuses the previous good one
/// (or black) so one bad seek doesn't lose the whole mosaic.
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
    // Smaller cells (120px), no audio/subtitles, cheap JPEG: each grab is
    // faster to decode, scale, write and re-read than the old 160px/q5.
    let vf = format!(
        "scale={c}:{c}:force_original_aspect_ratio=increase,crop={c}:{c}",
        c = MOSAIC_CELL
    );
    let mut cells: Vec<image::RgbImage> = Vec::with_capacity(6);
    let mut good = 0;
    for i in 0..6 {
        // endpoints inclusive: first cell in the first second, last cell in
        // the last second, rest evenly between.
        let t = (0.5 + i as f64 * (dur - 1.0) / 5.0).clamp(0.0, dur * 0.98);
        // hybrid seek: fast-seek to just before, then accurate-seek the rest
        // (plain input -ss snaps to keyframes and clusters frames).
        let pre = (t - 5.0).max(0.0);
        let post = t - pre;
        let out = dir.join(format!("c{i}.jpg"));
        let ok = Command::new("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-an",
                "-sn",
                "-dn",
                "-ss",
                &format!("{pre:.2}"),
                "-timeout",
                "10000000",
                "-i",
                input,
                "-ss",
                &format!("{post:.2}"),
                "-vframes",
                "1",
                "-vf",
                &vf,
                "-q:v",
                "9",
            ])
            .arg(&out)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        // Tolerant: reuse the previous good frame (or black) instead of
        // scrapping the whole mosaic when one seek fails.
        if ok {
            if let Ok(img) = image::open(&out) {
                let cell = img.to_rgb8();
                if cell.width() == MOSAIC_CELL && cell.height() == MOSAIC_CELL {
                    cells.push(cell);
                    good += 1;
                    continue;
                }
            }
        }
        cells.push(cells.last().cloned().unwrap_or_else(mosaic_black_cell));
    }
    std::fs::remove_dir_all(&dir).ok();
    if good == 0 {
        return None;
    }
    encode_mosaic_jpeg(&cells)
}

/// Mosaic engine 2: 6 mpv screenshots at evenly spaced percents of the
/// video, square-cropped in-process. Needs only mpv (no ffprobe/ffmpeg).
/// Screenshots are pre-scaled to 640px wide (less to write/decode) and
/// kept in memory — no intermediate cell files, no re-read of the mosaic.
/// All 6 shots run in parallel threads; if the FIRST shot fails the video
/// is skipped outright (dead/unreadable source — no point running the rest).
/// Other failed frames reuse the previous good one (or black).
fn mpv_shot(input: &str, pct: f64, celldir: &Path) -> Option<image::RgbImage> {
    std::fs::create_dir_all(celldir).ok()?;
    let ok = Command::new("mpv")
        .arg("--no-terminal")
        .arg("--really-quiet")
        .arg("--no-audio")
        .arg("--frames=1")
        .arg("--network-timeout=10")
        .arg(format!("--start={pct:.2}%"))
        .arg("--vo=image")
        .arg("--vo-image-format=jpg")
        .arg("--vo-image-jpeg-quality=60")
        .arg("--vf=scale=640:-2")
        .arg(format!("--vo-image-outdir={}", celldir.display()))
        .arg(input)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let shot = ok
        .then(|| {
            std::fs::read_dir(celldir).ok()?.flatten().map(|e| e.path()).find(|p| {
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("jpg"))
                    .unwrap_or(false)
            })
        })
        .flatten()?;
    // square-crop the screenshot to a 120x120 cell in-process,
    // keeping the decoded pixels — no cell file round-trip.
    let img = image::open(&shot).ok()?;
    Some(
        img.resize_to_fill(MOSAIC_CELL, MOSAIC_CELL, image::imageops::FilterType::Triangle)
            .to_rgb8(),
    )
}

fn grab_mosaic_mpv(input: &str) -> Option<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("scwoll-mosaic-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).ok()?;
    // same endpoint-inclusive spacing as the ffmpeg engine, in percent:
    // first cell ~first second, last cell ~last second.
    let pcts: Vec<f64> = (0..6).map(|i| 0.5 + i as f64 * 99.0 / 5.0).collect();
    let mut shots: Vec<Option<image::RgbImage>> = vec![None, None, None, None, None, None];
    std::thread::scope(|s| {
        let handles: Vec<_> = pcts
            .iter()
            .enumerate()
            .map(|(i, &pct)| {
                let celldir = dir.join(format!("c{i}"));
                s.spawn(move || mpv_shot(input, pct, &celldir))
            })
            .collect();
        for (i, h) in handles.into_iter().enumerate() {
            shots[i] = h.join().ok().flatten();
            if shots[i].is_none() {
                eprintln!("[scwoll] thumb: mpv shot {i}/6 failed for {input}");
            }
        }
    });
    std::fs::remove_dir_all(&dir).ok();
    // First shot failed: the source is dead — skip the video entirely.
    if shots[0].is_none() {
        eprintln!("[scwoll] thumb: mpv first shot failed for {input} (skipping)");
        return None;
    }
    // shots[0] is good, so `cells.last()` always exists for gaps below.
    let mut cells: Vec<image::RgbImage> = Vec::with_capacity(6);
    for shot in shots {
        match shot {
            Some(cell) => cells.push(cell),
            None => cells.push(cells.last().cloned().unwrap_or_else(mosaic_black_cell)),
        }
    }
    encode_mosaic_jpeg(&cells)
}

fn load_thumb_bytes(v: &Video) -> Option<Vec<u8>> {
    let cache = thumb_cache_file(&v.id);
    if let Ok(b) = std::fs::read(&cache) {
        if !b.is_empty() {
            return Some(b);
        }
    }
    // Failed recently (unplayable/dead source)? Don't burn mpv runs every
    // launch — the cooldown marker says when it's worth retrying.
    if thumb_failed_recently(&v.id) {
        return None;
    }
    // YouTube thumbnail image
    if !v.thumbnail.is_empty() {
        if let Some(b) = download_bytes(&v.thumbnail) {
            if let Some(parent) = cache.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&cache, &b).ok();
            std::fs::remove_file(thumb_failed_file(&v.id)).ok();
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
        std::fs::remove_file(thumb_failed_file(&v.id)).ok();
        return Some(b);
    }
    // Total failure: stamp a cooldown marker so the next launches use the
    // letter tile instead of re-running doomed ffmpeg/mpv work.
    mark_thumb_failed(&v.id);
    eprintln!("[scwoll] thumb: no artwork and no mosaic for {input} (keeps letter tile)");
    None
}

/// Cooldown before retrying a video whose thumbnail failed completely.
const THUMB_FAIL_RETRY_SECS: u64 = 24 * 3600;

fn thumb_failed_file(key: &str) -> PathBuf {
    data_dir().join("thumbs").join(format!("{key}.failed"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn thumb_failed_recently(key: &str) -> bool {
    let s = match std::fs::read_to_string(thumb_failed_file(key)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    match s.trim().parse::<u64>() {
        Ok(ts) => now_secs().saturating_sub(ts) < THUMB_FAIL_RETRY_SECS,
        Err(_) => false,
    }
}

fn mark_thumb_failed(key: &str) {
    let p = thumb_failed_file(key);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&p, now_secs().to_string()).ok();
}

// use Read trait in fetch
use std::io::Read as _;
use wait_timeout::ChildExt;

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum SettingsTab {
    Appearance,
    Glass,
    Colors,
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
    settings_tab: SettingsTab,
    import_tab: ImportTab,
    // import buffers
    in_url: String,
    in_url_cat: String,
    in_url_title: String,
    in_bulk: String,
    in_bulk_cat: String,
    bulk_pin_end: bool,
    in_folder: String,
    in_folder_cat: String,
    in_recursive: bool,
    // theater edit buffers
    edit_title: String,
    edit_cat: String,
    show_cats: bool,
    cat_edit_buf: HashMap<String, String>,
    // sources: bookmarked HTML pages browsed in-app
    sources: Vec<Source>,
    showing_sources: bool,
    active_source_id: Option<String>,
    source_input: String,
    source_url_bar: String,
    source_cache: HashMap<String, SourcePage>,
    source_loading: HashSet<String>,
    source_rx: Receiver<SourceFetchMsg>,
    source_tx: Sender<SourceFetchMsg>,
    source_img_rx: Receiver<SourceImgMsg>,
    source_img_tx: Sender<SourceImgMsg>,
    source_img_cache: HashMap<String, egui::TextureHandle>,
    source_img_pending: HashSet<String>,
    player: Player,
    toast: Option<(String, Instant)>,
    // thumbnails: `thumb_job_tx` is the FIFO queue, drained by a pool of
    // background workers sized from free RAM; `thumb_rx` carries results.
    thumb_rx: Receiver<ThumbMsg>,
    thumb_job_tx: Sender<Video>,
    thumb_cache: HashMap<String, egui::TextureHandle>,
    thumb_pending: HashSet<String>,
}

/// Free RAM in bytes (Linux: MemAvailable from /proc/meminfo).
fn free_ram_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// How many video thumbnail jobs to run at once: one per ~2 GiB of free
/// RAM (a job can hold 6 mpv children in flight), clamped to 1..=4 so a
/// big machine doesn't fork-bomb itself. Falls back to CPU count.
fn thumb_worker_count() -> usize {
    const PER_TASK_BYTES: u64 = 2 * 1024 * 1024 * 1024;
    match free_ram_bytes() {
        Some(free) => ((free / PER_TASK_BYTES) as usize).clamp(1, 4),
        None => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 4),
    }
}

impl ScwollApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        let (job_tx, job_rx) = channel::<Video>();
        let (source_tx, source_rx) = channel::<SourceFetchMsg>();
        let (source_img_tx, source_img_rx) = channel::<SourceImgMsg>();
        // Worker pool sharing one queue: N videos at a time, N from free
        // RAM. The mutex is only held for the recv itself, never while a
        // thumbnail is being built.
        let jobs = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
        let n_workers = thumb_worker_count();
        eprintln!("[scwoll] thumb: {n_workers} parallel video tasks");
        for _ in 0..n_workers {
            let jobs = jobs.clone();
            let out = tx.clone();
            std::thread::spawn(move || loop {
                let vid: Video = match jobs.lock().ok().and_then(|q| q.recv().ok()) {
                    Some(v) => v,
                    None => break,
                };
                let msg = match load_thumb_bytes(&vid).and_then(|b| decode_thumb_bytes(&b)) {
                    Some(image) => ThumbMsg::Done { key: vid.id, image },
                    None => ThumbMsg::Failed { key: vid.id },
                };
                if out.send(msg).is_err() {
                    break;
                }
            });
        }
        drop(tx);
        Self {
            videos: load_videos(),
            theme: load_theme(),
            active_category: "All".into(),
            source: SourceFilter::All,
            search: String::new(),
            selected: None,
            show_import: false,
            show_theme: false,
            settings_tab: SettingsTab::Appearance,
            import_tab: ImportTab::Url,
            in_url: String::new(),
            in_url_cat: String::new(),
            in_url_title: String::new(),
            in_bulk: String::new(),
            in_bulk_cat: String::new(),
            bulk_pin_end: false,
            in_folder: String::new(),
            in_folder_cat: String::new(),
            in_recursive: true,
            edit_title: String::new(),
            edit_cat: String::new(),
            show_cats: false,
            cat_edit_buf: HashMap::new(),
            sources: load_sources(),
            showing_sources: false,
            active_source_id: None,
            source_input: String::new(),
            source_url_bar: String::new(),
            source_cache: HashMap::new(),
            source_loading: HashSet::new(),
            source_rx: source_rx,
            source_tx: source_tx,
            source_img_rx: source_img_rx,
            source_img_tx: source_img_tx,
            source_img_cache: HashMap::new(),
            source_img_pending: HashSet::new(),
            player: Player::new(),
            toast: None,
            thumb_rx: rx,
            thumb_job_tx: job_tx,
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

    /// Delete a video and its cached thumbnail files.
    /// Removes `data/thumbs/<id>.jpg` + `<id>.failed`, drops any
    /// in-memory texture / pending job, stops playback of it, persists.
    fn delete_video(&mut self, id: &str) {
        if self.player.current_id.as_deref() == Some(id) {
            self.player.stop();
        }
        self.videos.retain(|x| x.id != id);
        self.thumb_cache.remove(id);
        self.thumb_pending.remove(id);
        std::fs::remove_file(thumb_cache_file(id)).ok();
        std::fs::remove_file(thumb_failed_file(id)).ok();
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
        if self.active_category != "All"
            && self.active_category != "Favorites"
            && !self.videos.iter().any(|v| v.category == self.active_category)
        {
            self.active_category = "All".into();
        }
        self.persist();
    }

    /// Change one video's category (normalizes blank -> "General").
    fn set_video_category(&mut self, id: &str, cat: &str) {
        let cat = normalize_category(Some(cat.to_string()));
        if let Some(slot) = self.videos.iter_mut().find(|x| x.id == id) {
            slot.category = cat;
        }
        self.persist();
    }

    /// Rename a whole category: every video in `old` moves to `new`.
    /// Returns number of videos moved. No-op (with toast msg) on bad input.
    fn rename_category(&mut self, old: &str, new: &str) -> Result<usize, String> {
        let new = normalize_category(Some(new.to_string()));
        if old == new {
            return Err("Same name — nothing to do.".into());
        }
        if !self.videos.iter().any(|v| v.category == old) {
            return Err("Category not found.".into());
        }
        let mut n = 0;
        for v in self.videos.iter_mut().filter(|v| v.category == old) {
            v.category = new.clone();
            n += 1;
        }
        if self.active_category == old {
            self.active_category = new.clone();
        }
        if self.edit_cat == old {
            self.edit_cat = new.clone();
        }
        self.cat_edit_buf.remove(old);
        self.persist();
        Ok(n)
    }

    /// Delete a category: its videos move to "General" (never deleted).
    /// Returns number of videos moved.
    fn clear_category(&mut self, cat: &str) -> usize {
        let mut n = 0;
        for v in self.videos.iter_mut().filter(|v| v.category == cat) {
            v.category = "General".into();
            n += 1;
        }
        if self.active_category == cat {
            self.active_category = "All".into();
        }
        self.cat_edit_buf.remove(cat);
        self.persist();
        n
    }

    // ---------------- sources ----------------
    fn persist_sources(&self) {
        save_sources(&self.sources);
    }
    fn active_source(&self) -> Option<Source> {
        self.active_source_id
            .as_deref()
            .and_then(|id| self.sources.iter().find(|s| s.id == id).cloned())
    }
    /// Add one pasted line: it may hold a single URL or a whole list
    /// (lines/spaces/commas, like the bulk importer). Returns (added, skipped).
    fn add_sources_bulk(&mut self, raw: &str) -> (usize, usize) {
        let mut cands = split_url_list(raw);
        if cands.is_empty() {
            if let Some(u) = normalize_source_url(raw) {
                cands.push(u);
            }
        }
        let mut added = 0;
        let mut skipped = 0;
        for c in cands {
            let norm = match normalize_source_url(&c) {
                Some(u) => u,
                None => {
                    skipped += 1;
                    continue;
                }
            };
            if self.sources.iter().any(|s| s.url == norm) {
                skipped += 1;
                continue;
            }
            self.sources.push(make_source(norm));
            added += 1;
        }
        if added > 0 {
            // newest first
            self.sources.sort_by(|a, b| b.added_at.cmp(&a.added_at));
            self.persist_sources();
        }
        (added, skipped)
    }
    fn remove_source(&mut self, id: &str) {
        self.sources.retain(|s| s.id != id);
        self.source_cache.remove(id);
        self.source_loading.remove(id);
        if self.active_source_id.as_deref() == Some(id) {
            self.active_source_id = None;
            self.source_url_bar.clear();
            self.showing_sources = true;
        }
        self.persist_sources();
    }
    /// Single entry point for the Sources page.
    fn open_sources_page(&mut self) {
        self.showing_sources = true;
        self.active_source_id = None;
        self.selected = None;
    }
    /// Open a URL in the Sources view: selects the saved entry when known,
    /// otherwise saves it first so Back/refresh keep working.
    fn open_source_url(&mut self, raw: &str) -> Result<(), String> {
        let norm = normalize_source_url(raw).ok_or("That doesn't look like an http(s) URL.")?;
        if let Some(s) = self.sources.iter().find(|s| s.url == norm).cloned() {
            self.active_source_id = Some(s.id.clone());
            self.showing_sources = false;
            self.selected = None;
            self.source_url_bar = s.url.clone();
            self.request_source(&s.id, &s.url);
        } else {
            let s = make_source(norm.clone());
            let id = s.id.clone();
            self.sources.push(s);
            self.sources.sort_by(|a, b| b.added_at.cmp(&a.added_at));
            self.persist_sources();
            self.active_source_id = Some(id.clone());
            self.showing_sources = false;
            self.selected = None;
            self.source_url_bar = norm.clone();
            self.request_source(&id, &norm);
        }
        Ok(())
    }
    fn request_source(&mut self, id: &str, url: &str) {
        self.request_source_inner(id, url, false);
    }
    /// Re-fetch forcing a post-JS DOM snapshot (manual "Load with JS").
    fn request_source_js(&mut self, id: &str, url: &str) {
        self.source_cache.remove(id);
        self.source_loading.remove(id);
        self.request_source_inner(id, url, true);
        self.toast("Rendering page JS…");
    }
    fn request_source_inner(&mut self, id: &str, url: &str, force_js: bool) {
        if self.source_cache.contains_key(id) || self.source_loading.contains(id) {
            return;
        }
        self.source_loading.insert(id.to_string());
        let tx = self.source_tx.clone();
        let id = id.to_string();
        let url = url.to_string();
        std::thread::spawn(move || {
            let result = fetch_source_page(&url, force_js);
            tx.send(SourceFetchMsg { id, result }).ok();
        });
    }
    fn pump_sources(&mut self) {
        while let Ok(msg) = self.source_rx.try_recv() {
            self.source_loading.remove(&msg.id);
            let page = match msg.result {
                Ok((title, links, doc, js)) => {
                    // remember the page title as the source label
                    if !title.is_empty() {
                        if let Some(s) = self.sources.iter_mut().find(|s| s.id == msg.id) {
                            s.title = title.clone();
                        }
                        self.persist_sources();
                    }
                    SourcePage { title, links, doc, fetched_at: now_iso(), error: None, js_rendered: js }
                }
                Err(e) => SourcePage { title: String::new(), links: vec![], doc: HtmlDoc::default(), fetched_at: now_iso(), error: Some(e), js_rendered: false },
            };
            self.source_cache.insert(msg.id, page);
        }
    }
    fn request_source_image(&mut self, url: &str) {
        if self.source_img_cache.contains_key(url) || self.source_img_pending.contains(url) {
            return;
        }
        // cap in-flight image jobs so a photo-heavy page can't fork-bomb
        if self.source_img_pending.len() >= 8 {
            return;
        }
        self.source_img_pending.insert(url.to_string());
        let tx = self.source_img_tx.clone();
        let url = url.to_string();
        std::thread::spawn(move || {
            let image = fetch_page_image(&url);
            tx.send(SourceImgMsg { url, image }).ok();
        });
    }
    fn pump_source_images(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.source_img_rx.try_recv() {
            self.source_img_pending.remove(&msg.url);
            if let Some(image) = msg.image {
                let tex = ctx.load_texture(format!("srcimg-{}", msg.url), image, egui::TextureOptions::LINEAR);
                self.source_img_cache.insert(msg.url, tex);
            }
        }
    }

    fn filtered(&self) -> Vec<&Video> {
        let q = self.search.trim().to_lowercase();
        let mut out: Vec<&Video> = self
            .videos
            .iter()
            .filter(|v| {
                if self.active_category == "Favorites" {
                    if !v.favorite {
                        return false;
                    }
                } else if self.active_category != "All" && v.category != self.active_category {
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

    /// Retry a failed thumbnail: clear the failure cooldown marker so the
    /// worker really re-runs ffmpeg/mpv instead of skipping, then re-queue.
    fn retry_thumb(&mut self, v: &Video) {
        std::fs::remove_file(thumb_failed_file(&v.id)).ok();
        self.thumb_pending.remove(&v.id);
        self.request_thumb(v);
        self.toast("Retrying thumbnail…");
    }

    fn request_thumb(&mut self, v: &Video) {
        if self.thumb_cache.contains_key(&v.id) || self.thumb_pending.contains(&v.id) {
            return;
        }
        // Failed inside the cooldown window: don't re-queue (and spin the
        // workers) every frame — the tile keeps its retry button instead.
        if thumb_failed_recently(&v.id) {
            return;
        }
        // Queue instead of spawning per-video: the worker pool above
        // drains this list, N videos at a time based on free RAM.
        self.thumb_pending.insert(v.id.clone());
        self.thumb_job_tx.send(v.clone()).ok();
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
        paint_backdrop(ctx, &self.theme);
        self.pump_thumbs(ctx);
        self.pump_sources();
        self.pump_source_images(ctx);
        let animated = self.theme.animate && self.theme.aurora && self.theme.ambient > 0;
        ctx.request_repaint_after(Duration::from_millis(if animated { 33 } else { 500 }));

        let glass = self.theme.glass;
        let accent = hex_to_color(&self.theme.accent);
        let accent2 = hex_to_color(&self.theme.accent2);
        let muted = hex_to_color(&self.theme.muted);
        let panel_a = panel_alpha(&self.theme);
        let top_fill = with_alpha(hex_to_color(&self.theme.surface), panel_a, glass);
        let sidebar_fill = with_alpha(hex_to_color(&self.theme.sidebar), panel_a, glass);
        let panel_line = alpha_color(accent, (45.0 + self.theme.glow as f32 * 1.5).min(190.0) as u8);
        let panel_line2 = alpha_color(accent2, (35.0 + self.theme.glow as f32 * 1.1).min(150.0) as u8);

        // ---------------- top bar ----------------
        let top_resp = egui::TopBottomPanel::top("topbar")
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .fill(top_fill)
                    .inner_margin(egui::Margin::symmetric(14.0, 8.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("▶").color(accent).size(24.0));
                    ui.heading(egui::RichText::new("Scwoll").color(accent).strong());
                    ui.label(egui::RichText::new("tube").color(muted).small());
                    ui.add_space(10.0);
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Search videos, categories, urls…")
                            .desired_width(320.0),
                    );
                    let _ = resp;
                    if ui.button("✕").on_hover_text("Clear search").clicked() {
                        self.search.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚙ Settings").on_hover_text("Appearance, glass and colour options").clicked() {
                            self.show_theme = true;
                        }
                        if ui.button("＋ Import").clicked() {
                            self.show_import = true;
                        }
                    });
                });
            });
        paint_panel_edge(ctx, top_resp.response.rect, true, panel_line, panel_line2);

        // ---------------- sidebar ----------------
        let side_resp = egui::SidePanel::left("sidebar")
            .exact_width(228.0)
            .frame(
                egui::Frame::side_top_panel(&ctx.style())
                    .fill(sidebar_fill)
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
            )
            .show(ctx, |ui| {
                // One scrollbar for the whole sidebar: categories scroll with
                // the sidebar instead of trapping scroll in a nested box.
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.heading("Library");
                for s in [SourceFilter::All, SourceFilter::Youtube, SourceFilter::Local, SourceFilter::Direct] {
                    let label = match s {
                        SourceFilter::All => format!("🏠 {}", s.label()),
                        SourceFilter::Youtube => format!("▶ {}", s.label()),
                        SourceFilter::Local => format!("🗂 {}", s.label()),
                        SourceFilter::Direct => format!("🔗 {}", s.label()),
                    };
                    if ui.selectable_label(self.source == s && !self.showing_sources && self.active_source_id.is_none(), label).clicked() {
                        self.source = s;
                        self.selected = None;
                        self.active_source_id = None;
                        self.showing_sources = false;
                    }
                }
                // Single entry point to the Sources page.
                if ui.selectable_label(self.showing_sources || self.active_source_id.is_some(), format!("🌐 Sources ({})", self.sources.len())).on_hover_text("Saved web pages — rendered in-app").clicked() {
                    self.open_sources_page();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Categories");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("⚙ Manage").on_hover_text("Rename / move / clear categories").clicked() {
                            self.show_cats = true;
                        }
                    });
                });
                let cats = self.categories();
                if ui.selectable_label(self.active_category == "All" && !self.showing_sources && self.active_source_id.is_none(), format!("All ({})", self.videos.len())).clicked() {
                        self.active_category = "All".into();
                        self.active_source_id = None;
                        self.showing_sources = false;
                    }
                    let fav_n = self.videos.iter().filter(|x| x.favorite).count();
                    if ui.selectable_label(self.active_category == "Favorites" && !self.showing_sources && self.active_source_id.is_none(), format!("★ Favorites ({fav_n})")).clicked() {
                        self.active_category = "Favorites".into();
                        self.active_source_id = None;
                        self.showing_sources = false;
                    }
                    for (name, count) in &cats {
                        if ui.selectable_label(self.active_category == *name && !self.showing_sources && self.active_source_id.is_none(), format!("{name} ({count})")).clicked() {
                            self.active_category = name.clone();
                            self.active_source_id = None;
                            self.showing_sources = false;
                        }
                    }
                    if cats.is_empty() {
                        ui.label(egui::RichText::new("No categories yet — import something!").color(muted).small());
                    }
                ui.separator();
                let (yt, lo, di) = (
                    self.videos.iter().filter(|v| v.kind == VideoKind::Youtube).count(),
                    self.videos.iter().filter(|v| v.kind == VideoKind::Local).count(),
                    self.videos.iter().filter(|v| v.kind == VideoKind::Direct).count(),
                );
                ui.small(format!("📼 {} total · ▶ {yt} · 🗂 {lo} · 🔗 {di}", self.videos.len()));
                ui.small(format!("Player: {}", if self.player.child.is_some() { "busy" } else { "idle" }));
                });
            });
        paint_panel_edge(ctx, side_resp.response.rect, false, panel_line, panel_line2);

        // ---------------- main ----------------
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(&ctx.style())
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(16.0, 10.0)),
            )
            .show(ctx, |ui| {
            // stale source selection (deleted elsewhere)? drop it.
            if let Some(id) = self.active_source_id.clone() {
                if !self.sources.iter().any(|s| s.id == id) {
                    self.active_source_id = None;
                }
            }
            if self.showing_sources {
                self.show_sources_list(ui, accent, muted);
            } else if self.active_source_id.is_some() {
                self.show_source_doc(ui, ctx, accent, muted);
            } else if let Some(id) = self.selected.clone() {
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
        }        if self.show_cats {
            self.categories_window(ctx, muted);
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
        // Categories live in the sidebar — no duplicate category row here
        // eating vertical space or stealing scroll.
        let list: Vec<Video> = self.filtered().into_iter().cloned().collect();
        let all_cats: Vec<String> = self.categories().into_iter().map(|(n, _)| n).collect();
        ui.horizontal(|ui| {
            ui.heading(if self.active_category == "All" { self.source.label() } else { self.active_category.as_str() });
            ui.label(egui::RichText::new(format!("{} videos", list.len())).color(muted));
        });

        if list.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                if self.active_category == "Favorites" {
                    ui.heading("☆ No favorites yet");
                    ui.label("Tap ☆ on any video to pin it here.");
                } else {
                    ui.heading("📼 Nothing here yet");
                    ui.label("Import a URL, a .txt list, or a folder of videos.");
                    if ui.button("＋ Import videos").clicked() {
                        self.show_import = true;
                    }
                }
            });
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // Responsive shelf: fit as many columns as the window holds at
            // the target width, then stretch cards to fill each row exactly.
            // Columns always fit — nothing runs off the right edge.
            let avail = ui.available_width().max(120.0);
            let gap = egui::vec2(10.0, 10.0);
            let target = (220.0 * self.theme.card_scale).clamp(140.0, 320.0);
            let cols = (((avail + gap.x) / (target + gap.x)).floor() as usize).max(1);
            let card_w =
                ((avail - gap.x * (cols.saturating_sub(1) as f32)) / cols as f32).max(110.0);
            // Frame adds inner margin + stroke; keep the content width a
            // little narrower so the rounded glass card fits exactly.
            let content_w = (card_w - 20.0).max(80.0);
            let slot_h = content_w * 9.0 / 16.0;
            let rounding = egui::Rounding::same(self.theme.radius.min(22) as f32);
            let img_rounding = egui::Rounding::same((self.theme.radius.min(18) as f32 - 2.0).max(2.0));
            let glass = self.theme.glass;
            let surface = hex_to_color(&self.theme.surface);
            let card_fill = with_alpha(surface, card_alpha(&self.theme), glass);
            let card_stroke = alpha_color(
                mix_color(hex_to_color(&self.theme.muted), accent, 0.35),
                (22.0 + self.theme.glow as f32 * 1.35).min(140.0) as u8,
            );
            let card_shadow = theme_shadow(&self.theme);
            let hover_glow = self.theme.hover_glow;
            let glow_width = 1.0 + self.theme.glow as f32 / 70.0;
            let glow_color = alpha_color(accent, (115.0 + self.theme.glow as f32 * 1.2).min(245.0) as u8);

            // Explicit grid rows: exactly `cols` cards per row, rows stacked
            // vertically inside the scroll area. No wrap heuristics — the
            // list fills vertical space and scrolls instead of snaking off
            // the right edge where it can't be reached.
            ui.spacing_mut().item_spacing.y = gap.y;
            for chunk in list.chunks(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap.x;
                    for v in chunk.iter() {
                        let vid = v.clone();
                        self.request_thumb(v);
                        let mut opened = false;
                        let mut play = false;
                        let mut delete = false;
                        let mut fav_toggle = false;
                        let mut new_cat: Option<String> = None;
                        // ============ video box ============
                        // Glass card: translucent fill, accent rim, soft depth
                        // shadow and an optional glow on hover.
                        let card = egui::Frame::none()
                            .fill(card_fill)
                            .stroke(egui::Stroke::new(1.0_f32, card_stroke))
                            .rounding(rounding)
                            .inner_margin(egui::Margin::same(8.0))
                            .shadow(card_shadow)
                            .show(ui, |ui| {
                                // Pin the content width so the card (content +
                                // margins) is exactly card_w — the wrapping row
                                // can never push it past the window edge.
                                ui.set_min_width(content_w);
                                ui.set_max_width(content_w);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                // Force vertical stacking inside the box: the wrapping
                                // row outside hands down a HORIZONTAL layout, which is
                                // what glued the buttons beside the thumbnail and threw
                                // the second box lower (rows center uneven heights).
                                ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                                // (width already pinned above; no second set_width)
                                let src = match vid.kind {
                                    VideoKind::Youtube => "\u{25b6} YouTube",
                                    VideoKind::Local => "\u{1F5C2} Local",
                                    VideoKind::Direct => "\u{1F517} Link",
                                };
                                // top row of the box: source + category menu left,
                                // favorite star right.
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(egui::RichText::new(src).color(muted).small());
                                    ui.label(egui::RichText::new("\u{00b7}").color(muted).small());
                                    let cat_resp = ui.menu_button(
                                        egui::RichText::new(format!("{} ▾", vid.category))
                                            .color(accent)
                                            .small()
                                            .strong(),
                                        |ui| {
                                            for c in &all_cats {
                                                if ui.selectable_label(*c == vid.category, c.clone()).clicked() {
                                                    new_cat = Some(c.clone());
                                                    ui.close_menu();
                                                }
                                            }
                                            ui.separator();
                                            if ui.selectable_label(false, "✎ Edit / new…").clicked() {
                                                opened = true;
                                                ui.close_menu();
                                            }
                                        },
                                    );
                                    cat_resp.response.on_hover_text("Change category");
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        let star = if vid.favorite { "★" } else { "☆" };
                                        if ui.small_button(egui::RichText::new(star).color(if vid.favorite { accent } else { muted }))
                                            .on_hover_text(if vid.favorite { "Remove from Favorites" } else { "Add to Favorites" })
                                            .clicked()
                                        {
                                            fav_toggle = true;
                                        }
                                    });
                                });
                                ui.add_space(2.0);
                                // thumbnail (click to open); aspect-fit into the
                                // 16:9 slot. The box is only as tall as the
                                // fitted image, so no dead space sits below it.
                                let thumb_clicked = if let Some(tex) = self.thumb_cache.get(&vid.id) {
                                    let ts = tex.size();
                                    let s = (ts[0] as f32 / ts[1].max(1) as f32).clamp(0.2, 4.0);
                                    let disp_w = (slot_h * s).min(content_w);
                                    let disp = egui::vec2(disp_w.max(20.0), (disp_w / s).max(10.0));
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(content_w, disp.y),
                                        egui::Layout::top_down(egui::Align::Center),
                                        |ui| {
                                            let img = ui.add(
                                                egui::Image::from_texture(tex)
                                                    .fit_to_exact_size(disp)
                                                    .rounding(img_rounding)
                                                    .sense(egui::Sense::click()),
                                            );
                                            if img.hovered() {
                                                let center = img.rect.center();
                                                let r = (disp.y * 0.16).clamp(15.0, 26.0);
                                                ui.painter().circle_filled(
                                                    center,
                                                    r,
                                                    egui::Color32::from_black_alpha(130),
                                                );
                                                ui.painter().circle_stroke(
                                                    center,
                                                    r,
                                                    egui::Stroke::new(1.0_f32, alpha_color(egui::Color32::WHITE, 110)),
                                                );
                                                ui.painter().text(
                                                    center,
                                                    egui::Align2::CENTER_CENTER,
                                                    "▶",
                                                    egui::FontId::proportional(r * 0.9),
                                                    egui::Color32::WHITE,
                                                );
                                            }
                                            img.clicked()
                                        },
                                    )
                                    .inner
                                } else {
                                    let letter = vid
                                        .title
                                        .chars()
                                        .next()
                                        .unwrap_or('\u{25b6}')
                                        .to_uppercase()
                                        .to_string();
                                    let tile = ui.add_sized(
                                        egui::vec2(content_w, slot_h),
                                        egui::Button::new(
                                            egui::RichText::new(letter)
                                                .size(36.0)
                                                .color(egui::Color32::WHITE),
                                        )
                                        .rounding(img_rounding)
                                        .fill(placeholder_color(&vid.id)),
                                    );
                                    // Failed picture (.failed marker on disk):
                                    // overlay a retry button in the middle.
                                    if thumb_failed_file(&vid.id).exists() {
                                        let retry = ui.put(
                                            egui::Rect::from_center_size(
                                                tile.rect.center(),
                                                egui::vec2(76.0, 26.0),
                                            ),
                                            egui::Button::new("↻ Retry"),
                                        );
                                        if retry.clicked() {
                                            self.retry_thumb(&vid);
                                        }
                                    }
                                    tile.clicked()
                                };
                                if thumb_clicked {
                                    opened = true;
                                }
                                // one compact button row directly under the image;
                                // exact thirds of the content width so it fits.
                                ui.add_space(2.0);
                                let bw = (content_w - gap.x * 2.0) / 3.0;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = gap.x;
                                    if ui
                                        .add_sized(
                                            egui::vec2(bw, 22.0),
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
                                        .add_sized(egui::vec2(bw, 22.0), egui::Button::new("Open"))
                                        .clicked()
                                    {
                                        opened = true;
                                    }
                                    if ui
                                        .add_sized(egui::vec2(bw, 22.0), egui::Button::new("\u{1F5D1}"))
                                        .on_hover_text("Delete")
                                        .clicked()
                                    {
                                        delete = true;
                                    }
                                });
                                // title: one truncated line — compact, and every
                                // card stays the same height so rows line up.
                                ui.label(
                                    egui::RichText::new(shorten_title(&vid.title, content_w))
                                        .strong(),
                                );
                                });
                            });
                        if hover_glow && card.response.hovered() {
                            ui.painter().rect_stroke(
                                card.response.rect.shrink(0.5),
                                rounding,
                                egui::Stroke::new(glow_width, glow_color),
                            );
                        }
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
                            let title = vid.title.clone();
                            self.delete_video(&vid.id);
                            self.toast(format!("Deleted \"{title}\" ✓"));
                        }
                        if let Some(c) = new_cat {
                            if c != vid.category {
                                self.set_video_category(&vid.id, &c);
                                self.toast(format!("Moved to \"{c}\" ✓"));
                            }
                        }
                        if fav_toggle {
                            let now_fav = if let Some(slot) = self.videos.iter_mut().find(|x| x.id == vid.id) {
                                slot.favorite = !slot.favorite;
                                slot.favorite
                            } else {
                                false
                            };
                            self.persist();
                            self.toast(if now_fav { "★ Added to Favorites" } else { "☆ Removed from Favorites" });
                        }
                    }
                    });
            }
        });
    }

    /// Sources home page: every saved source in one place. Add URLs
    /// (single or bulk list), open one to render it in-app.
    fn show_sources_list(&mut self, ui: &mut egui::Ui, accent: egui::Color32, muted: egui::Color32) {
        if ui.button("← Back to library").clicked() {
            self.showing_sources = false;
            return;
        }
        ui.horizontal(|ui| {
            ui.heading("🌐 Sources");
            ui.label(egui::RichText::new(format!("{} saved", self.sources.len())).color(muted));
        });
        ui.label(egui::RichText::new("Save article / index / channel pages here, then open one to read it rendered inside the app. Links stay in-app; videos get ▶ Play / ＋ Import.").color(muted).small());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.source_input)
                    .hint_text("Paste page URL(s) — lines, spaces or commas all work…")
                    .desired_width(f32::INFINITY),
            );
            let commit = ui.button("＋ Add").clicked()
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if commit {
                let raw = self.source_input.clone();
                if raw.trim().is_empty() {
                    self.toast("Paste a page URL first.");
                } else {
                    let (a, sk) = self.add_sources_bulk(&raw);
                    if a > 0 {
                        self.toast(if sk > 0 { format!("Added {a} source(s), skipped {sk} ✓") } else { format!("Added {a} source(s) ✓") });
                        self.source_input.clear();
                    } else {
                        self.toast("Nothing new (bad URL or already added).");
                    }
                }
            }
        });
        ui.separator();
        let rows: Vec<Source> = self.sources.clone();
        if rows.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                ui.heading("🌐 No sources yet");
                ui.label("Paste any article, blog, docs or index page above — it will render inside the app.");
            });
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for s in &rows {
                egui::Frame::group(ui.style()).rounding(egui::Rounding::same(10.0)).show(ui, |ui| {
                    ui.set_min_width(ui.available_width().max(200.0));
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(source_label(s)).strong());
                            ui.label(egui::RichText::new(s.url.clone()).color(muted).small());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("🗑").on_hover_text("Remove source").clicked() {
                                self.remove_source(&s.id);
                                self.toast("Source removed ✓");
                            }
                            if ui.button("↗").on_hover_text("Open in system browser").clicked() {
                                open::that(&s.url).ok();
                            }
                            if ui.add(egui::Button::new(egui::RichText::new("Open ➜").color(egui::Color32::WHITE)).fill(accent)).on_hover_text("Render this page inside the app").clicked() {
                                self.showing_sources = false;
                                self.active_source_id = Some(s.id.clone());
                                self.selected = None;
                                self.source_url_bar = s.url.clone();
                                self.request_source(&s.id, &s.url);
                            }
                        });
                    });
                });
            }
        });
    }

    /// Render one saved source as a document page inside the app.
    fn show_source_doc(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, accent: egui::Color32, muted: egui::Color32) {
        let Some(src) = self.active_source() else {
            self.active_source_id = None;
            self.showing_sources = true;
            return;
        };
        let sid = src.id.clone();
        let surl = src.url.clone();
        if self.source_url_bar.is_empty() {
            self.source_url_bar = surl.clone();
        }
        ui.horizontal(|ui| {
            if ui.button("← Sources").clicked() {
                self.active_source_id = None;
                self.showing_sources = true;
                return;
            }
            if ui.button("← Library").clicked() {
                self.active_source_id = None;
                self.showing_sources = false;
                return;
            }
        });
        ui.add_space(2.0);
        ui.heading(egui::RichText::new(source_label(&src)).strong());
        ui.label(egui::RichText::new(surl.clone()).color(muted).small());
        // address bar: any URL renders in-app (auto-saved)
        let mut go_now = false;
        ui.horizontal(|ui| {
            ui.label("URL:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.source_url_bar)
                    .hint_text("https://…")
                    .desired_width(f32::INFINITY),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go_now = true;
            }
            if ui.button("Go ➜").clicked() {
                go_now = true;
            }
        });
        if go_now {
            let raw = self.source_url_bar.clone();
            if let Err(e) = self.open_source_url(&raw) {
                self.toast(e);
            }
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("↻ Refresh").clicked() {
                self.source_cache.remove(&sid);
                self.source_loading.remove(&sid);
                self.request_source(&sid, &surl);
            }
            if ui.button("⚡ Load with JS").on_hover_text("Re-render via headless chromium (for script-rendered pages)").clicked() {
                self.request_source_js(&sid, &surl);
            }
            if ui.button("↗ Browser").clicked() {
                open::that(&surl).ok();
            }
            if ui.button("⧉ Copy").clicked() {
                ctx.output_mut(|o| o.copied_text = surl.clone());
                self.toast("URL copied ✓");
            }
            if ui.button("🗑 Remove").clicked() {
                self.remove_source(&sid);
                self.toast("Source removed ✓");
                return;
            }
        });
        ui.separator();
        self.request_source(&sid, &surl);
        if self.source_loading.contains(&sid) && !self.source_cache.contains_key(&sid) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading page…");
            });
            return;
        }
        let Some(page) = self.source_cache.get(&sid).cloned() else {
            ui.label("Nothing loaded yet — hit Refresh.");
            return;
        };
        if let Some(err) = page.error {
            ui.label(egui::RichText::new(format!("⚠ Couldn't load page: {err}")).color(egui::Color32::from_rgb(220, 120, 120)));
            ui.horizontal(|ui| {
                if ui.button("↻ Retry").clicked() {
                    self.source_cache.remove(&sid);
                    self.request_source(&sid, &surl);
                }
                if ui.button("↗ Open in browser instead").clicked() {
                    open::that(&surl).ok();
                }
            });
            return;
        }
        ui.horizontal(|ui| {
            if page.js_rendered {
                ui.label(egui::RichText::new("⚡ JS-rendered").color(accent).strong());
            } else if page_looks_empty(&page) {
                ui.label(egui::RichText::new("Looks script-rendered — hit ⚡ Load with JS (needs chromium via nix-shell).").color(muted).small());
            }
        });
        // video strip: every video found on the page, playable/importable
        let vids: Vec<PageLink> = page.links.iter().filter(|l| l.is_video).cloned().collect();
        if !vids.is_empty() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("🎬 {} video(s) on this page", vids.len())).color(accent).strong());
                if ui.small_button("＋ Import all").clicked() {
                    let cat = if self.active_category != "All" && self.active_category != "Favorites" {
                        self.active_category.clone()
                    } else {
                        "General".into()
                    };
                    let mut added = 0;
                    let mut skipped = 0;
                    for l in &vids {
                        if self.videos.iter().any(|x| x.url == l.url) {
                            skipped += 1;
                            continue;
                        }
                        self.videos.push(make_video_from_url(&l.url, cat.clone()));
                        added += 1;
                    }
                    self.persist();
                    self.toast(format!("Imported {added}, skipped {skipped}."));
                }
            });
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    for l in &vids {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_width(220.0);
                            ui.label(egui::RichText::new(l.label.clone()).strong());
                            ui.horizontal(|ui| {
                                if self.videos.iter().any(|x| x.url == l.url) {
                                    ui.label(egui::RichText::new("in library ✓").color(muted).small());
                                } else if ui.small_button("＋ Import").clicked() {
                                    let cat = if self.active_category != "All" && self.active_category != "Favorites" {
                                        self.active_category.clone()
                                    } else {
                                        "General".into()
                                    };
                                    match self.import_url(&l.url, &cat, &l.label) {
                                        Ok(()) => self.toast("Added ✓"),
                                        Err(e) => self.toast(e),
                                    }
                                }
                                if ui.small_button("▶ Play").clicked() {
                                    let v = make_video_from_url(&l.url, "General".into());
                                    self.player.play(&v);
                                    self.toast(format!("Playing: {}", l.label));
                                }
                            });
                        });
                    }
                });
            });
            ui.separator();
        }
        if !page.title.is_empty() {
            ui.heading(page.title.clone());
        }
        ui.label(egui::RichText::new(format!("fetched {} · {} blocks", page.fetched_at, page.doc.blocks.len())).color(muted).small());
        ui.separator();
        // the rendered page itself
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let blocks = page.doc.blocks.clone();
            if blocks.is_empty() {
                ui.label(egui::RichText::new("This page has no readable text (it may be script-rendered). Try the browser button.").color(muted));
                return;
            }
            let mut nav_to: Option<String> = None;
            for b in blocks.iter().take(600) {
                match b {
                    DocBlock::Heading { level, text } => {
                        let size = match level {
                            1 => 24.0,
                            2 => 20.0,
                            3 => 17.0,
                            _ => 15.0,
                        };
                        ui.label(egui::RichText::new(text).size(size).strong());
                        ui.add_space(2.0);
                    }
                    DocBlock::Para(spans) | DocBlock::Quote(spans) => {
                        if matches!(b, DocBlock::Quote(_)) {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("▌").color(accent));
                                ui.vertical(|ui| {
                                    self.render_spans(ui, spans, accent, &mut nav_to);
                                });
                            });
                        } else {
                            self.render_spans(ui, spans, accent, &mut nav_to);
                        }
                        ui.add_space(4.0);
                    }
                    DocBlock::Bullet(spans) => {
                        ui.horizontal(|ui| {
                            ui.label("•");
                            ui.vertical(|ui| {
                                self.render_spans(ui, spans, accent, &mut nav_to);
                            });
                        });
                    }
                    DocBlock::Numbered(n, spans) => {
                        ui.horizontal(|ui| {
                            ui.label(format!("{n}."));
                            ui.vertical(|ui| {
                                self.render_spans(ui, spans, accent, &mut nav_to);
                            });
                        });
                    }
                    DocBlock::Image { src, alt } => {
                        self.request_source_image(src);
                        if let Some(tex) = self.source_img_cache.get(src) {
                            let size = tex.size();
                            let max_w = ui.available_width().min(640.0).max(80.0);
                            let scale = (max_w / size[0] as f32).min(1.0);
                            ui.add(egui::Image::from_texture(tex).fit_to_exact_size(egui::vec2(size[0] as f32 * scale, size[1] as f32 * scale)));
                            if !alt.is_empty() {
                                ui.label(egui::RichText::new(alt.clone()).color(muted).small().italics());
                            }
                        } else {
                            ui.label(egui::RichText::new(format!("🖼 {}", if alt.is_empty() { "loading image…" } else { alt })).color(muted).small());
                        }
                        ui.add_space(4.0);
                    }
                    DocBlock::Hr => {
                        ui.separator();
                    }
                    DocBlock::Pre(t) => {
                        ui.label(egui::RichText::new(t.clone()).monospace().small());
                        ui.add_space(4.0);
                    }
                }
            }
            if let Some(url) = nav_to {
                let _ = self.open_source_url(&url);
            }
        });
    }

    /// Inline rich-text row: bold/italic/code + clickable links (stay in-app).
    fn render_spans(&mut self, ui: &mut egui::Ui, spans: &[DocSpan], accent: egui::Color32, nav_to: &mut Option<String>) {
        ui.horizontal_wrapped(|ui| {
            for sp in spans {
                let mut rt = egui::RichText::new(sp.text.clone());
                if sp.bold {
                    rt = rt.strong();
                }
                if sp.italic {
                    rt = rt.italics();
                }
                if sp.code {
                    rt = rt.monospace();
                }
                if let Some(link) = &sp.link {
                    let is_vid = parse_youtube_id(link).is_some() || is_direct_video_url(link);
                    let label = if is_vid { format!("▶ {}", sp.text) } else { sp.text.clone() };
                    let resp = ui.link(egui::RichText::new(label).color(accent).underline());
                    if resp.clicked() {
                        if is_vid {
                            let v = make_video_from_url(link, "General".into());
                            self.player.play(&v);
                            self.toast(format!("Playing: {}", sp.text));
                        } else {
                            *nav_to = Some(link.clone());
                        }
                    }
                    resp.on_hover_text(link.clone());
                } else if sp.code {
                    ui.label(rt);
                } else {
                    ui.label(rt);
                }
            }
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
        let thumb = self.thumb_cache.get(&v.id).map(|t| (t.id(), t.size()));
        let glass = self.theme.glass;
        let screen_rounding = egui::Rounding::same(self.theme.radius.min(22) as f32);
        let screen_fill = alpha_color(
            egui::Color32::from_rgb(4, 5, 9),
            if glass { 238 } else { 255 },
        );
        let screen_stroke = egui::Stroke::new(
            1.0_f32,
            alpha_color(
                accent,
                (45.0 + self.theme.glow as f32 * 1.3).min(180.0) as u8,
            ),
        );
        let screen_shadow = theme_shadow(&self.theme);
        egui::Frame::none()
            .fill(screen_fill)
            .rounding(screen_rounding)
            .stroke(screen_stroke)
            .shadow(screen_shadow)
            .inner_margin(egui::Margin::same(12.0))
            .show(ui, |ui| {
            let bg_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), 330.0),
            );
            if let Some((tex_id, size)) = thumb {
                let iw = size[0].max(1) as f32;
                let ih = size[1].max(1) as f32;
                let scale = (bg_rect.width() / iw).max(bg_rect.height() / ih);
                let target = egui::Rect::from_center_size(
                    bg_rect.center(),
                    egui::vec2(iw * scale, ih * scale),
                );
                let painter = ui.painter().with_clip_rect(bg_rect);
                painter.image(
                    tex_id,
                    target,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha(if glass { 90 } else { 60 }),
                );
                painter.rect_filled(
                    bg_rect,
                    screen_rounding,
                    egui::Color32::from_black_alpha(if glass { 118 } else { 150 }),
                );
            }
            ui.set_min_height(330.0);
            ui.vertical_centered(|ui| {
                ui.add_space(70.0);
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
            ui.add(egui::TextEdit::singleline(&mut self.edit_cat).hint_text("e.g. Music").desired_width(160.0));
            // Quick-pick from existing categories (fills the text field; hit Save).
            let cats: Vec<String> = self.categories().into_iter().map(|(n, _)| n).collect();
            egui::ComboBox::from_id_source(format!("theater-cat-{}", v.id))
                .selected_text("Pick…")
                .show_ui(ui, |ui| {
                    for c in &cats {
                        if ui.selectable_label(self.edit_cat == *c, c.clone()).clicked() {
                            self.edit_cat = c.clone();
                        }
                    }
                });
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
            if ui.button(if v.favorite { "★ Favorited" } else { "☆ Favorite" }).clicked() {
                if let Some(slot) = self.videos.iter_mut().find(|x| x.id == v.id) {
                    slot.favorite = !slot.favorite;
                }
                self.persist();
            }
            if ui.button("🗑 Delete").clicked() {
                let title = v.title.clone();
                let vid_id = v.id.clone();
                self.delete_video(&vid_id);
                self.toast(format!("Deleted \"{title}\" ✓"));
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
            .max_height(560.0)
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
                        ui.label("Paste URLs (separated by lines, spaces or commas) or load a .txt file. # = comment.");
                        // compact 3-row box pinned to the END so huge lists stay usable
                        if self.bulk_pin_end {
                            self.bulk_pin_end = false;
                            let end = self.in_bulk.chars().count();
                            let sid = ui.make_persistent_id(egui::Id::new("bulk_urls"));
                            if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), sid) {
                                st.set_ccursor_range(Some(egui::text::CCursorRange::one(
                                    egui::text::CCursor::new(end),
                                )));
                                egui::TextEdit::store_state(ui.ctx(), sid, st);
                            }
                        }
                        // FIXED 3-row-high scroll box: egui's TextEdit always sizes
                        // to its FULL text (desired_rows is only a minimum), so the
                        // only way to cap it is an outer ScrollArea with max_height.
                        // stick_to_bottom keeps the tail (last 3 lines) in view.
                        let box_h =
                            ui.text_style_height(&egui::TextStyle::Body) * 3.0 + 12.0;
                        egui::ScrollArea::vertical()
                            .id_source("bulk_scroll")
                            .max_height(box_h)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.in_bulk)
                                        .hint_text("https://… https://…\nhttps://…")
                                        .desired_width(f32::INFINITY)
                                        .id_source("bulk_urls"),
                                );
                            });
                        ui.small(format!(
                            "{} in box · {} look like urls",
                            self.in_bulk.lines().filter(|l| !l.trim().is_empty()).count(),
                            split_url_list(&self.in_bulk).len()
                        ));
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
                                        self.bulk_pin_end = true; // jump the 3-row box to the new tail
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

    fn theme_window(&mut self, ctx: &egui::Context, _accent: egui::Color32, muted: egui::Color32) {
        let mut open = self.show_theme;
        let mut changed = false;
        let mut preset_msg: Option<String> = None;
        egui::Window::new("⚙ Settings")
            .open(&mut open)
            .resizable(true)
            .default_width(465.0)
            .max_height(650.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Appearance, "✨ Appearance");
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Glass, "🧊 Glass & motion");
                    ui.selectable_value(&mut self.settings_tab, SettingsTab::Colors, "🎨 Colors");
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(535.0)
                    .show(ui, |ui| {
                        match self.settings_tab {
                            SettingsTab::Appearance => {
                                ui.label(egui::RichText::new("Presets").strong());
                                ui.horizontal_wrapped(|ui| {
                                    for name in ["Liquid Glass", "Frosted", "Neon", "OLED", "Minimal"] {
                                        if ui.button(name).clicked() {
                                            apply_theme_preset(&mut self.theme, name);
                                            preset_msg = Some(name.to_string());
                                            changed = true;
                                        }
                                    }
                                });
                                ui.add_space(4.0);
                                ui.separator();
                                let glass_resp = ui
                                    .checkbox(&mut self.theme.glass, "Liquid glass panels")
                                    .on_hover_text("Translucent surfaces, soft shadows and accent rim lights");
                                if glass_resp.changed() {
                                    changed = true;
                                }
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.theme.opacity, 20..=100)
                                            .text("Panel opacity")
                                            .suffix("%"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                if ui
                                    .add(egui::Slider::new(&mut self.theme.radius, 0..=28).text("Corner radius"))
                                    .changed()
                                {
                                    changed = true;
                                }
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.theme.shadow, 0..=100)
                                            .text("Shadow / depth"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.theme.glow, 0..=100)
                                            .text("Accent glow"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                if ui
                                    .checkbox(&mut self.theme.hover_glow, "Accent rim on card hover")
                                    .changed()
                                {
                                    changed = true;
                                }
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.theme.card_scale, 0.6..=1.6)
                                            .text("Card size")
                                            .suffix("×"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(
                                        "Tip: turn Liquid glass on with a warm accent and the Aurora background for the signature Scwoll look.",
                                    )
                                    .color(muted)
                                    .small(),
                                );
                            }
                            SettingsTab::Glass => {
                                ui.label(egui::RichText::new("Glass & backdrop").strong());
                                if ui
                                    .add(
                                        egui::Slider::new(&mut self.theme.blur, 0..=40)
                                            .text("Frost / blur"),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.label(
                                    egui::RichText::new(
                                        "egui cannot sample the desktop behind the window, so blur is a soft translucent simulation: higher values widen the frosted tint and background light.",
                                    )
                                    .color(muted)
                                    .small(),
                                );
                                ui.add_space(4.0);
                                ui.separator();
                                ui.label(egui::RichText::new("Aurora background").strong());
                                if ui
                                    .checkbox(&mut self.theme.aurora, "Coloured ambient blobs behind the UI")
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.add_enabled_ui(self.theme.aurora, |ui| {
                                    if ui
                                        .checkbox(&mut self.theme.animate, "Animate the aurora")
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                    if ui
                                        .add(
                                            egui::Slider::new(&mut self.theme.ambient, 0..=100)
                                                .text("Aurora intensity"),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                });
                                ui.add_space(4.0);
                                ui.separator();
                                ui.label(egui::RichText::new("Atmosphere").strong());
                                if ui
                                    .add(egui::Slider::new(&mut self.theme.grain, 0..=100).text("Film grain"))
                                    .changed()
                                {
                                    changed = true;
                                }
                                if ui
                                    .add(egui::Slider::new(&mut self.theme.vignette, 0..=100).text("Vignette"))
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(
                                        "Motion can be disabled independently of the aurora; the UI stays static when both are off.",
                                    )
                                    .color(muted)
                                    .small(),
                                );
                            }
                            SettingsTab::Colors => {
                                ui.label(egui::RichText::new("Palette").strong());
                                let fields: [(&str, &str); 8] = [
                                    ("accent", "Accent"),
                                    ("accent2", "Accent 2 / aurora"),
                                    ("background", "Background"),
                                    ("surface", "Cards / windows"),
                                    ("surface2", "Inputs"),
                                    ("text", "Text"),
                                    ("muted", "Muted text"),
                                    ("sidebar", "Sidebar"),
                                ];
                                for (key, label) in fields {
                                    ui.horizontal(|ui| {
                                        ui.label(format!("{label}:"));
                                        let hex = match key {
                                            "accent" => &mut self.theme.accent,
                                            "accent2" => &mut self.theme.accent2,
                                            "background" => &mut self.theme.background,
                                            "surface" => &mut self.theme.surface,
                                            "surface2" => &mut self.theme.surface2,
                                            "text" => &mut self.theme.text,
                                            "muted" => &mut self.theme.muted,
                                            "sidebar" => &mut self.theme.sidebar,
                                            _ => &mut self.theme.accent,
                                        };
                                        let col = hex_to_color(hex);
                                        let mut rgb = [col.r(), col.g(), col.b()];
                                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                                            *hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
                                            changed = true;
                                        }
                                        ui.monospace(hex.clone());
                                    });
                                }
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    if ui.button("⇄ Swap accents").clicked() {
                                        std::mem::swap(&mut self.theme.accent, &mut self.theme.accent2);
                                        changed = true;
                                    }
                                });
                                ui.small("Accent 2 is used by the background aurora and gradient panel edges.");
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset defaults").clicked() {
                        self.theme = Theme::default();
                        changed = true;
                        preset_msg = Some("Defaults restored".into());
                    }
                    if ui.button("💾 Save").clicked() {
                        save_theme(&self.theme);
                        preset_msg = Some("Settings saved".into());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new("auto-saved to data/theme.json").color(muted).small());
                    });
                });
            });
        self.show_theme = open;
        if changed {
            save_theme(&self.theme);
        }
        if let Some(msg) = preset_msg {
            self.toast(msg);
        }
    }

    fn categories_window(&mut self, ctx: &egui::Context, muted: egui::Color32) {
        let mut open = self.show_cats;
        let mut toast_msg: Option<String> = None;
        let mut rename_job: Option<(String, String)> = None;
        let mut clear_job: Option<String> = None;
        egui::Window::new("⚙ Manage categories")
            .open(&mut open)
            .resizable(true)
            .default_width(440.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Rename a category (moves all its videos), or clear one (moves its videos to General). Per-video moves: card ▾ menu or theater view.")
                        .color(muted)
                        .small(),
                );
                ui.separator();
                let cats = self.categories();
                if cats.is_empty() {
                    ui.label("No categories yet — import something!");
                }
                for (name, count) in &cats {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{name} ({count})")).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("🗑 Clear").on_hover_text("Move these videos to General").clicked() {
                                clear_job = Some(name.clone());
                            }
                            if ui.small_button("✎ Rename").clicked() {
                                let buf = self.cat_edit_buf.get(name).cloned().unwrap_or_else(|| name.clone());
                                rename_job = Some((name.clone(), buf));
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("New name:");
                        let buf = self.cat_edit_buf.entry(name.clone()).or_insert_with(|| name.clone());
                        ui.add(egui::TextEdit::singleline(buf).hint_text("e.g. Music").desired_width(200.0));
                    });
                    ui.separator();
                }
            });
        self.show_cats = open;
        if let Some((old, new)) = rename_job {
            match self.rename_category(&old, &new) {
                Ok(n) => toast_msg = Some(format!("Renamed \"{old}\" → \"{new}\" ({n} videos) ✓")),
                Err(e) => toast_msg = Some(e),
            }
        }
        if let Some(cat) = clear_job {
            let n = self.clear_category(&cat);
            toast_msg = Some(format!("Cleared \"{cat}\" — {n} videos → General ✓"));
        }
        if let Some(m) = toast_msg {
            self.toast(m);
        }
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
        assert_eq!((img.width(), img.height()), (360, 240));
        // decode must preserve the full mosaic (no 16:9 crop eating rows)
        let decoded = decode_thumb_bytes(&bytes).expect("no decode");
        assert_eq!(decoded.size, [360, 240]);
        // the 6 cells must actually differ (frames spread across time)
        let px = |cx: usize, cy: usize| {
            let c = decoded.pixels[cy * 360 + cx];
            (c.r(), c.g(), c.b())
        };
        let mut cells = std::collections::HashSet::new();
        for c in 0..6 {
            cells.insert(px((c % 3) * 120 + 60, (c / 3) * 120 + 60));
        }
        assert!(cells.len() >= 5, "cells not spread across video: {cells:?}");
        // endpoints: first cell from the first second (red), last cell from
        // the last second (cyan) — in the right order, not clustered.
        let first = px(60, 60);
        let last = px(2 * 120 + 60, 120 + 60);
        assert!(first.0 > first.1 && first.0 > first.2, "first cell not red: {first:?}");
        assert!(last.1 > last.0 && last.2 > last.0, "last cell not cyan: {last:?}");
        std::fs::remove_file(&vid).ok();
    }
}

#[cfg(test)]
mod source_js_tests {
    use super::*;

    fn spa_shell() -> String {
        // padding lives inside <script> (skipped text) so the shell stays big
        // but yields (almost) no readable content — like a real SPA shell.
        format!(
            "<html><head><title>App</title><script src=\"/static/bundle.js\">var p=\"{}\";</script></head>\
             <body><div id=\"root\"></div><noscript>need js</noscript></body></html>",
            "x".repeat(3000)
        )
    }

    #[test]
    fn detects_spa_shell() {
        let html = spa_shell();
        let doc = parse_html_doc("https://example.com/", &html);
        assert!(doc_text_len(&doc) < 400);
        assert!(looks_like_spa(&html, &doc));
    }

    #[test]
    fn plain_article_is_not_spa() {
        let body = "lorem ipsum ".repeat(80); // ~960 chars of text
        let html = format!("<html><head><title>T</title></head><body><p>{body}</p></body></html>");
        let doc = parse_html_doc("https://example.com/", &html);
        assert!(doc_text_len(&doc) >= 400);
        assert!(!looks_like_spa(&html, &doc));
    }

    #[test]
    fn tiny_pages_never_spa() {
        let html = "<html><body><div id=\"root\"></div></body></html>";
        let doc = parse_html_doc("https://example.com/", html);
        assert!(!looks_like_spa(html, &doc));
    }

    #[test]
    fn empty_page_hint() {
        let page = SourcePage {
            title: String::new(),
            links: vec![],
            doc: HtmlDoc::default(),
            fetched_at: now_iso(),
            error: None,
            js_rendered: false,
        };
        assert!(page_looks_empty(&page));
    }
}
