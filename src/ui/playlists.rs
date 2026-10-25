//! Left panel: the user's playlists.

use egui::{Role, WidgetInfo};

use super::{App, Dialog, FocusTarget};
use crate::api::{Playlist, format_duration};
use crate::i18n;
use crate::worker::Cmd;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let tr = app.tr;
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.heading(tr.playlists);
        if ui.button(tr.new_short).on_hover_text(tr.new_playlist_dots).clicked() {
            app.open_dialog(Dialog::new_playlist(Vec::new()));
        }
        if ui.button(tr.refresh).clicked() {
            app.worker.send(Cmd::LoadPlaylists);
        }
    });
    let label = ui.label(tr.filter_playlists);
    let filter = ui
        .add(egui::TextEdit::singleline(&mut app.pl_filter).hint_text(tr.type_to_filter).desired_width(f32::INFINITY));
    let filter = filter.labelled_by(label.id);
    if app.focus == Some(FocusTarget::PlaylistFilter) {
        filter.request_focus();
        app.focus = None;
    }
    ui.separator();
    if !app.playlists_loaded {
        ui.horizontal(|ui| {
            super::style::busy(ui, app.reduced_motion);
            ui.label(tr.loading);
        });
        return;
    }
    if app.playlists.is_empty() {
        ui.label(tr.no_playlists_yet);
        return;
    }
    let needle = app.pl_filter.to_lowercase();
    let list: Vec<Playlist> = app
        .playlists
        .iter()
        .filter(|p| needle.is_empty() || p.name.to_lowercase().contains(&needle))
        .cloned()
        .collect();
    let mut action = None;
    egui::ScrollArea::vertical().id_salt("pl-scroll").auto_shrink([false, false]).show(ui, |ui| {
        for p in &list {
            let selected = app.current.as_deref() == Some(&p.id);
            let count = p.number_of_items.map(|n| n.to_string()).unwrap_or_else(|| "…".into());
            let mut detail = i18n::fmt(tr.n_items, &[&count]);
            if let Some(d) = p.duration_secs.filter(|d| *d > 0) {
                detail.push_str(&format!(" · {}", format_duration(d)));
            }
            if !p.owned {
                detail.push_str(&format!(" · {}", tr.collaborator));
            }
            let text = format!("{}\n{}", p.name, detail);
            let resp =
                ui.add(egui::Button::selectable(selected, text).wrap().min_size(egui::vec2(ui.available_width(), 0.0)));
            let a11y = format!("{}, {}", p.name, detail);
            resp.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &a11y));
            if resp.clicked() {
                action = Some(Action::Select(p.id.clone()));
            }
            resp.context_menu(|ui| {
                if ui.button(tr.edit_details_dots).clicked() {
                    action = Some(Action::Dialog(Dialog::edit(p)));
                }
                if ui.button(tr.duplicate_dots).clicked() {
                    action = Some(Action::Dialog(Dialog::duplicate(p, tr)));
                }
                if ui.button(tr.export_dots).clicked() {
                    action = Some(Action::Dialog(Dialog::export([p.id.clone()].into())));
                }
                if ui.button(tr.open_in_tidal).clicked() {
                    action = Some(Action::Open(p.app_url(), p.web_url()));
                }
                if p.owned && ui.button(tr.delete_playlist_dots).clicked() {
                    action = Some(Action::Dialog(Dialog::ConfirmDelete { pid: p.id.clone(), name: p.name.clone() }));
                }
            });
        }
    });
    match action {
        Some(Action::Select(id)) => app.select_playlist(&id),
        Some(Action::Dialog(d)) => app.open_dialog(d),
        Some(Action::Open(a, w)) => app.open_in_tidal(a, w),
        None => {}
    }
}

enum Action {
    Select(String),
    Dialog(Dialog),
    Open(String, String),
}
