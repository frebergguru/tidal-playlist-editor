//! Theme, contrast, focus ring and reduced-motion handling.

use egui::{Color32, CornerRadius, Stroke, Theme};

use crate::store::{Config, MotionChoice, ThemeChoice};

/// Focus ring colours with ≥ 3:1 contrast against the panel backgrounds.
const FOCUS_DARK: Color32 = Color32::from_rgb(0xFF, 0xD5, 0x4F);
const FOCUS_LIGHT: Color32 = Color32::from_rgb(0x00, 0x4F, 0xC4);

/// Noto Sans for text (regular weight reads better than the default light face),
/// DejaVu Sans as fallback for symbols such as ⚠ ✓ ▶ ↻ ⠿.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
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

pub fn reduced_motion(cfg: &Config) -> bool {
    match cfg.motion {
        MotionChoice::Reduced => true,
        MotionChoice::Full => false,
        MotionChoice::System => system_reduced_motion(),
    }
}
