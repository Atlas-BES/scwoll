// ---------------------------------------------------------- mobile (M3) --
//! Material-You-flavoured phone layout, used on Android.
//!
//! The desktop UI is a glass/multi-pane layout that needs a wide window; on a
//! phone we render a Material 3 style single-column app instead: a small top
//! app bar, category chips, a vertical feed of media cards, a bottom
//! navigation bar and full-screen dialogs for Settings / Import. Playback
//! hands off to the system video player (see `android.rs`).

use super::*;

/// Material 3 colour roles, derived from the app theme's accent + surfaces.
#[derive(Clone, Copy)]
struct M3 {
    surface: egui::Color32,
    surface_container: egui::Color32,
    surface_container_high: egui::Color32,
    on_surface: egui::Color32,
    on_surface_variant: egui::Color32,
    primary: egui::Color32,
    on_primary: egui::Color32,
    primary_container: egui::Color32,
    on_primary_container: egui::Color32,
    outline: egui::Color32,
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t) as u8;
    egui::Color32::from_rgb(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
    )
}

impl M3 {
    fn from_theme(t: &Theme) -> Self {
        let accent = hex_to_color(&t.accent);
        let mut surface = mix(hex_to_color(&t.background), egui::Color32::from_rgb(8, 9, 12), 0.8);
        // Material surfaces are near-black in dark mode but never pure black.
        let lum = (surface.r() as u32 + surface.g() as u32 + surface.b() as u32) / 3;
        if lum < 10 {
            surface = egui::Color32::from_rgb(16, 17, 20);
        }
        let container = mix(surface, egui::Color32::WHITE, 0.05);
        let container_high = mix(surface, egui::Color32::WHITE, 0.09);
        let on_surface = mix(hex_to_color(&t.text), egui::Color32::WHITE, 0.35);
        let on_surface_variant = mix(hex_to_color(&t.muted), egui::Color32::WHITE, 0.1);
        // Light accents get dark text on them (Material's contrast rule).
        let accent_lum = (accent.r() as u32 * 299 + accent.g() as u32 * 587 + accent.b() as u32 * 114)
            / 1000;
        let on_primary = if accent_lum > 150 {
            egui::Color32::from_rgb(12, 12, 14)
        } else {
            egui::Color32::WHITE
        };
        let primary_container = mix(accent, surface, 0.72);
        let on_primary_container = mix(accent, egui::Color32::WHITE, 0.65);
        Self {
            surface,
            surface_container: container,
            surface_container_high: container_high,
            on_surface,
            on_surface_variant,
            primary: accent,
            on_primary,
            primary_container,
            on_primary_container,
            outline: mix(on_surface_variant, surface, 0.55),
        }
    }

    fn apply(&self, ctx: &egui::Context) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 10.0);
        style.spacing.interact_size.y = 40.0;
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(21.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(14.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        let v = &mut style.visuals;
        v.override_text_color = Some(self.on_surface);
        v.panel_fill = self.surface;
        v.window_fill = self.surface_container;
        v.extreme_bg_color = mix(self.surface, egui::Color32::BLACK, 0.25);
        v.selection.bg_fill = self.primary_container;
        v.selection.stroke = egui::Stroke::new(1.0_f32, self.primary);
        v.hyperlink_color = self.primary;
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.rounding = egui::Rounding::same(14.0);
            w.bg_stroke = egui::Stroke::NONE;
        }
        v.widgets.inactive.weak_bg_fill = self.surface_container_high;
        v.widgets.hovered.weak_bg_fill = mix(self.surface_container_high, self.primary, 0.18);
        v.widgets.active.weak_bg_fill = mix(self.surface_container_high, self.primary, 0.3);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, self.outline);
        v.window_rounding = egui::Rounding::same(24.0);
        ctx.set_style(style);
    }
}

/// What a media card wants the app to do.
#[derive(Default)]
struct M3Action {
    open: bool,
    play: bool,
    favorite: bool,
}

impl ScwollApp {
    pub(crate) fn mobile_ui(&mut self, ctx: &egui::Context) {
        let c = M3::from_theme(&self.theme);
        c.apply(ctx);

        // Background (flat, no glass): Material surfaces are opaque.
        let screen = ctx.screen_rect();
        ctx.layer_painter(egui::LayerId::background())
            .rect_filled(screen, 0.0, c.surface);

        self.pump_thumbs(ctx);
        self.pump_sources();
        self.pump_source_images(ctx);
        let animated = self.theme.animate && self.theme.aurora && self.theme.ambient > 0;
        ctx.request_repaint_after(Duration::from_millis(if animated { 33 } else { 500 }));

        // Stale source selection (deleted elsewhere)? drop it.
        if let Some(id) = self.active_source_id.clone() {
            if !self.sources.iter().any(|s| s.id == id) {
                self.active_source_id = None;
            }
        }

        // ---------------- system bar insets ----------------
        // The status/navigation bars stay visible, so pad the app bar and the
        // bottom navigation by them (re-read about once a second: they change
        // on rotation, or when the bars hide/show).
        let stale = self
            .m3_insets_at
            .map(|t| t.elapsed() > Duration::from_secs(1))
            .unwrap_or(true);
        if stale {
            self.m3_insets = android::system_bar_insets_px();
            self.m3_insets_at = Some(Instant::now());
        }
        let ppp = ctx.pixels_per_point().max(1.0);
        let inset_top = self.m3_insets.0 / ppp;
        let inset_bottom = self.m3_insets.1 / ppp;

        // ---------------- top app bar ----------------
        egui::TopBottomPanel::top("m3_appbar")
            .frame(
                egui::Frame::none()
                    .fill(c.surface)
                    .inner_margin(egui::Margin {
                        left: 14.0,
                        right: 14.0,
                        top: 10.0 + inset_top,
                        bottom: 10.0,
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::from_texture(&self.logo_tex)
                            .fit_to_exact_size(egui::vec2(26.0, 26.0))
                            .rounding(egui::Rounding::same(8.0)),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new("Scwoll")
                            .size(19.0)
                            .strong()
                            .color(c.on_surface),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if m3_icon_button(ui, "⚙", &c).clicked() {
                            self.m3_settings_open = true;
                        }
                        if m3_icon_button(ui, "📥", &c).clicked() {
                            self.m3_import_open = true;
                        }
                        let search_on = self.m3_search_open;
                        if m3_icon_button(ui, if search_on { "✖" } else { "🔍" }, &c).clicked() {
                            self.m3_search_open = !search_on;
                            if !self.m3_search_open {
                                self.search.clear();
                            }
                        }
                    });
                });
                if self.m3_search_open {
                    ui.add_space(6.0);
                    let w = ui.available_width();
                    m3_field(ui, &mut self.search, "Search videos, categories, urls…", w, &c);
                }
            });

        // ---------------- bottom navigation ----------------
        egui::TopBottomPanel::bottom("m3_nav")
            .frame(egui::Frame::none().fill(c.surface_container).inner_margin(
                egui::Margin {
                    left: 6.0,
                    right: 6.0,
                    top: 6.0,
                    bottom: 6.0 + inset_bottom,
                },
            ))
            .show(ctx, |ui| {
                let mut go_all = false;
                let mut go_yt = false;
                let mut go_local = false;
                let mut go_sources = false;
                ui.columns(4, |cols| {
                    let on_lib = !self.showing_sources && self.active_source_id.is_none();
                    go_all = m3_nav_item(
                        &mut cols[0],
                        "🏠",
                        "Home",
                        on_lib && self.source == SourceFilter::All,
                        &c,
                    );
                    go_yt = m3_nav_item(
                        &mut cols[1],
                        "▶",
                        "YouTube",
                        on_lib && self.source == SourceFilter::Youtube,
                        &c,
                    );
                    go_local = m3_nav_item(
                        &mut cols[2],
                        "🗂",
                        "Local",
                        on_lib && self.source == SourceFilter::Local,
                        &c,
                    );
                    go_sources = m3_nav_item(
                        &mut cols[3],
                        "🌐",
                        "Sources",
                        self.showing_sources || self.active_source_id.is_some(),
                        &c,
                    );
                });
                if go_all {
                    self.source = SourceFilter::All;
                    self.reset_destination();
                }
                if go_yt {
                    self.source = SourceFilter::Youtube;
                    self.reset_destination();
                }
                if go_local {
                    self.source = SourceFilter::Local;
                    self.reset_destination();
                }
                if go_sources {
                    self.open_sources_page();
                }
            });

        // ---------------- content ----------------
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(c.surface)
                    .inner_margin(egui::Margin::symmetric(14.0, 6.0)),
            )
            .show(ctx, |ui| {
                if self.showing_sources {
                    self.show_sources_list(ui, c.primary, c.on_surface_variant);
                } else if self.active_source_id.is_some() {
                    self.show_source_doc(ui, ctx, c.primary, c.on_surface_variant);
                } else if let Some(id) = self.selected.clone() {
                    self.m3_detail(ui, &id, &c);
                } else {
                    self.m3_feed(ui, &c);
                }
            });

        // ---------------- dialogs ----------------
        self.m3_dialogs(ctx, &c);

        // ---------------- toast ----------------
        if let Some((msg, t)) = self.toast.clone() {
            if t.elapsed() < Duration::from_secs(3) {
                let painter = ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("m3_toast"),
                ));
                let rect = egui::Rect::from_center_size(
                    egui::pos2(screen.center().x, screen.bottom() - 96.0),
                    egui::vec2((screen.width() - 60.0).min(360.0), 44.0),
                );
                painter.rect_filled(rect, egui::Rounding::same(22.0), c.surface_container_high);
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &msg,
                    egui::FontId::proportional(14.0),
                    c.on_surface,
                );
                ctx.request_repaint_after(Duration::from_millis(200));
            } else {
                self.toast = None;
            }
        }
    }

    fn reset_destination(&mut self) {
        self.selected = None;
        self.active_source_id = None;
        self.showing_sources = false;
    }

    // ------------------------------------------------------------ feed ----
    fn m3_feed(&mut self, ui: &mut egui::Ui, c: &M3) {
        // Category chips (Material "filter chips", horizontally scrollable).
        egui::ScrollArea::horizontal()
            .id_source("m3_chips")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let total = self.videos.len();
                    let favs = self.videos.iter().filter(|v| v.favorite).count();
                    let mut pick: Option<String> = None;
                    if m3_chip(ui, &format!("All ({total})"), self.active_category == "All", c) {
                        pick = Some("All".into());
                    }
                    if m3_chip(ui, &format!("★ {favs}"), self.active_category == "Favorites", c) {
                        pick = Some("Favorites".into());
                    }
                    for (name, count) in self.categories() {
                        if m3_chip(
                            ui,
                            &format!("{name} ({count})"),
                            self.active_category == name,
                            c,
                        ) {
                            pick = Some(name);
                        }
                    }
                    if let Some(cat) = pick {
                        self.active_category = cat;
                        self.selected = None;
                    }
                });
            });
        ui.add_space(6.0);

        let list: Vec<Video> = self.filtered().into_iter().cloned().collect();
        if list.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(60.0);
                ui.label(
                    egui::RichText::new(if self.active_category == "Favorites" {
                        "☆ No favorites yet"
                    } else {
                        "📼 Nothing here yet"
                    })
                    .size(18.0)
                    .color(c.on_surface),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Tap 📥 to add a URL or a list of links.")
                        .color(c.on_surface_variant),
                );
                ui.add_space(14.0);
                if m3_filled_button(ui, "📥 Import videos", c).clicked() {
                    self.m3_import_open = true;
                }
            });
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for v in &list {
                    let act = self.m3_card(ui, v, c);
                    if act.play {
                        self.player.play(v);
                        self.toast("▶ Opening in the system player…");
                    }
                    if act.favorite {
                        if let Some(slot) = self.videos.iter_mut().find(|x| x.id == v.id) {
                            slot.favorite = !slot.favorite;
                        }
                        self.persist();
                    }
                    if act.open {
                        self.selected = Some(v.id.clone());
                        self.edit_title = v.title.clone();
                        self.edit_cat = v.category.clone();
                    }
                    ui.add_space(10.0);
                }
                ui.add_space(12.0);
            });
    }

    fn m3_card(&mut self, ui: &mut egui::Ui, v: &Video, c: &M3) -> M3Action {
        let mut act = M3Action::default();
        let width = ui.available_width();
        egui::Frame::none()
            .fill(c.surface_container)
            .rounding(egui::Rounding::same(20.0))
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                ui.set_width(width - 20.0);
                let thumb_h = ((width - 20.0) * 9.0 / 16.0).clamp(140.0, 240.0);
                let (rect, thumb_resp) = ui.allocate_exact_size(
                    egui::vec2(width - 20.0, thumb_h),
                    egui::Sense::click(),
                );
                self.paint_thumb(ui, v, rect, c);
                if thumb_resp.clicked() {
                    act.open = true;
                }

                // Play overlay (Material "filled tonal" circle).
                let play_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.right() - 30.0, rect.bottom() - 30.0),
                    egui::vec2(44.0, 44.0),
                );
                let play_resp = ui.interact(
                    play_rect,
                    ui.id().with(("m3_play", &v.id)),
                    egui::Sense::click(),
                );
                ui.painter()
                    .circle_filled(play_rect.center(), 22.0, c.primary_container);
                ui.painter().text(
                    play_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "▶",
                    egui::FontId::proportional(17.0),
                    c.on_primary_container,
                );
                if play_resp.clicked() {
                    act.play = true;
                }

                if v.favorite {
                    let fav_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.left() + 22.0, rect.top() + 22.0),
                        egui::vec2(34.0, 34.0),
                    );
                    ui.painter()
                        .circle_filled(fav_rect.center(), 17.0, c.surface_container_high);
                    ui.painter().text(
                        fav_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "★",
                        egui::FontId::proportional(15.0),
                        c.primary,
                    );
                }

                ui.add_space(8.0);
                let title_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&v.title)
                            .size(15.0)
                            .strong()
                            .color(c.on_surface),
                    )
                    .wrap()
                    .sense(egui::Sense::click()),
                );
                if title_resp.clicked() {
                    act.open = true;
                }
                let meta = format!(
                    "{} · {}",
                    match v.kind {
                        VideoKind::Youtube => "YouTube",
                        VideoKind::Local => "Local file",
                        VideoKind::Direct => "Direct link",
                    },
                    v.category
                );
                ui.label(
                    egui::RichText::new(meta)
                        .size(12.5)
                        .color(c.on_surface_variant),
                );
            });
        act
    }

    fn paint_thumb(&mut self, ui: &egui::Ui, v: &Video, rect: egui::Rect, c: &M3) {
        let rounding = egui::Rounding::same(14.0);
        ui.painter().rect_filled(rect, rounding, c.surface_container_high);
        if let Some(tex) = self.thumb_cache.get(&v.id) {
            let size = tex.size_vec2();
            let scale = (rect.width() / size.x).max(rect.height() / size.y);
            let target = egui::Rect::from_center_size(rect.center(), size * scale);
            ui.painter().with_clip_rect(rect).image(
                tex.id(),
                target,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            // Scrim at the bottom so the play chip stays readable.
            let scrim = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.bottom() - 56.0),
                rect.max,
            );
            ui.painter().with_clip_rect(rect).rect_filled(
                scrim,
                egui::Rounding::ZERO,
                egui::Color32::from_black_alpha(70),
            );
        } else {
            self.request_thumb(v);
            let letter = v
                .title
                .chars()
                .next()
                .map(|ch| ch.to_uppercase().to_string())
                .unwrap_or_else(|| "▶".into());
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                letter,
                egui::FontId::proportional(46.0),
                c.outline,
            );
        }
    }

    // ---------------------------------------------------------- detail ----
    fn m3_detail(&mut self, ui: &mut egui::Ui, id: &str, c: &M3) {
        let Some(v) = self.by_id(id) else {
            self.selected = None;
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if m3_text_button(ui, "← Back", c).clicked() {
                    self.selected = None;
                }
                ui.add_space(4.0);

                let w = ui.available_width();
                let h = (w * 9.0 / 16.0).clamp(150.0, 260.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
                self.paint_thumb(ui, &v, rect, c);

                ui.add_space(12.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&v.title)
                            .size(19.0)
                            .strong()
                            .color(c.on_surface),
                    )
                    .wrap(),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {}",
                        match v.kind {
                            VideoKind::Youtube => "YouTube",
                            VideoKind::Local => "Local file",
                            VideoKind::Direct => "Direct link",
                        },
                        v.category
                    ))
                    .size(13.0)
                    .color(c.on_surface_variant),
                );

                ui.add_space(10.0);
                let playing = self.player.is_playing(&v.id);
                let btn_w = ui.available_width();
                if m3_filled_button_sized(
                    ui,
                    if playing { "⏸  Stop" } else { "▶  Play in system player" },
                    btn_w,
                    48.0,
                    c,
                )
                .clicked()
                {
                    if playing {
                        self.player.stop();
                        self.toast("Stopped");
                    } else {
                        self.player.play(&v);
                        self.toast("▶ Opening in the system player…");
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let half = (ui.available_width() - 8.0) / 2.0;
                    if m3_tonal_button_sized(
                        ui,
                        if v.favorite { "★  Favorited" } else { "☆  Favorite" },
                        half,
                        44.0,
                        c,
                    )
                    .clicked()
                    {
                        if let Some(slot) = self.videos.iter_mut().find(|x| x.id == v.id) {
                            slot.favorite = !slot.favorite;
                        }
                        self.persist();
                    }
                    if m3_tonal_button_sized(ui, "🗑  Delete", half, 44.0, c).clicked() {
                        let title = v.title.clone();
                        let vid_id = v.id.clone();
                        self.delete_video(&vid_id);
                        self.toast(format!("Deleted \"{title}\""));
                        self.selected = None;
                        return;
                    }
                });

                if !self.player.status.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(self.player.status.clone())
                            .size(12.0)
                            .color(c.on_surface_variant),
                    );
                }

                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("Rename & category")
                        .size(14.0)
                        .strong()
                        .color(c.on_surface),
                );
                ui.add_space(4.0);
                let fw = ui.available_width();
                m3_field(ui, &mut self.edit_title, "Title", fw, c);
                ui.horizontal(|ui| {
                    let one = (ui.available_width() - 8.0) / 2.0;
                    m3_field(ui, &mut self.edit_cat, "Category", one, c);
                    let cats: Vec<String> =
                        self.categories().into_iter().map(|(n, _)| n).collect();
                    egui::ComboBox::from_id_source(format!("m3-cat-{}", v.id))
                        .selected_text("Pick…")
                        .width(one)
                        .show_ui(ui, |ui| {
                            for name in &cats {
                                if ui
                                    .selectable_label(self.edit_cat == *name, name.clone())
                                    .clicked()
                                {
                                    self.edit_cat = name.clone();
                                }
                            }
                        });
                });
                if m3_tonal_button(ui, "💾  Save", c).clicked() {
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

                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("Source")
                        .size(12.0)
                        .strong()
                        .color(c.on_surface_variant),
                );
                let source = if v.kind == VideoKind::Local {
                    v.file_path.clone().unwrap_or_else(|| v.url.clone())
                } else {
                    v.url.clone()
                };
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(source.clone())
                            .size(12.0)
                            .color(c.on_surface_variant),
                    )
                    .wrap(),
                );
                ui.horizontal(|ui| {
                    if m3_text_button(ui, "⧉ Copy", c).clicked() {
                        ui.ctx().output_mut(|o| o.copied_text = source.clone());
                        self.toast("URL copied ✓");
                    }
                    if m3_text_button(ui, "↗ Open externally", c).clicked() {
                        let _ = open_external(&source);
                    }
                });

                // Up next
                let rest: Vec<Video> = self
                    .videos
                    .iter()
                    .filter(|x| x.id != v.id)
                    .take(8)
                    .cloned()
                    .collect();
                if !rest.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Up next")
                            .size(16.0)
                            .strong()
                            .color(c.on_surface),
                    );
                    ui.add_space(4.0);
                    for x in rest {
                        let mut open = false;
                        egui::Frame::none()
                            .fill(c.surface_container)
                            .rounding(egui::Rounding::same(16.0))
                            .inner_margin(egui::Margin::same(8.0))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (r, resp) = ui.allocate_exact_size(
                                        egui::vec2(96.0, 56.0),
                                        egui::Sense::click(),
                                    );
                                    self.paint_thumb(ui, &x, r, c);
                                    ui.vertical(|ui| {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&x.title)
                                                    .size(13.5)
                                                    .strong()
                                                    .color(c.on_surface),
                                            )
                                            .wrap(),
                                        );
                                        ui.label(
                                            egui::RichText::new(x.category.clone())
                                                .size(11.5)
                                                .color(c.on_surface_variant),
                                        );
                                    });
                                    if resp.clicked() {
                                        open = true;
                                    }
                                });
                            });
                        if open {
                            self.selected = Some(x.id.clone());
                            self.edit_title = x.title.clone();
                            self.edit_cat = x.category.clone();
                        }
                        ui.add_space(6.0);
                    }
                }
                ui.add_space(16.0);
            });
    }

    // --------------------------------------------------------- dialogs ----
    fn m3_dialogs(&mut self, ctx: &egui::Context, c: &M3) {
        // Settings
        if self.m3_settings_open {
            let muted = c.on_surface_variant;
            let mut open = true;
            m3_sheet(ctx, c, "Settings", "m3_settings_sheet", &mut open, |ui| {
                self.settings_dropdown_contents(ui, muted);
            });
            self.m3_settings_open = open;
        }

        // Import
        if self.m3_import_open {
            let mut open = true;
            m3_sheet(ctx, c, "Import", "m3_import_sheet", &mut open, |ui| {
                self.import_dropdown_contents(ui);
            });
            self.m3_import_open = open;
        }

        // Category manager
        if self.show_cats {
            self.categories_window(ctx, c.on_surface_variant);
        }
    }
}

// ------------------------------------------------------------ M3 widgets --
/// Full-screen Material "dialog": its own header with a big close button and a
/// scrollable body. Sets `*open = false` when the user closes it.
fn m3_sheet(
    ctx: &egui::Context,
    c: &M3,
    title: &str,
    id: &str,
    open: &mut bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    let screen = ctx.screen_rect();
    let mut close = false;
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.set_clip_rect(screen);
            ui.painter().rect_filled(screen, 0.0, c.surface);
            ui.allocate_ui_with_layout(
                screen.size(),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        if m3_icon_button(ui, "✖", c).clicked() {
                            close = true;
                        }
                        ui.label(
                            egui::RichText::new(title)
                                .size(19.0)
                                .strong()
                                .color(c.on_surface),
                        );
                    });
                    ui.add_space(6.0);
                    let body = egui::Rect::from_min_max(
                        egui::pos2(screen.left() + 14.0, ui.cursor().top()),
                        egui::pos2(screen.right() - 14.0, screen.bottom() - 8.0),
                    );
                    ui.allocate_ui_at_rect(body, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| add(ui));
                    });
                },
            );
        });
    if close {
        *open = false;
    }
}

fn m3_icon_button(ui: &mut egui::Ui, glyph: &str, c: &M3) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
    let hovered = resp.hovered();
    if hovered {
        ui.painter()
            .circle_filled(rect.center(), 20.0, c.surface_container_high);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(17.0),
        c.on_surface,
    );
    resp
}

fn m3_text_button(ui: &mut egui::Ui, label: &str, c: &M3) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(14.0),
        c.on_surface,
    );
    let size = egui::vec2(galley.size().x + 20.0, 38.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(19.0), c.surface_container_high);
    }
    ui.painter().galley(
        egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        c.on_surface,
    );
    resp
}

fn m3_chip(ui: &mut egui::Ui, label: &str, selected: bool, c: &M3) -> bool {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(13.0),
        if selected { c.on_primary_container } else { c.on_surface_variant },
    );
    let size = egui::vec2(galley.size().x + 26.0, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if selected {
        c.primary_container
    } else if resp.hovered() {
        c.surface_container_high
    } else {
        c.surface_container
    };
    ui.painter().rect_filled(rect, egui::Rounding::same(17.0), fill);
    if !selected {
        ui.painter().rect_stroke(
            rect,
            egui::Rounding::same(17.0),
            egui::Stroke::new(1.0_f32, c.outline),
        );
    }
    ui.painter().galley(
        egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        if selected { c.on_primary_container } else { c.on_surface_variant },
    );
    resp.clicked()
}

fn m3_filled_button(ui: &mut egui::Ui, label: &str, c: &M3) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(15.0),
        c.on_primary,
    );
    let size = egui::vec2(galley.size().x + 32.0, 46.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if resp.hovered() { mix(c.primary, egui::Color32::WHITE, 0.12) } else { c.primary };
    ui.painter().rect_filled(rect, egui::Rounding::same(23.0), fill);
    ui.painter().galley(
        egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        c.on_primary,
    );
    resp
}

fn m3_filled_button_sized(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    height: f32,
    c: &M3,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let fill = if resp.hovered() { mix(c.primary, egui::Color32::WHITE, 0.12) } else { c.primary };
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(height / 2.0), fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(15.5),
        c.on_primary,
    );
    resp
}

fn m3_tonal_button(ui: &mut egui::Ui, label: &str, c: &M3) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(14.5),
        c.on_primary_container,
    );
    let size = egui::vec2(galley.size().x + 28.0, 42.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if resp.hovered() {
        mix(c.primary_container, c.primary, 0.2)
    } else {
        c.primary_container
    };
    ui.painter().rect_filled(rect, egui::Rounding::same(21.0), fill);
    ui.painter().galley(
        egui::pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        c.on_primary_container,
    );
    resp
}

fn m3_tonal_button_sized(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    height: f32,
    c: &M3,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let fill = if resp.hovered() {
        mix(c.primary_container, c.primary, 0.2)
    } else {
        c.primary_container
    };
    ui.painter()
        .rect_filled(rect, egui::Rounding::same(height / 2.0), fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(14.0),
        c.on_primary_container,
    );
    resp
}

fn m3_nav_item(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool, c: &M3) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 58.0), egui::Sense::click());
    if selected {
        let pill = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.top() + 18.0),
            egui::vec2(56.0, 30.0),
        );
        ui.painter()
            .rect_filled(pill, egui::Rounding::same(15.0), c.primary_container);
    }
    let color = if selected { c.on_primary_container } else { c.on_surface_variant };
    ui.painter().text(
        egui::pos2(rect.center().x, rect.top() + 18.0),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        color,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 11.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        color,
    );
    resp.clicked()
}

/// Material "filled text field": a rounded surface box with no visible border.
fn m3_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32, c: &M3) {
    ui.scope(|ui| {
        ui.visuals_mut().extreme_bg_color = c.surface_container_high;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = c.surface_container_high;
        ui.visuals_mut().widgets.hovered.weak_bg_fill = c.surface_container_high;
        ui.visuals_mut().widgets.active.weak_bg_fill = c.surface_container_high;
        ui.visuals_mut().selection.bg_fill = c.primary_container;
        ui.add(
            egui::TextEdit::singleline(text)
                .hint_text(egui::RichText::new(hint).color(c.on_surface_variant))
                .desired_width(width)
                .margin(egui::Margin::symmetric(12.0, 10.0)),
        );
    });
}

/// Platform "open this URL/file with the default handler".
fn open_external(target: &str) -> Result<(), String> {
    android::open_url(target)
}
