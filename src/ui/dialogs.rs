//! Modal dialogs. Esc or Cancel closes every dialog.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::{Key, RichText};

use super::{App, JobKind};
use crate::api::{AccessType, ItemRef, Playlist, PlaylistItem, Track, format_duration};
use crate::i18n::{self, Tr};
use crate::ops::analysis::{self, DupGroup, DupMode, SortKey};
use crate::ops::files::{self, ExportFormat, ExportPlaylist, ImportPlaylist};
use crate::ops::matcher::{MatchMethod, MatchRow};
use crate::store::{Config, Language, MotionChoice, OpenIn, ThemeChoice};
use crate::worker::{Cmd, Reason, UnavailableHit};

pub struct SettingsForm {
    pub cfg: Config,
}

pub struct UnavailableDialog {
    pub setup: bool,
    pub all: bool,
    pub strict: bool,
    pub current: Option<String>,
    pub hits: Vec<UnavailableHit>,
    pub remove: Vec<bool>,
    pub replacements: Option<Vec<Option<Track>>>,
    pub use_repl: Vec<bool>,
    pub searching: bool,
}

impl UnavailableDialog {
    pub fn set_replacements(&mut self, list: Vec<(UnavailableHit, Option<Track>)>) {
        self.searching = false;
        let by_occ: HashMap<String, Option<Track>> = list.into_iter().map(|(h, t)| (h.item.item_id, t)).collect();
        let repl: Vec<Option<Track>> =
            self.hits.iter().map(|h| by_occ.get(&h.item.item_id).cloned().flatten()).collect();
        self.use_repl = repl.iter().map(Option::is_some).collect();
        self.replacements = Some(repl);
    }
}

#[derive(Clone, PartialEq)]
pub enum ImportTarget {
    New,
    Existing(String),
}

pub struct ImportEntry {
    pub source: ImportPlaylist,
    pub rows: Vec<MatchRow>,
    pub target: ImportTarget,
    pub name: String,
    pub skip_dups: bool,
}

pub struct RestoreEntry {
    pub backup: ExportPlaylist,
    pub target: Option<String>,
}

pub enum Dialog {
    Settings(Box<SettingsForm>),
    NewPlaylist {
        name: String,
        description: String,
        access: AccessType,
        add: Vec<ItemRef>,
    },
    Edit {
        pid: String,
        name: String,
        description: String,
        access: AccessType,
    },
    ConfirmDelete {
        pid: String,
        name: String,
    },
    Duplicate {
        source: Playlist,
        name: String,
    },
    Merge {
        selected: HashSet<String>,
        name: String,
        dedupe: bool,
    },
    AddTo {
        refs: Vec<ItemRef>,
        exclude: Option<String>,
        move_from: Option<HashSet<String>>,
        targets: HashSet<String>,
        new_name: String,
        skip_dups: bool,
    },
    Sort {
        pid: String,
        key: SortKey,
        asc: bool,
    },
    Stats(String),
    Unavailable(Box<UnavailableDialog>),
    Duplicates {
        pid: String,
        mode: DupMode,
        groups: Vec<DupGroup>,
        remove: HashSet<usize>,
    },
    ImportReview(Vec<ImportEntry>),
    Export {
        selected: HashSet<String>,
        format: ExportFormat,
    },
    Restore(Vec<RestoreEntry>),
    Shortcuts,
    About,
    Accessibility,
}

impl Dialog {
    pub fn settings(cfg: &Config) -> Self {
        Dialog::Settings(Box::new(SettingsForm { cfg: cfg.clone() }))
    }
    pub fn new_playlist(add: Vec<ItemRef>) -> Self {
        Dialog::NewPlaylist { name: String::new(), description: String::new(), access: AccessType::Unlisted, add }
    }
    pub fn edit(p: &Playlist) -> Self {
        Dialog::Edit {
            pid: p.id.clone(),
            name: p.name.clone(),
            description: p.description.clone(),
            access: p.access_type,
        }
    }
    pub fn duplicate(p: &Playlist, tr: &Tr) -> Self {
        Dialog::Duplicate { source: p.clone(), name: i18n::fmt(tr.copy_of, &[&p.name]) }
    }
    pub fn merge(selected: HashSet<String>, tr: &Tr) -> Self {
        Dialog::Merge { selected, name: tr.merged_playlist.into(), dedupe: true }
    }
    pub fn add_to(refs: Vec<ItemRef>, exclude: Option<String>, move_from: Option<HashSet<String>>) -> Self {
        Dialog::AddTo { refs, exclude, move_from, targets: HashSet::new(), new_name: String::new(), skip_dups: true }
    }
    pub fn sort(pid: &str) -> Self {
        Dialog::Sort { pid: pid.to_owned(), key: SortKey::Artist, asc: true }
    }
    pub fn export(selected: HashSet<String>) -> Self {
        Dialog::Export { selected, format: ExportFormat::Json }
    }
    pub fn unavailable_setup(current: Option<String>, strict: bool) -> Self {
        Dialog::Unavailable(Box::new(UnavailableDialog {
            setup: true,
            all: current.is_none(),
            strict,
            current,
            hits: Vec::new(),
            remove: Vec::new(),
            replacements: None,
            use_repl: Vec::new(),
            searching: false,
        }))
    }
    pub fn unavailable_results(hits: Vec<UnavailableHit>) -> Self {
        let n = hits.len();
        Dialog::Unavailable(Box::new(UnavailableDialog {
            setup: false,
            all: false,
            strict: false,
            current: None,
            remove: vec![true; n],
            hits,
            replacements: None,
            use_repl: vec![false; n],
            searching: false,
        }))
    }
    pub fn duplicates(app: &App, pid: &str) -> Self {
        let mode = DupMode::Exact;
        let groups = dup_groups(app, pid, mode);
        let remove = groups.iter().flat_map(|g| g.extra.iter().copied()).collect();
        Dialog::Duplicates { pid: pid.to_owned(), mode, groups, remove }
    }
    pub fn import_review(playlists: Vec<(ImportPlaylist, Vec<MatchRow>)>, mine: &[Playlist]) -> Self {
        Dialog::ImportReview(
            playlists
                .into_iter()
                .map(|(source, rows)| {
                    let existing = mine.iter().find(|p| p.name == source.name && p.owned).map(|p| p.id.clone());
                    let _ = existing;
                    ImportEntry { name: source.name.clone(), source, rows, target: ImportTarget::New, skip_dups: true }
                })
                .collect(),
        )
    }
}

fn dup_groups(app: &App, pid: &str, mode: DupMode) -> Vec<DupGroup> {
    app.items.get(pid).map(|items| analysis::find_duplicates(items, mode, &|id| app.cache.get(id))).unwrap_or_default()
}

enum Outcome {
    Keep,
    Close,
}

/// Opens the modal frame shared by every dialog; returns true when Esc was pressed.
fn frame<R>(ctx: &egui::Context, id: &str, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> (bool, R) {
    let max_h = ctx.content_rect().height() * 0.85;
    let max_w = (ctx.content_rect().width() * 0.9).min(980.0);
    let resp = egui::Modal::new(egui::Id::unique(("dialog", id))).show(ctx, |ui| {
        ui.set_max_width(max_w);
        ui.set_max_height(max_h);
        ui.heading(title);
        ui.add_space(4.0);
        add(ui)
    });
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    (esc, resp.inner)
}

fn track_desc(tr: &Tr, t: Option<&Track>, fallback_id: &str) -> String {
    match t {
        Some(t) if !t.title.is_empty() => {
            let who = t.artist_line();
            if who.is_empty() { t.full_title() } else { format!("{} — {who}", t.full_title()) }
        }
        _ => i18n::fmt(tr.unknown_item, &[fallback_id]),
    }
}

fn labelled_text(ui: &mut egui::Ui, label: &str, value: &mut String, first: bool, password: bool) -> egui::Response {
    let l = ui.label(label);
    let r = ui.add(egui::TextEdit::singleline(value).password(password).desired_width(f32::INFINITY)).labelled_by(l.id);
    if first {
        r.request_focus();
    }
    r
}

/// Text field with a Paste button (Ctrl+V also works) for long values such as
/// the Client ID. Secrets are hidden unless «Show» is ticked.
fn pasteable_text(ui: &mut egui::Ui, tr: &Tr, label: &str, value: &mut String, secret: bool) {
    let l = ui.label(label);
    let show_id = ui.scope_id().with(("show-secret", label));
    let mut show = ui.data(|d| d.get_temp::<bool>(show_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        let reserve = if secret { 190.0 } else { 90.0 };
        ui.add(
            egui::TextEdit::singleline(value)
                .password(secret && !show)
                .desired_width((ui.available_width() - reserve).max(120.0)),
        )
        .labelled_by(l.id);
        let paste = ui.button(tr.paste);
        paste.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{}: {label}", tr.paste)));
        if paste.clicked() {
            match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                Ok(text) => *value = text.trim().to_owned(),
                Err(e) => {
                    ui.data_mut(|d| d.insert_temp(show_id.with("err"), format!("{}: {e}", tr.paste_failed)));
                }
            }
        }
        if secret && ui.checkbox(&mut show, tr.show_secret).changed() {
            ui.data_mut(|d| d.insert_temp(show_id, show));
        }
    });
    if let Some(err) = ui.data(|d| d.get_temp::<String>(show_id.with("err"))) {
        ui.label(RichText::new(format!("⚠ {err}")).strong());
    }
}

fn access_radio(ui: &mut egui::Ui, tr: &Tr, access: &mut AccessType) {
    ui.label(tr.visibility);
    ui.horizontal(|ui| {
        ui.radio_value(access, AccessType::Unlisted, tr.access_unlisted);
        ui.radio_value(access, AccessType::Public, tr.access_public);
    });
}

fn ok_cancel(ui: &mut egui::Ui, tr: &Tr, ok: &str, enabled: bool) -> (bool, bool) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let okr = ui.add_enabled(enabled, egui::Button::new(ok)).clicked();
        let c = ui.button(tr.cancel).clicked();
        (okr, c)
    })
    .inner
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.dialog.take() else { return };
    let first = std::mem::take(&mut app.dialog_opened);
    let tr = app.tr;
    let outcome = match &mut d {
        Dialog::Settings(f) => settings(app, ctx, f, first),
        Dialog::NewPlaylist { name, description, access, add } => {
            let (esc, (ok, cancel)) = frame(ctx, "new", tr.new_playlist, |ui| {
                labelled_text(ui, tr.name, name, first, false);
                let l = ui.label(tr.description);
                ui.add(egui::TextEdit::multiline(description).desired_rows(3).desired_width(f32::INFINITY))
                    .labelled_by(l.id);
                access_radio(ui, tr, access);
                if !add.is_empty() {
                    ui.label(i18n::fmt(tr.will_add_n, &[&add.len().to_string()]));
                }
                ok_cancel(ui, tr, tr.create, !name.trim().is_empty())
            });
            if ok {
                let job = app.start_job(i18n::fmt(tr.creating, &[name.trim()]), JobKind::Plain);
                app.worker.send(Cmd::CreatePlaylist {
                    name: name.trim().to_owned(),
                    description: description.clone(),
                    access: *access,
                    add: std::mem::take(add),
                    job,
                });
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::Edit { pid, name, description, access } => {
            let (esc, (ok, cancel)) = frame(ctx, "edit", tr.edit_details, |ui| {
                labelled_text(ui, tr.name, name, first, false);
                let l = ui.label(tr.description);
                ui.add(egui::TextEdit::multiline(description).desired_rows(3).desired_width(f32::INFINITY))
                    .labelled_by(l.id);
                access_radio(ui, tr, access);
                ok_cancel(ui, tr, tr.save, !name.trim().is_empty())
            });
            if ok {
                app.worker.send(Cmd::UpdatePlaylist {
                    pid: pid.clone(),
                    name: name.trim().to_owned(),
                    description: description.clone(),
                    access: *access,
                });
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::ConfirmDelete { pid, name } => {
            let (esc, (ok, cancel)) = frame(ctx, "delete", tr.delete_playlist, |ui| {
                ui.label(i18n::fmt(tr.confirm_delete, &[name]));
                ui.label(RichText::new(tr.delete_cannot_undo).strong());

                ok_cancel(ui, tr, tr.delete, true)
            });
            if ok {
                app.worker.send(Cmd::DeletePlaylist(pid.clone()));
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::Duplicate { source, name } => {
            let (esc, (ok, cancel)) = frame(ctx, "dup", tr.duplicate_playlist, |ui| {
                ui.label(i18n::fmt(tr.duplicate_explain, &[&source.name]));
                labelled_text(ui, tr.new_name, name, first, false);
                ok_cancel(ui, tr, tr.duplicate, !name.trim().is_empty())
            });
            if ok {
                let job = app.start_job(i18n::fmt(tr.duplicating, &[&source.name]), JobKind::Plain);
                app.worker.send(Cmd::Duplicate { source: source.clone(), name: name.trim().to_owned(), job });
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::Merge { selected, name, dedupe } => {
            let list = app.playlists.clone();
            let (esc, (ok, cancel)) = frame(ctx, "merge", tr.merge_playlists, |ui| {
                ui.label(tr.merge_explain);
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for p in &list {
                        let mut on = selected.contains(&p.id);
                        if ui.checkbox(&mut on, &p.name).changed() {
                            if on {
                                selected.insert(p.id.clone())
                            } else {
                                selected.remove(&p.id)
                            };
                        }
                    }
                });
                labelled_text(ui, tr.new_name, name, first, false);
                ui.checkbox(dedupe, tr.skip_duplicates);
                ok_cancel(ui, tr, tr.merge, selected.len() >= 2 && !name.trim().is_empty())
            });
            if ok {
                let sources: Vec<Playlist> = list.into_iter().filter(|p| selected.contains(&p.id)).collect();
                let job = app.start_job(i18n::fmt(tr.merging_n, &[&sources.len().to_string()]), JobKind::Plain);
                app.worker.send(Cmd::Merge { sources, name: name.trim().to_owned(), dedupe: *dedupe, job });
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::AddTo { refs, exclude, move_from, targets, new_name, skip_dups } => {
            let list: Vec<Playlist> =
                app.playlists.iter().filter(|p| Some(&p.id) != exclude.as_ref()).cloned().collect();
            let title = if move_from.is_some() { tr.move_to } else { tr.add_to };
            let (esc, (ok, cancel)) = frame(ctx, "addto", title, |ui| {
                ui.label(i18n::fmt(tr.n_tracks_chosen, &[&refs.len().to_string()]));
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (n, p) in list.iter().enumerate() {
                        let mut on = targets.contains(&p.id);
                        let r = ui.checkbox(&mut on, &p.name);
                        if first && n == 0 {
                            r.request_focus();
                        }
                        if r.changed() {
                            if on {
                                targets.insert(p.id.clone())
                            } else {
                                targets.remove(&p.id)
                            };
                        }
                    }
                });
                labelled_text(ui, tr.or_new_playlist_named, new_name, false, false);
                ui.checkbox(skip_dups, tr.skip_already_present);
                ok_cancel(ui, tr, title, !targets.is_empty() || !new_name.trim().is_empty())
            });
            if ok {
                for pid in targets.iter() {
                    app.add_to_playlist(pid, refs.clone(), *skip_dups);
                }
                if !new_name.trim().is_empty() {
                    let job = app.start_job(i18n::fmt(tr.creating, &[new_name.trim()]), JobKind::Plain);
                    app.worker.send(Cmd::CreatePlaylist {
                        name: new_name.trim().to_owned(),
                        description: String::new(),
                        access: AccessType::Unlisted,
                        add: refs.clone(),
                        job,
                    });
                }
                if let (Some(ids), Some(src)) = (move_from.as_ref(), exclude.as_ref()) {
                    app.remove_from_playlist(src, ids);
                }
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::Sort { pid, key, asc } => {
            let (esc, (ok, cancel)) = frame(ctx, "sort", tr.sort_permanently, |ui| {
                ui.label(tr.sort_explain);
                for k in [SortKey::Artist, SortKey::Title, SortKey::Album, SortKey::Duration, SortKey::Added] {
                    let r = ui.radio_value(key, k, i18n::sort_label(tr, k));
                    if first && k == SortKey::Artist {
                        r.request_focus();
                    }
                }
                ui.horizontal(|ui| {
                    ui.radio_value(asc, true, tr.ascending);
                    ui.radio_value(asc, false, tr.descending);
                });
                ok_cancel(ui, tr, tr.sort, true)
            });
            if ok && let Some(items) = app.items.get(pid.as_str()).cloned() {
                let order = analysis::sorted(&items, *key, *asc);
                super::tracks::apply_order(app, pid, items, order, tr.sort_permanently);
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::Stats(pid) => {
            let st = app.items.get(pid.as_str()).map(|v| analysis::stats(v)).unwrap_or_default();
            let name = app.playlist(pid).map(|p| p.name.clone()).unwrap_or_default();
            let (esc, close) = frame(ctx, "stats", &format!("{} — {name}", tr.statistics), |ui| {
                egui::Grid::new("stats-grid").num_columns(2).striped(true).show(ui, |ui| {
                    let rows: [(&str, String); 9] = [
                        (tr.stat_items, st.items.to_string()),
                        (tr.tracks, st.tracks.to_string()),
                        (tr.stat_videos, st.videos.to_string()),
                        (tr.stat_total_time, format_duration(st.total_secs as u32)),
                        (tr.artists, st.artists.to_string()),
                        (tr.albums, st.albums.to_string()),
                        (tr.status_unavailable, st.unavailable.to_string()),
                        (tr.explicit, st.explicit.to_string()),
                        (tr.stat_duplicates, st.duplicates.to_string()),
                    ];
                    for (k, v) in rows {
                        ui.label(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
                if !st.top_artists.is_empty() {
                    ui.add_space(6.0);
                    ui.strong(tr.top_artists);
                    for (a, n) in &st.top_artists {
                        ui.label(format!("{a}: {n}"));
                    }
                }
                ui.add_space(8.0);
                let b = ui.button(tr.close);
                if first {
                    b.request_focus();
                }
                b.clicked()
            });
            if close || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::Unavailable(u) => unavailable(app, ctx, u, first),
        Dialog::Duplicates { pid, mode, groups, remove } => {
            let items = app.items.get(pid.as_str()).cloned().unwrap_or_default();
            let mut new_mode = *mode;
            let (esc, (ok, cancel)) = frame(ctx, "dups", tr.find_duplicates, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr.compare_by);
                    let r = ui.radio_value(&mut new_mode, DupMode::Exact, tr.dup_exact);
                    if first {
                        r.request_focus();
                    }
                    ui.radio_value(&mut new_mode, DupMode::Isrc, tr.dup_isrc);
                    ui.radio_value(&mut new_mode, DupMode::Loose, tr.dup_loose);
                });
                if groups.is_empty() {
                    ui.label(tr.no_duplicates);
                }
                egui::ScrollArea::vertical().max_height(ui.available_height() - 80.0).show(ui, |ui| {
                    for g in groups.iter() {
                        let keep = items.get(g.keep);
                        ui.label(
                            RichText::new(format!(
                                "{} {}: {}",
                                tr.keep,
                                g.keep + 1,
                                keep.map(|k| track_desc(tr, app.track_meta(k).as_ref(), &k.r.id)).unwrap_or_default()
                            ))
                            .strong(),
                        );
                        for &e in &g.extra {
                            let Some(it) = items.get(e) else { continue };
                            let mut on = remove.contains(&e);
                            let text = format!(
                                "{} {}: {}",
                                tr.remove,
                                e + 1,
                                track_desc(tr, app.track_meta(it).as_ref(), &it.r.id)
                            );
                            if ui.checkbox(&mut on, text).changed() {
                                if on {
                                    remove.insert(e)
                                } else {
                                    remove.remove(&e)
                                };
                            }
                        }
                        ui.separator();
                    }
                });
                ok_cancel(ui, tr, &i18n::fmt(tr.remove_n, &[&remove.len().to_string()]), !remove.is_empty())
            });
            if new_mode != *mode {
                *mode = new_mode;
                *groups = dup_groups(app, pid, new_mode);
                *remove = groups.iter().flat_map(|g| g.extra.iter().copied()).collect();
            }
            if ok {
                let ids: HashSet<String> =
                    remove.iter().filter_map(|&i| items.get(i)).map(|i| i.item_id.clone()).collect();
                app.remove_from_playlist(pid, &ids);
            }
            if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::ImportReview(entries) => import_review(app, ctx, entries, first),
        Dialog::Export { selected, format } => {
            let list = app.playlists.clone();
            let (esc, (ok, cancel)) = frame(ctx, "export", tr.export_playlists, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr.format);
                    for (n, f) in ExportFormat::ALL.into_iter().enumerate() {
                        let r = ui.radio_value(format, f, f.label());
                        if first && n == 0 {
                            r.request_focus();
                        }
                    }
                });
                ui.label(match format {
                    ExportFormat::Json => tr.fmt_json_help,
                    ExportFormat::Csv => tr.fmt_csv_help,
                    ExportFormat::M3u8 => tr.fmt_m3u_help,
                    ExportFormat::Xspf => tr.fmt_xspf_help,
                });
                ui.horizontal(|ui| {
                    if ui.button(tr.select_all).clicked() {
                        *selected = list.iter().map(|p| p.id.clone()).collect();
                    }
                    if ui.button(tr.select_none).clicked() {
                        selected.clear();
                    }
                });
                egui::ScrollArea::vertical().max_height(ui.available_height() - 80.0).show(ui, |ui| {
                    for p in &list {
                        let mut on = selected.contains(&p.id);
                        if ui.checkbox(&mut on, &p.name).changed() {
                            if on {
                                selected.insert(p.id.clone())
                            } else {
                                selected.remove(&p.id)
                            };
                        }
                    }
                });
                ok_cancel(ui, tr, tr.choose_folder_and_export, !selected.is_empty())
            });
            let mut close = cancel || esc;
            if ok {
                let start = app
                    .cfg
                    .export_dir
                    .clone()
                    .or_else(|| directories::UserDirs::new().and_then(|u| u.document_dir().map(Path::to_path_buf)));
                let mut dlg = rfd::FileDialog::new().set_title(tr.choose_folder_and_export);
                if let Some(s) = start {
                    dlg = dlg.set_directory(s);
                }
                if let Some(dir) = dlg.pick_folder() {
                    app.cfg.export_dir = Some(dir.clone());
                    app.save_cfg();
                    let pls: Vec<Playlist> = list.into_iter().filter(|p| selected.contains(&p.id)).collect();
                    let job = app.start_job(i18n::fmt(tr.exporting_n, &[&pls.len().to_string()]), JobKind::Export);
                    app.worker.send(Cmd::FetchForExport { playlists: pls, format: *format, dir, backup: false, job });
                    close = true;
                }
            }
            if close { Outcome::Close } else { Outcome::Keep }
        }
        Dialog::Restore(entries) => restore(app, ctx, entries, first),
        Dialog::Shortcuts => text_dialog(ctx, "keys", tr.keyboard_shortcuts, tr.shortcuts_text, tr, first),
        Dialog::About => {
            let text = i18n::fmt(tr.about_text, &[env!("CARGO_PKG_VERSION")]);
            text_dialog(ctx, "about", tr.about, &text, tr, first)
        }
        Dialog::Accessibility => text_dialog(ctx, "a11y", tr.accessibility_statement, tr.accessibility_text, tr, first),
    };
    if matches!(outcome, Outcome::Keep) && app.dialog.is_none() {
        app.dialog = Some(d);
    }
}

fn text_dialog(ctx: &egui::Context, id: &str, title: &str, text: &str, tr: &Tr, first: bool) -> Outcome {
    let (esc, close) = frame(ctx, id, title, |ui| {
        egui::ScrollArea::vertical().max_height(ui.available_height() - 60.0).show(ui, |ui| {
            ui.add(egui::Label::new(text).wrap().selectable(true));
        });
        ui.add_space(8.0);
        let b = ui.button(tr.close);
        if first {
            b.request_focus();
        }
        b.clicked()
    });
    if close || esc { Outcome::Close } else { Outcome::Keep }
}

fn settings(app: &mut App, ctx: &egui::Context, f: &mut SettingsForm, first: bool) -> Outcome {
    let tr = app.tr;
    let c = &mut f.cfg;
    let (esc, (ok, cancel)) = frame(ctx, "settings", tr.settings, |ui| {
        egui::ScrollArea::vertical().max_height(ui.available_height() - 60.0).show(ui, |ui| {
            ui.strong(tr.appearance);
            let lang = |l: Language| match l {
                Language::Auto => tr.lang_auto,
                Language::English => "English",
                Language::Norwegian => "Norsk (bokmål)",
            };
            let r = egui::ComboBox::from_label(tr.language).selected_text(lang(c.language)).show_ui(ui, |ui| {
                for l in [Language::Auto, Language::English, Language::Norwegian] {
                    ui.selectable_value(&mut c.language, l, lang(l));
                }
            });
            if first {
                r.response.request_focus();
            }
            let theme = |t: ThemeChoice| match t {
                ThemeChoice::System => tr.follow_system,
                ThemeChoice::Dark => tr.theme_dark,
                ThemeChoice::Light => tr.theme_light,
            };
            egui::ComboBox::from_label(tr.theme).selected_text(theme(c.theme)).show_ui(ui, |ui| {
                for t in [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light] {
                    ui.selectable_value(&mut c.theme, t, theme(t));
                }
            });
            let motion = |m: MotionChoice| match m {
                MotionChoice::System => tr.follow_system,
                MotionChoice::Reduced => tr.motion_reduced,
                MotionChoice::Full => tr.motion_full,
            };
            egui::ComboBox::from_label(tr.animations).selected_text(motion(c.motion)).show_ui(ui, |ui| {
                for m in [MotionChoice::System, MotionChoice::Reduced, MotionChoice::Full] {
                    ui.selectable_value(&mut c.motion, m, motion(m));
                }
            });
            let l = ui.label(tr.text_size);
            ui.add(
                egui::Slider::new(&mut c.text_scale, 0.5..=3.0)
                    .step_by(0.1)
                    .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
            )
            .labelled_by(l.id);
            ui.separator();
            ui.strong(tr.behaviour);
            ui.horizontal(|ui| {
                ui.label(tr.open_links_in);
                ui.radio_value(&mut c.open_in, OpenIn::App, tr.tidal_app);
                ui.radio_value(&mut c.open_in, OpenIn::Web, tr.web_browser);
            });
            ui.checkbox(&mut c.strict_availability, tr.strict_check);
            ui.separator();
            ui.strong(tr.tidal_api);
            ui.label(tr.api_explain);
            pasteable_text(ui, tr, tr.client_id, &mut c.client_id, false);
            pasteable_text(ui, tr, tr.client_secret_optional, &mut c.client_secret, true);
            pasteable_text(ui, tr, tr.redirect_uri, &mut c.redirect_uri, false);
            ui.label(RichText::new(tr.paste_hint).weak());
            ui.hyperlink_to(tr.developer_portal, "https://developer.tidal.com/dashboard");
        });
        let valid = url::Url::parse(c.redirect_uri.trim()).is_ok_and(|u| u.port_or_known_default().is_some());
        if !valid {
            ui.label(RichText::new(format!("⚠ {}", tr.redirect_invalid)).strong());
        }
        ok_cancel(ui, tr, tr.save, valid)
    });
    if ok {
        let api_changed = app.cfg.client_id != c.client_id
            || app.cfg.redirect_uri != c.redirect_uri
            || app.cfg.client_secret != c.client_secret;
        c.client_id = c.client_id.trim().to_owned();
        c.redirect_uri = c.redirect_uri.trim().to_owned();
        // Keep runtime state the dialog does not edit.
        c.last_playlist = app.cfg.last_playlist.clone();
        c.export_dir = app.cfg.export_dir.clone();
        c.volume = app.cfg.volume;
        c.show_search = app.cfg.show_search;
        app.cfg = c.clone();
        app.set_text_scale(ctx, c.text_scale);
        app.restyle(ctx);
        if api_changed && app.auth == super::AuthState::LoggedIn {
            app.note(app.tr.api_changed_relogin);
        }
    }
    if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn reason_text(tr: &Tr, r: &Reason) -> &'static str {
    match r {
        Reason::Missing => tr.reason_missing,
        Reason::NotStreamable => tr.reason_not_streamable,
        Reason::NoUsageRule => tr.reason_no_rule,
    }
}

fn unavailable(app: &mut App, ctx: &egui::Context, u: &mut UnavailableDialog, first: bool) -> Outcome {
    let tr = app.tr;
    if u.setup {
        let cur_name = u.current.as_ref().and_then(|id| app.playlist(id)).map(|p| p.name.clone());
        let (esc, (ok, cancel)) = frame(ctx, "unav-setup", tr.find_unavailable, |ui| {
            ui.label(tr.unavailable_explain);
            if let Some(n) = &cur_name {
                let r = ui.radio_value(&mut u.all, false, i18n::fmt(tr.only_playlist, &[n]));
                if first {
                    r.request_focus();
                }
            }
            ui.radio_value(&mut u.all, true, i18n::fmt(tr.all_my_playlists, &[&app.playlists.len().to_string()]));
            ui.checkbox(&mut u.strict, tr.strict_check);
            ok_cancel(ui, tr, tr.start_scan, true)
        });
        if ok {
            let pls: Vec<Playlist> = if u.all || cur_name.is_none() {
                app.playlists.clone()
            } else {
                app.playlists.iter().filter(|p| Some(&p.id) == u.current.as_ref()).cloned().collect()
            };
            app.cfg.strict_availability = u.strict;
            app.save_cfg();
            let job = app.start_job(tr.scanning, JobKind::Scan);
            app.worker.send(Cmd::ScanUnavailable { playlists: pls, strict: u.strict, job });
        }
        return if ok || cancel || esc { Outcome::Close } else { Outcome::Keep };
    }
    let mut find = false;
    let (esc, (ok, cancel)) = frame(ctx, "unav", tr.unavailable_songs, |ui| {
        if u.hits.is_empty() {
            ui.label(tr.no_unavailable);
            let b = ui.button(tr.close);
            if first {
                b.request_focus();
            }
            return (false, b.clicked());
        }
        ui.label(i18n::fmt(tr.found_n_unavailable, &[&u.hits.len().to_string()]));
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr.select_all).clicked() {
                u.remove.iter_mut().for_each(|r| *r = true);
            }
            if ui.button(tr.select_none).clicked() {
                u.remove.iter_mut().for_each(|r| *r = false);
            }
            let fr = ui.add_enabled(!u.searching, egui::Button::new(tr.find_replacements));
            if first {
                fr.request_focus();
            }
            if fr.clicked() {
                find = true;
            }
            if u.searching {
                ui.spinner();
                ui.label(tr.searching);
            }
        });
        egui::ScrollArea::vertical().max_height(ui.available_height() - 80.0).show(ui, |ui| {
            let mut last_pl = String::new();
            for (n, h) in u.hits.iter().enumerate() {
                if h.pid != last_pl {
                    ui.add_space(4.0);
                    ui.strong(&h.playlist);
                    last_pl = h.pid.clone();
                }
                ui.push_id(n, |ui| {
                    let desc = track_desc(tr, h.known.as_ref(), &h.item.r.id);
                    let text = format!("{desc} — {}", reason_text(tr, &h.reason));
                    ui.checkbox(&mut u.remove[n], text);
                    if let Some(Some(rep)) = u.replacements.as_ref().map(|r| &r[n]) {
                        ui.indent("rep", |ui| {
                            let t = format!(
                                "{} {} ({})",
                                tr.replace_with,
                                track_desc(tr, Some(rep), &rep.id),
                                rep.album.clone().unwrap_or_default()
                            );
                            ui.checkbox(&mut u.use_repl[n], t);
                        });
                    } else if u.replacements.is_some() {
                        ui.indent("rep", |ui| ui.label(RichText::new(tr.no_replacement_found).italics()));
                    }
                });
            }
        });
        let n = u.remove.iter().filter(|r| **r).count();
        ok_cancel(ui, tr, &i18n::fmt(tr.apply_to_n, &[&n.to_string()]), n > 0)
    });
    if find {
        u.searching = true;
        let job = app.start_job(tr.find_replacements, JobKind::Replacements);
        app.worker.send(Cmd::FindReplacements { hits: u.hits.clone(), job });
    }
    if ok {
        // Per playlist: (replacements as old → new, plain removals).
        type Plan = (Vec<(PlaylistItem, Track)>, HashSet<String>);
        let mut by_pid: HashMap<String, Plan> = HashMap::new();
        for (n, h) in u.hits.iter().enumerate() {
            if !u.remove[n] {
                continue;
            }
            let entry = by_pid.entry(h.pid.clone()).or_default();
            match u.replacements.as_ref().and_then(|r| r[n].clone()) {
                Some(rep) if u.use_repl[n] => entry.0.push((h.item.clone(), rep)),
                _ => {
                    entry.1.insert(h.item.item_id.clone());
                }
            }
        }
        for (pid, (swaps, removes)) in by_pid {
            if !removes.is_empty() {
                app.remove_from_playlist(&pid, &removes);
            }
            if !swaps.is_empty() {
                app.snapshot(&pid);
                let job = app.start_job(i18n::fmt(tr.replacing_n, &[&swaps.len().to_string()]), JobKind::Plain);
                app.worker.send(Cmd::Replace { pid, swaps, job });
            }
        }
    }
    if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn method_text(tr: &Tr, m: MatchMethod) -> &'static str {
    match m {
        MatchMethod::Id => tr.match_id,
        MatchMethod::Isrc => tr.match_isrc,
        MatchMethod::Search => tr.match_search,
        MatchMethod::None => tr.match_none,
    }
}

fn import_review(app: &mut App, ctx: &egui::Context, entries: &mut [ImportEntry], first: bool) -> Outcome {
    let tr = app.tr;
    let mine: Vec<Playlist> = app.playlists.iter().filter(|p| p.owned).cloned().collect();
    let (esc, (ok, cancel)) = frame(ctx, "import", tr.import_review, |ui| {
        ui.label(tr.import_review_explain);
        egui::ScrollArea::vertical().max_height(ui.available_height() - 80.0).show(ui, |ui| {
            for (e_idx, e) in entries.iter_mut().enumerate() {
                ui.push_id(e_idx, |ui| {
                    let matched = e.rows.iter().filter(|r| r.matched.is_some()).count();
                    let included = e.rows.iter().filter(|r| r.include && r.matched.is_some()).count();
                    ui.strong(format!(
                        "{} — {}",
                        e.source.name,
                        i18n::fmt(tr.matched_of, &[&matched.to_string(), &e.rows.len().to_string()])
                    ));
                    ui.horizontal_wrapped(|ui| {
                        let r = ui.radio_value(&mut e.target, ImportTarget::New, tr.new_playlist_named);
                        if first && e_idx == 0 {
                            r.request_focus();
                        }
                        let l = ui.label(tr.name);
                        ui.add_enabled(
                            e.target == ImportTarget::New,
                            egui::TextEdit::singleline(&mut e.name).desired_width(240.0),
                        )
                        .labelled_by(l.id);
                    });
                    ui.horizontal_wrapped(|ui| {
                        let current = match &e.target {
                            ImportTarget::Existing(id) => {
                                mine.iter().find(|p| &p.id == id).map(|p| p.name.clone()).unwrap_or_default()
                            }
                            ImportTarget::New => tr.choose_dots.to_owned(),
                        };
                        let l = ui.label(tr.or_add_to_existing);
                        let c = egui::ComboBox::from_id_salt(("imp-target", e_idx)).selected_text(current).show_ui(
                            ui,
                            |ui| {
                                for p in &mine {
                                    ui.selectable_value(&mut e.target, ImportTarget::Existing(p.id.clone()), &p.name);
                                }
                            },
                        );
                        c.response.labelled_by(l.id);
                        if matches!(e.target, ImportTarget::Existing(_)) {
                            ui.checkbox(&mut e.skip_dups, tr.skip_already_present);
                        }
                    });
                    ui.label(i18n::fmt(tr.will_add_n, &[&included.to_string()]));
                    for (n, r) in e.rows.iter_mut().enumerate() {
                        ui.push_id(n, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let src = r.row.describe();
                                let target = match &r.matched {
                                    Some(t) => format!(
                                        "→ {} [{} {:.0} %]",
                                        track_desc(tr, Some(t), &t.id),
                                        method_text(tr, r.method),
                                        r.confidence * 100.0
                                    ),
                                    None => format!("→ ⚠ {}", tr.match_none),
                                };
                                let enabled = r.matched.is_some();
                                ui.add_enabled(
                                    enabled,
                                    egui::Checkbox::new(&mut r.include, format!("{}. {src} {target}", n + 1)),
                                );
                                if !r.alternatives.is_empty() {
                                    let alts = r.alternatives.clone();
                                    let c = egui::ComboBox::from_id_salt(("alt", n))
                                        .selected_text(tr.other_match)
                                        .show_ui(ui, |ui| {
                                            for a in alts {
                                                if ui.button(track_desc(tr, Some(&a), &a.id)).clicked() {
                                                    if let Some(old) = r.matched.replace(a.clone()) {
                                                        r.alternatives.retain(|x| x.id != a.id);
                                                        r.alternatives.insert(0, old);
                                                    }
                                                    r.include = true;
                                                    r.method = MatchMethod::Search;
                                                    r.confidence = crate::ops::matcher::score(&r.row, &a);
                                                }
                                            }
                                        });
                                    let name = format!("{}: {}", tr.other_match, src);
                                    c.response
                                        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &name));
                                }
                            });
                        });
                    }
                    ui.separator();
                });
            }
        });
        let total: usize =
            entries.iter().map(|e| e.rows.iter().filter(|r| r.include && r.matched.is_some()).count()).sum();
        let valid = entries.iter().all(|e| e.target != ImportTarget::New || !e.name.trim().is_empty());
        ok_cancel(ui, tr, &i18n::fmt(tr.import_n, &[&total.to_string()]), total > 0 && valid)
    });
    if ok {
        for e in entries.iter() {
            let refs: Vec<ItemRef> =
                e.rows.iter().filter(|r| r.include).filter_map(|r| r.matched.as_ref()).map(Track::item_ref).collect();
            if refs.is_empty() {
                continue;
            }
            match &e.target {
                ImportTarget::New => {
                    let job = app.start_job(i18n::fmt(tr.creating, &[e.name.trim()]), JobKind::Plain);
                    app.worker.send(Cmd::CreatePlaylist {
                        name: e.name.trim().to_owned(),
                        description: e.source.description.clone(),
                        access: e.source.access_type,
                        add: refs,
                        job,
                    });
                }
                ImportTarget::Existing(pid) => app.add_to_playlist(pid, refs, e.skip_dups),
            }
        }
    }
    if ok || cancel || esc { Outcome::Close } else { Outcome::Keep }
}

fn restore(app: &mut App, ctx: &egui::Context, entries: &mut [RestoreEntry], first: bool) -> Outcome {
    let tr = app.tr;
    let mine = app.playlists.clone();
    let mut load = Vec::new();
    let mut restore_into = None;
    let mut import_new = None;
    let (esc, close) = frame(ctx, "restore", tr.restore_backup, |ui| {
        ui.label(tr.restore_explain);
        egui::ScrollArea::vertical().max_height(ui.available_height() - 60.0).show(ui, |ui| {
            for (n, e) in entries.iter_mut().enumerate() {
                ui.push_id(n, |ui| {
                    ui.strong(format!(
                        "{} ({})",
                        e.backup.name,
                        i18n::fmt(tr.n_items, &[&e.backup.items.len().to_string()])
                    ));
                    ui.horizontal_wrapped(|ui| {
                        let l = ui.label(tr.compare_with);
                        let current =
                            e.target.as_ref().and_then(|id| mine.iter().find(|p| &p.id == id)).map(|p| p.name.clone());
                        let r = egui::ComboBox::from_id_salt(("rt", n))
                            .selected_text(current.unwrap_or_else(|| tr.choose_dots.into()))
                            .show_ui(ui, |ui| {
                                for p in &mine {
                                    ui.selectable_value(&mut e.target, Some(p.id.clone()), &p.name);
                                }
                            });
                        let resp = r.response.labelled_by(l.id);
                        if first && n == 0 {
                            resp.request_focus();
                        }
                    });
                    if let Some(pid) = &e.target {
                        match app.items.get(pid) {
                            None => {
                                if !app.loading.contains(pid) {
                                    load.push(pid.clone());
                                }
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(tr.loading);
                                });
                            }
                            Some(items) => {
                                let refs: Vec<ItemRef> = items.iter().map(|i| i.r.clone()).collect();
                                let target = e.backup.refs();
                                let d = analysis::diff(&refs, &target);
                                if d.is_empty() {
                                    ui.label(format!("✓ {}", tr.identical));
                                } else {
                                    ui.label(i18n::fmt(
                                        tr.diff_summary,
                                        &[&d.missing.len().to_string(), &d.extra.len().to_string()],
                                    ));
                                    if d.order_differs {
                                        ui.label(tr.only_order_differs);
                                    }
                                    let names: HashMap<&str, &files::ExportItem> =
                                        e.backup.items.iter().map(|i| (i.id.as_str(), i)).collect();
                                    egui::CollapsingHeader::new(tr.show_differences).id_salt(("diff", n)).show(
                                        ui,
                                        |ui| {
                                            for r in d.missing.iter().take(200) {
                                                let label = names
                                                    .get(r.id.as_str())
                                                    .map(|i| format!("{} — {}", i.title, i.artists.join(", ")))
                                                    .unwrap_or_else(|| r.id.clone());
                                                ui.label(format!("+ {label}"));
                                            }
                                            for r in d.extra.iter().take(200) {
                                                let it = items.iter().find(|i| &i.r == r);
                                                let label = it
                                                    .map(|i| {
                                                        track_desc(
                                                            tr,
                                                            app.cache.get(&i.r.id).as_ref().or(i.track.as_ref()),
                                                            &r.id,
                                                        )
                                                    })
                                                    .unwrap_or_else(|| r.id.clone());
                                                ui.label(format!("− {label}"));
                                            }
                                        },
                                    );
                                    if ui.button(tr.restore_into_this).clicked() {
                                        restore_into = Some((pid.clone(), target));
                                    }
                                }
                            }
                        }
                    }
                    if ui.button(tr.import_as_new).clicked() {
                        import_new = Some(n);
                    }
                    ui.separator();
                });
            }
        });
        ui.button(tr.close).clicked()
    });
    for pid in load {
        app.load_items(&pid);
    }
    if let Some((pid, target)) = restore_into {
        app.snapshot(&pid);
        let job = app.start_job(tr.restoring, JobKind::Plain);
        app.worker.send(Cmd::Restore { pid, target, job });
    }
    if let Some(n) = import_new {
        let pl = files::parse_json(&files::to_json(&files::export_file(vec![entries[n].backup.clone()])))
            .unwrap_or_default();
        let job = app.start_job(tr.matching, JobKind::Match);
        app.worker.send(Cmd::Match { playlists: pl, job });
    }
    if close || esc { Outcome::Close } else { Outcome::Keep }
}

// ------------------------------------------------------------- file actions

pub fn start_import(app: &mut App) {
    let tr = app.tr;
    let Some(path) = rfd::FileDialog::new()
        .set_title(tr.import_dots)
        .add_filter(tr.playlist_files, &["json", "csv", "tsv", "txt", "m3u", "m3u8", "xspf", "xml"])
        .pick_file()
    else {
        return;
    };
    let content = match std::fs::read(&path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return app.error(format!("{}: {e}", tr.could_not_read_file)),
    };
    match files::parse_import(&path, &content) {
        Ok(pls) if pls.iter().all(|p| p.rows.is_empty()) => app.error(tr.file_has_no_tracks),
        Ok(pls) => {
            let n: usize = pls.iter().map(|p| p.rows.len()).sum();
            let job = app.start_job(i18n::fmt(tr.matching_n, &[&n.to_string()]), JobKind::Match);
            app.worker.send(Cmd::Match { playlists: pls, job });
        }
        Err(e) => app.error(format!("{}: {e}", tr.could_not_read_file)),
    }
}

pub fn start_backup(app: &mut App) {
    let dir = app.paths.backup_dir.join(chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string());
    let job = app.start_job(app.tr.backing_up, JobKind::Export);
    app.worker.send(Cmd::FetchForExport {
        playlists: app.playlists.clone(),
        format: ExportFormat::Json,
        dir,
        backup: true,
        job,
    });
}

pub fn start_restore(app: &mut App) {
    let tr = app.tr;
    let mut dlg = rfd::FileDialog::new().set_title(tr.restore_backup_dots).add_filter("JSON", &["json"]);
    if app.paths.backup_dir.exists() {
        dlg = dlg.set_directory(&app.paths.backup_dir);
    }
    let Some(path) = dlg.pick_file() else { return };
    let parsed = std::fs::read_to_string(&path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str::<files::ExportFile>(&s).map_err(|e| e.to_string()));
    match parsed {
        Ok(f) if f.format == files::FORMAT_ID => {
            let entries = f
                .playlists
                .into_iter()
                .map(|b| {
                    let target =
                        b.id.as_ref()
                            .filter(|id| app.playlist(id).is_some())
                            .cloned()
                            .or_else(|| app.playlists.iter().find(|p| p.name == b.name).map(|p| p.id.clone()));
                    RestoreEntry { backup: b, target }
                })
                .collect();
            app.open_dialog(Dialog::Restore(entries));
        }
        Ok(_) => app.error(tr.not_a_backup_file),
        Err(e) => app.error(format!("{}: {e}", tr.could_not_read_file)),
    }
}

/// Writes exported playlists; returns how many files were written.
pub fn write_export(
    app: &App,
    data: &[(Playlist, Vec<PlaylistItem>)],
    format: ExportFormat,
    dir: &Path,
    backup: bool,
) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let cache = |id: &str| app.cache.get(id);
    let pls: Vec<ExportPlaylist> = data.iter().map(|(p, items)| ExportPlaylist::new(p, items, &cache)).collect();
    let mut used = HashSet::new();
    let mut n = 0;
    for p in &pls {
        let mut stem = files::safe_file_name(&p.name);
        if !used.insert(stem.to_lowercase()) {
            stem = format!("{stem} ({})", p.id.as_deref().unwrap_or("").chars().take(8).collect::<String>());
            used.insert(stem.to_lowercase());
        }
        let path: PathBuf = dir.join(format!("{stem}.{}", format.ext()));
        std::fs::write(path, files::render(p, format))?;
        n += 1;
    }
    if format == ExportFormat::Json && (backup || pls.len() > 1) {
        std::fs::write(dir.join("all-playlists.json"), files::to_json(&files::export_file(pls)))?;
        n += 1;
    }
    Ok(n)
}
