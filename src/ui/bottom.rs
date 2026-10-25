//! Bottom panel: preview player, running jobs and the status log.

use egui::RichText;

use super::App;
use crate::api::format_duration;
use crate::i18n;
use crate::player::PlayerCmd;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let tr = app.tr;
    ui.add_space(2.0);
    let ps = app.player.snapshot();
    // The player has taken (or failed on) the preview that was loading.
    if app.preview_status.is_some() && ps.plays > app.preview_plays {
        app.preview_status = None;
    }
    ui.horizontal_wrapped(|ui| {
        let label = if ps.playing && !ps.paused { tr.pause } else { tr.play };
        if ui.add_enabled(ps.playing, egui::Button::new(label)).on_hover_text("Ctrl+Space").clicked() {
            app.player.send(PlayerCmd::TogglePause);
        }
        // Stop also drops a preview that is still loading.
        if ui.add_enabled(ps.playing || app.preview_status.is_some(), egui::Button::new(tr.stop)).clicked() {
            app.player.send(PlayerCmd::Stop);
            app.worker.send(crate::worker::Cmd::StopPreview);
            app.preview_status = None;
        }
        if let Some(s) = &app.preview_status {
            super::style::busy(ui, app.reduced_motion);
            ui.label(s);
        } else if ps.playing || !ps.title.is_empty() {
            let pos = format_duration(ps.pos_secs as u32);
            let total = ps.total_secs.map(|t| format_duration(t as u32)).unwrap_or_else(|| "–".into());
            let kind = if ps.full { tr.full_track } else { tr.preview_clip };
            let state = if !ps.playing {
                tr.stopped
            } else if ps.paused {
                tr.paused
            } else {
                tr.playing
            };
            ui.label(format!("{state}: {} · {kind} · {pos} / {total}", ps.title));
            if !ps.track_id.is_empty() && ui.button(tr.open_in_tidal).clicked() {
                let t = crate::api::Track { id: ps.track_id.clone(), ..Default::default() };
                app.open_in_tidal(t.app_url(), t.web_url());
            }
        } else {
            ui.label(RichText::new(tr.nothing_playing).weak());
        }
        let mut vol = app.cfg.volume;
        let vl = ui.label(tr.volume);
        let slider = ui.add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false)).labelled_by(vl.id);
        if slider.changed() {
            app.cfg.volume = vol;
            app.player.send(PlayerCmd::Volume(vol));
        }
        if slider.drag_stopped() || slider.lost_focus() {
            app.save_cfg();
        }
    });
    if let Some(e) = &ps.error {
        ui.label(RichText::new(format!("⚠ {}: {e}", tr.audio_error)).strong());
    }

    let mut cancel = None;
    for j in &app.jobs {
        ui.horizontal(|ui| {
            let frac = if j.total > 0 { j.done as f32 / j.total as f32 } else { 0.0 };
            let text = if j.permille {
                format!("{} — {} %", j.label, j.done / 10)
            } else if j.total > 0 {
                format!("{} — {}/{}", j.label, j.done, j.total)
            } else {
                format!("{} …", j.label)
            };
            let mut bar = egui::ProgressBar::new(frac).text(text.clone()).desired_width(ui.available_width() - 110.0);
            if j.total == 0 && !app.reduced_motion {
                bar = bar.animate(true);
            }
            ui.add(bar).widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &text));
            let cancel_name = format!("{}: {}", tr.cancel, j.label);
            let b = ui.button(tr.cancel);
            b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &cancel_name));
            if b.clicked() {
                cancel = Some(j.job.id);
            }
        });
    }
    if let Some(id) = cancel
        && let Some(j) = app.jobs.iter().find(|j| j.job.id == id)
    {
        j.job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    ui.horizontal(|ui| {
        if let Some(last) = app.log.last() {
            let prefix = if last.error { format!("⚠ {}:", tr.error) } else { "ℹ".into() };
            let rt = RichText::new(format!("{} {prefix} {}", last.at, last.text));
            ui.add(egui::Label::new(if last.error { rt.strong() } else { rt }).wrap());
        } else {
            ui.label(RichText::new(tr.ready).weak());
        }
    });
    if app.show_log {
        egui::ScrollArea::vertical().id_salt("log").max_height(160.0).stick_to_bottom(true).show(ui, |ui| {
            for l in &app.log {
                let prefix = if l.error { format!("⚠ {}:", tr.error) } else { "ℹ".into() };
                ui.add(egui::Label::new(format!("{} {prefix} {}", l.at, l.text)).wrap().selectable(true));
            }
            if app.log.is_empty() {
                ui.label(i18n::fmt(tr.n_items, &["0"]));
            }
        });
    }
}
