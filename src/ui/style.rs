//! Theme, contrast, focus ring and reduced-motion handling.

use egui::{Color32, CornerRadius, Stroke, Theme};

use crate::store::{Config, MotionChoice, ThemeChoice};

/// Focus ring colours with ≥ 3:1 contrast against the panel backgrounds.
const FOCUS_DARK: Color32 = Color32::from_rgb(0xFF, 0xD5, 0x4F);
const FOCUS_LIGHT: Color32 = Color32::from_rgb(0x00, 0x4F, 0xC4);

/// egui's black-and-white emoji fonts, used only when the system has no
/// (colour) emoji font with the character.
const MONO_EMOJI: [&str; 2] = ["NotoEmoji-Regular", "emoji-icon-font"];

/// Noto Sans for text (regular weight reads better than the default light face),
/// DejaVu Sans as fallback for symbols such as ⚠ ✓ ▶ ↻ ⠿. Emoji and scripts
/// these lack (😀 🇳🇴 日本語) come from the system's fonts, in colour where the
/// font has colour (Noto Color Emoji, Segoe UI Emoji), else from egui's
/// black-and-white emoji, so playlist and song names show what they say.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // Taken out of the families so the system's colour emoji win over them.
    let mono: Vec<(String, egui::FontData)> =
        MONO_EMOJI.iter().filter_map(|n| fonts.font_data.remove(*n).map(|d| ((*n).to_owned(), (*d).clone()))).collect();
    for list in fonts.families.values_mut() {
        list.retain(|f| !MONO_EMOJI.contains(&f.as_str()));
    }
    fonts.font_data.insert(
        "noto-sans".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../assets/fonts/NotoSans-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "dejavu-sans".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../../assets/fonts/DejaVuSans.ttf"))),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let list = fonts.families.entry(family.clone()).or_default();
        if family == egui::FontFamily::Proportional {
            list.insert(0, "noto-sans".into());
        }
        list.push("dejavu-sans".into());
    }
    ctx.set_fonts(fonts);
    // Providers are asked in order: the system first, then the bundled emoji.
    let system = std::sync::Arc::new(egui_system_font_provider::SystemFontProvider::new());
    ctx.add_font_provider(system.clone());
    ctx.add_font_provider(std::sync::Arc::new(MonoEmoji(mono)));
    ctx.add_glyph_rasterizer(emoji_style(system));
}

/// Fonts holding only the system's colour emoji font, for [`emoji_style`].
type EmojiFonts = std::sync::Mutex<epaint::text::Fonts>;

/// Draws clusters that should look like emoji (😀, ❤️ with U+FE0F, 👍🏽) with
/// the system's colour emoji font. Noto Sans and DejaVu have black-and-white
/// forms of some (❤ ☕ ☀ 😀 😊), which would otherwise win.
fn emoji_style(system: std::sync::Arc<egui_system_font_provider::SystemFontProvider>) -> epaint::text::GlyphRasterizer {
    use epaint::text::{FontProvider as _, GlyphBitmap, RasterizedGlyph};
    // Found on first use, so start-up does not wait for the font scan.
    let fonts: std::sync::OnceLock<Option<EmojiFonts>> = std::sync::OnceLock::new();
    epaint::text::GlyphRasterizer::new("tidal_editor::emoji_style", move |req| {
        if !epaint::text::has_emoji_presentation(req.cluster) {
            return None;
        }
        let fonts = fonts
            .get_or_init(|| {
                let family = egui::FontFamily::Proportional;
                // Probes that plain-text fonts such as DejaVu lack, so the answer is an emoji font.
                let found = ["🥰", "🫠", "🦀"]
                    .iter()
                    .filter_map(|c| system.font_for(&epaint::text::FallbackRequest { cluster: c, family: &family }))
                    .find(|f| has_colour(&f.data))?;
                let mut defs = egui::FontDefinitions::empty();
                defs.font_data.insert("emoji".into(), std::sync::Arc::new(found.data));
                defs.families.insert(family, vec!["emoji".into()]);
                Some(std::sync::Mutex::new(epaint::text::Fonts::new(Default::default(), defs)))
            })
            .as_ref()?;
        let mut fonts = fonts.lock().ok()?;
        let font_id = egui::FontId::proportional(req.font_size_px);
        if !fonts.with_pixels_per_point(1.0).has_glyph(&font_id, req.cluster.chars().next()?) {
            return None;
        }
        let job = egui::text::LayoutJob::simple_singleline(req.cluster.to_owned(), font_id, Color32::WHITE);
        let galley = fonts.layout_uncached(1.0, std::sync::Arc::new(job));
        // One visible glyph (selectors such as U+FE0F come out empty); more
        // means the font did not join the cluster: leave it to the normal fonts.
        let glyphs = &galley.rows.first()?.row.glyphs;
        let [g] = glyphs.iter().filter(|g| !g.uv_rect.is_nothing()).collect::<Vec<_>>()[..] else { return None };
        let uv = g.uv_rect;
        let advance: f32 = glyphs.iter().map(|g| g.advance_width).sum();
        let atlas = fonts.texture_atlas().image();
        let [x0, y0] = uv.min.map(usize::from);
        let [w, h] = [usize::from(uv.max[0]) - x0, usize::from(uv.max[1]) - y0];
        let pixels = (0..h).flat_map(|y| (0..w).map(move |x| (x0 + x, y0 + y))).map(|p| atlas[p]).collect();
        Some(RasterizedGlyph {
            bitmap: GlyphBitmap { image: egui::ColorImage::new([w, h], pixels), offset_px: uv.offset, is_color: true },
            advance_px: advance,
        })
    })
    .with_priority(epaint::text::FontPriority::Highest)
}

/// Does the font have colour glyphs (`COLR`, `CBDT` or `sbix`)?
fn has_colour(data: &egui::FontData) -> bool {
    use skrifa::raw::TableProvider as _;
    skrifa::FontRef::from_index((*data.font).as_ref(), data.index)
        .is_ok_and(|f| f.colr().is_ok() || f.cbdt().is_ok() || f.sbix().is_ok())
}

/// Last resort for emoji: the first of egui's black-and-white fonts that has the character.
struct MonoEmoji(Vec<(String, egui::FontData)>);

impl epaint::text::FontProvider for MonoEmoji {
    fn font_for(&self, request: &epaint::text::FallbackRequest<'_>) -> Option<epaint::text::FontInsert> {
        use skrifa::MetadataProvider as _;
        let c = request.cluster.chars().next()?;
        let (name, data) = self.0.iter().find(|(_, d)| {
            skrifa::FontRef::from_index((*d.font).as_ref(), d.index)
                .ok()
                .and_then(|f| f.charmap().map(c))
                .is_some_and(|g| g != skrifa::GlyphId::NOTDEF)
        })?;
        Some(epaint::text::FontInsert::new(
            name,
            data.clone(),
            vec![epaint::text::InsertFontFamily {
                family: request.family.clone(),
                priority: epaint::text::FontPriority::Lowest,
            }],
        ))
    }
}

pub fn apply(ctx: &egui::Context, cfg: &Config, reduced_motion: bool) {
    ctx.set_theme(match cfg.theme {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
        ThemeChoice::Light => egui::ThemePreference::Light,
    });
    for theme in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(theme, |s| {
            let dark = theme == Theme::Dark;
            let v = &mut s.visuals;
            // Text colours: WCAG AA (≥ 4.5:1) against the default panel fills.
            let text = if dark { Color32::from_gray(235) } else { Color32::from_gray(20) };
            let weak = if dark { Color32::from_gray(190) } else { Color32::from_gray(70) };
            v.override_text_color = Some(text);
            v.weak_text_color = Some(weak);
            v.hyperlink_color =
                if dark { Color32::from_rgb(0x8A, 0xC4, 0xFF) } else { Color32::from_rgb(0x00, 0x47, 0xB3) };
            v.selection.bg_fill =
                if dark { Color32::from_rgb(0x1E, 0x4F, 0x8F) } else { Color32::from_rgb(0xBF, 0xD9, 0xFF) };
            v.selection.stroke = Stroke::new(1.5, text);
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
                w.fg_stroke.color = text;
            }
            v.widgets.inactive.bg_stroke =
                Stroke::new(1.0, if dark { Color32::from_gray(140) } else { Color32::from_gray(110) });
            v.widgets.hovered.bg_stroke = Stroke::new(1.5, focus_color(dark));
            v.widgets.active.bg_stroke = Stroke::new(2.0, focus_color(dark));
            v.widgets.noninteractive.fg_stroke.color = text;
            s.spacing.interact_size.y = s.spacing.interact_size.y.max(26.0);
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.animation_time = if reduced_motion { 0.0 } else { 1.0 / 12.0 };
            s.scroll_animation = if reduced_motion { egui::style::ScrollAnimation::none() } else { Default::default() };
        });
    }
    ctx.set_zoom_factor(cfg.text_scale.clamp(0.5, 3.0));
}

pub fn focus_color(dark: bool) -> Color32 {
    if dark { FOCUS_DARK } else { FOCUS_LIGHT }
}

/// Draws a thick ring around the keyboard-focused widget, above everything else.
pub fn paint_focus_ring(ctx: &egui::Context) {
    let Some(id) = ctx.memory(|m| m.focused()) else { return };
    let Some(resp) = ctx.read_response(id) else { return };
    let dark = ctx.theme() == egui::Theme::Dark;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Debug, egui::Id::unique("focus-ring")));
    let rect = resp.rect.expand(2.0);
    painter.rect_stroke(rect, CornerRadius::same(4), Stroke::new(3.0, focus_color(dark)), egui::StrokeKind::Outside);
}

/// Best-effort read of the desktop "reduce animations" setting.
pub fn system_reduced_motion() -> bool {
    #[cfg(target_os = "linux")]
    {
        // KDE Plasma: AnimationDurationFactor=0 in kdeglobals.
        if let Some(home) = std::env::var_os("HOME") {
            let p = std::path::Path::new(&home).join(".config/kdeglobals");
            if let Ok(s) = std::fs::read_to_string(p)
                && let Some(line) = s.lines().find(|l| l.trim_start().starts_with("AnimationDurationFactor="))
                && line.split('=').nth(1).and_then(|v| v.trim().parse::<f32>().ok()) == Some(0.0)
            {
                return true;
            }
        }
        // GNOME and friends.
        if let Ok(out) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
            && String::from_utf8_lossy(&out.stdout).trim() == "false"
        {
            return true;
        }
        false
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // "Show animations in Windows" off ⇒ MinAnimate = 0. CREATE_NO_WINDOW avoids a console flash.
        std::process::Command::new("reg")
            .args(["query", r"HKCU\Control Panel\Desktop\WindowMetrics", "/v", "MinAnimate"])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout);
                s.lines().any(|l| l.contains("MinAnimate") && l.trim_end().ends_with('0'))
            })
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        false
    }
}

/// Re-reads the system setting every few seconds on a background thread, so a
/// change made while the app runs is followed. The thread ends with the app.
pub fn watch_system_motion(ctx: egui::Context) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let flag = std::sync::Arc::new(AtomicBool::new(system_reduced_motion()));
    let weak = std::sync::Arc::downgrade(&flag);
    let _ = std::thread::Builder::new().name("motion-watch".into()).spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let Some(f) = weak.upgrade() else { return };
            let now = system_reduced_motion();
            if f.swap(now, Ordering::Relaxed) != now {
                ctx.request_repaint();
            }
        }
    });
    flag
}

/// A busy indicator: the animated spinner, or a still "…" when motion is reduced.
pub fn busy(ui: &mut egui::Ui, reduced_motion: bool) {
    if reduced_motion {
        ui.label("…");
    } else {
        ui.spinner();
    }
}

pub fn reduced_motion(cfg: &Config) -> bool {
    match cfg.motion {
        MotionChoice::Reduced => true,
        MotionChoice::Full => false,
        MotionChoice::System => system_reduced_motion(),
    }
}

#[cfg(test)]
mod tests {
    use epaint::text::{FallbackRequest, FontProvider as _};

    #[test]
    fn bundled_emoji_cover_common_smileys() {
        let fonts = egui::FontDefinitions::default();
        let mono = super::MONO_EMOJI
            .iter()
            .filter_map(|n| fonts.font_data.get(*n).map(|d| ((*n).to_owned(), (**d).clone())))
            .collect();
        let p = super::MonoEmoji(mono);
        let family = egui::FontFamily::Proportional;
        for c in ["😀", "😂", "🎶", "🚗", "💔", "👍"] {
            let got = p.font_for(&FallbackRequest { cluster: c, family: &family });
            assert!(got.is_some_and(|f| f.name == "NotoEmoji-Regular"), "{c}");
        }
        assert!(p.font_for(&FallbackRequest { cluster: "日", family: &family }).is_none());
    }
}
