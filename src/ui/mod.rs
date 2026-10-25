//! The egui application: state, event handling and top-level layout.

mod bottom;
mod dialogs;
mod playlists;
mod search;
pub mod style;
#[cfg(test)]
mod tests;
mod tracks;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::api::auth::AuthConfig;
use crate::api::{Album, Artist, ItemRef, Playlist, PlaylistItem, SearchResults, Track, User};
use crate::i18n::{self, Tr};
use crate::ops::analysis::SortKey;
use crate::player::{PlayerCmd, PlayerHandle};
use crate::store::{Config, OpenIn, Paths, TrackCache};
use crate::worker::{Cmd, Event, Job, JobId, Note, PreviewFail, WorkerHandle};

pub use dialogs::Dialog;

#[derive(Clone, Debug, PartialEq)]
pub enum AuthState {
    Checking,
    LoggedOut,
    LoggingIn { url: String },
    LoggedIn,
}

#[derive(Default)]
pub struct ViewState {
    pub find: String,
    pub selected: HashSet<String>,
    pub cursor: Option<usize>,
    pub anchor: Option<usize>,
    pub sort: Option<(SortKey, bool)>,
    pub scroll_to: Option<usize>,
    pub focus_row: Option<usize>,
    /// Id of the row button that had keyboard focus last frame.
    pub row_focus: Option<egui::Id>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchTab {
    Tracks,
    Albums,
    Artists,
}

pub enum SearchView {
    Results,
    Album { album: Album, tracks: Vec<Track> },
    Artist { artist: Artist, albums: Vec<Album>, tracks: Vec<Track> },
}

pub struct SearchState {
    pub query: String,
    pub results: Option<SearchResults>,
    pub tab: SearchTab,
    pub view: SearchView,
    pub selected: HashSet<String>,
    pub busy: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobKind {
    Plain,
    Login,
    Scan,
    Replacements,
    Match,
    Export,
}

pub struct JobInfo {
    pub job: Job,
    pub label: String,
    pub done: usize,
    pub total: usize,
    pub kind: JobKind,
}

pub struct LogLine {
    pub error: bool,
    pub text: String,
    pub at: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    Find,
    Search,
    PlaylistFilter,
}

pub struct App {
    pub tr: &'static Tr,
    pub cfg: Config,
    pub paths: Paths,
    pub cache: TrackCache,
    pub worker: WorkerHandle,
    pub player: PlayerHandle,
    pub auth: AuthState,
    pub user: Option<User>,
    pub playlists: Vec<Playlist>,
    pub playlists_loaded: bool,
    pub pl_filter: String,
    pub current: Option<String>,
    pub items: HashMap<String, Vec<PlaylistItem>>,
    pub loading: HashSet<String>,
    pub views: HashMap<String, ViewState>,
    pub undo: HashMap<String, Vec<Vec<ItemRef>>>,
    pub redo: HashMap<String, Vec<Vec<ItemRef>>>,
    pub search: SearchState,
    pub jobs: Vec<JobInfo>,
    pub log: Vec<LogLine>,
    pub show_log: bool,
    pub dialog: Option<Dialog>,
    pub dialog_opened: bool,
    pub focus: Option<FocusTarget>,
    pub preview_status: Option<String>,
    pub reduced_motion: bool,
    pub copy_text: Option<String>,
    applied_scale: f32,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let paths = Paths::new();
        let cfg = Config::load(&paths);
        let ctx = cc.egui_ctx.clone();
        let repaint_ctx = ctx.clone();
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || repaint_ctx.request_repaint());
        let r2 = repaint.clone();
        let player = PlayerHandle::spawn(cfg.volume, move || r2());
        let worker = WorkerHandle::spawn(paths.token_file.clone(), player.clone(), repaint);
        let reduced_motion = style::reduced_motion(&cfg);
        style::install_fonts(&ctx);
        style::apply(&ctx, &cfg, reduced_motion);
        let mut app = Self {
            tr: i18n::for_language(cfg.language),
            cache: TrackCache::load(&paths),
            worker,
            player,
            auth: AuthState::LoggedOut,
            user: None,
            playlists: Vec::new(),
            playlists_loaded: false,
            pl_filter: String::new(),
            current: cfg.last_playlist.clone(),
            items: HashMap::new(),
            loading: HashSet::new(),
            views: HashMap::new(),
            undo: HashMap::new(),
            redo: HashMap::new(),
            search: SearchState {
                query: String::new(),
                results: None,
                tab: SearchTab::Tracks,
                view: SearchView::Results,
                selected: HashSet::new(),
                busy: false,
            },
            jobs: Vec::new(),
            log: Vec::new(),
            show_log: false,
            dialog: None,
            dialog_opened: false,
            focus: None,
            preview_status: None,
            reduced_motion,
            copy_text: None,
            applied_scale: cfg.text_scale,
            cfg,
            paths,
        };
        if app.cfg.has_client() {
            app.auth = AuthState::Checking;
            app.worker.send(Cmd::RestoreSession(app.auth_config()));
        }
        app
    }

    pub fn auth_config(&self) -> AuthConfig {
        AuthConfig::new(&self.cfg.client_id, Some(self.cfg.client_secret.clone()), &self.cfg.redirect_uri)
    }

    pub fn save_cfg(&self) {
        self.cfg.save(&self.paths);
    }

    pub fn note(&mut self, text: impl Into<String>) {
        self.push_log(false, text.into());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push_log(true, text.into());
    }

    fn push_log(&mut self, error: bool, text: String) {
        let at = chrono::Local::now().format("%H:%M:%S").to_string();
        if error {
            tracing::warn!("{text}");
        }
        self.log.push(LogLine { error, text, at });
        if self.log.len() > 200 {
            self.log.remove(0);
        }
    }

    pub fn open_dialog(&mut self, d: Dialog) {
        self.dialog = Some(d);
        self.dialog_opened = true;
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn current_playlist(&self) -> Option<&Playlist> {
        self.current.as_deref().and_then(|id| self.playlist(id))
    }

    pub fn select_playlist(&mut self, id: &str) {
        self.current = Some(id.to_owned());
        self.cfg.last_playlist = Some(id.to_owned());
        self.save_cfg();
        if !self.items.contains_key(id) && !self.loading.contains(id) {
            self.load_items(id);
        }
    }

    pub fn load_items(&mut self, id: &str) {
        self.loading.insert(id.to_owned());
        self.worker.send(Cmd::LoadItems(id.to_owned()));
    }

    pub fn start_job(&mut self, label: impl Into<String>, kind: JobKind) -> Job {
        let job = Job::new();
        self.jobs.push(JobInfo { job: job.clone(), label: label.into(), done: 0, total: 0, kind });
        job
    }

    /// Remembers the playlist's current contents so the next change can be undone.
    pub fn snapshot(&mut self, pid: &str) {
        if let Some(items) = self.items.get(pid) {
            let refs = items.iter().map(|i| i.r.clone()).collect();
            let stack = self.undo.entry(pid.to_owned()).or_default();
            stack.push(refs);
            if stack.len() > 30 {
                stack.remove(0);
            }
            self.redo.remove(pid);
        }
    }

    pub fn undo_redo(&mut self, pid: &str, redo: bool) {
        let (from, to) = if redo { (&mut self.redo, &mut self.undo) } else { (&mut self.undo, &mut self.redo) };
        let Some(target) = from.get_mut(pid).and_then(Vec::pop) else { return };
        if let Some(items) = self.items.get(pid) {
            to.entry(pid.to_owned()).or_default().push(items.iter().map(|i| i.r.clone()).collect());
        }
        let label = if redo { self.tr.redo } else { self.tr.undo };
        let job = self.start_job(label, JobKind::Plain);
        self.worker.send(Cmd::Restore { pid: pid.to_owned(), target, job });
    }

    pub fn track_meta(&self, item: &PlaylistItem) -> Option<Track> {
        item.track.clone().or_else(|| self.cache.get(&item.r.id))
    }

    pub fn open_in_tidal(&mut self, app_url: String, web_url: String) {
        let r = match self.cfg.open_in {
            OpenIn::App => open::that_detached(&app_url).or_else(|_| open::that_detached(&web_url)),
            OpenIn::Web => open::that_detached(&web_url),
        };
        if let Err(e) = r {
            self.error(format!("{}: {e}", self.tr.could_not_open));
        }
    }

    pub fn play(&mut self, t: &Track) {
        self.worker.send(Cmd::Preview(t.clone()));
    }

    /// Adds tracks to a playlist (with undo snapshot and optimistic job entry).
    pub fn add_to_playlist(&mut self, pid: &str, refs: Vec<ItemRef>, skip_dups: bool) {
        if refs.is_empty() {
            return;
        }
        self.snapshot(pid);
        let name = self.playlist(pid).map(|p| p.name.clone()).unwrap_or_default();
        let job = self.start_job(i18n::fmt(self.tr.adding_to, &[&refs.len().to_string(), &name]), JobKind::Plain);
        self.worker.send(Cmd::AddItems { pid: pid.to_owned(), refs, before: None, skip_dups, job });
    }

    pub fn remove_from_playlist(&mut self, pid: &str, item_ids: &HashSet<String>) {
        let Some(items) = self.items.get(pid) else { return };
        let victims: Vec<(ItemRef, String)> =
            items.iter().filter(|i| item_ids.contains(&i.item_id)).map(|i| (i.r.clone(), i.item_id.clone())).collect();
        if victims.is_empty() {
            return;
        }
        self.snapshot(pid);
        if let Some(list) = self.items.get_mut(pid) {
            list.retain(|i| !item_ids.contains(&i.item_id));
        }
        if let Some(v) = self.views.get_mut(pid) {
            v.selected.clear();
        }
        let job = self.start_job(i18n::fmt(self.tr.removing_n, &[&victims.len().to_string()]), JobKind::Plain);
        self.worker.send(Cmd::RemoveItems { pid: pid.to_owned(), items: victims, job });
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.worker.events.try_recv() {
            self.handle_event(ctx, ev);
        }
        self.cache.save_if_dirty(&self.paths);
    }

    fn finish_job(&mut self, id: JobId) -> Option<JobInfo> {
        let pos = self.jobs.iter().position(|j| j.job.id == id)?;
        Some(self.jobs.remove(pos))
    }

    fn handle_event(&mut self, ctx: &egui::Context, ev: Event) {
        let tr = self.tr;
        match ev {
            Event::LoginStarted(url) => self.auth = AuthState::LoggingIn { url },
            Event::LoggedIn(user) => {
                self.jobs.retain(|j| j.kind != JobKind::Login);
                self.note(i18n::fmt(tr.logged_in_as, &[&user.username]));
                self.user = Some(user);
                self.auth = AuthState::LoggedIn;
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Event::LoginFailed(e) => {
                self.jobs.retain(|j| j.kind != JobKind::Login);
                self.auth = AuthState::LoggedOut;
                self.error(format!("{}: {e}", tr.login_failed));
            }
            Event::LoggedOut => {
                self.jobs.retain(|j| j.kind != JobKind::Login);
                self.auth = AuthState::LoggedOut;
                self.user = None;
                self.playlists.clear();
                self.items.clear();
                self.playlists_loaded = false;
            }
            Event::Playlists(p) => {
                self.playlists = p;
                self.playlists_loaded = true;
                match self.current.clone() {
                    Some(id) if self.playlist(&id).is_some() => self.select_playlist(&id),
                    _ => {
                        self.current = None;
                        if let Some(first) = self.playlists.first().map(|p| p.id.clone()) {
                            self.select_playlist(&first);
                        }
                    }
                }
            }
            Event::Items { pid, items } => {
                self.loading.remove(&pid);
                self.cache.remember(items.iter().filter_map(|i| i.track.as_ref()));
                if let Some(v) = self.views.get_mut(&pid) {
                    let ids: HashSet<&String> = items.iter().map(|i| &i.item_id).collect();
                    v.selected.retain(|s| ids.contains(s));
                    if v.cursor.is_some_and(|c| c >= items.len()) {
                        v.cursor = items.len().checked_sub(1);
                    }
                }
                if let Some(p) = self.playlists.iter_mut().find(|p| p.id == pid) {
                    p.number_of_items = Some(items.len() as u32);
                }
                self.items.insert(pid, items);
            }
            Event::ItemsFailed { pid, error } => {
                self.loading.remove(&pid);
                self.error(format!("{}: {error}", tr.could_not_load_items));
            }
            Event::PlaylistCreated(p) => {
                let id = p.id.clone();
                self.playlists.insert(0, p);
                self.select_playlist(&id);
            }
            Event::PlaylistUpdated(p) => {
                if let Some(old) = self.playlists.iter_mut().find(|o| o.id == p.id) {
                    let owned = old.owned;
                    *old = p;
                    old.owned = owned;
                }
            }
            Event::PlaylistDeleted(pid) => {
                self.playlists.retain(|p| p.id != pid);
                self.items.remove(&pid);
                if self.current.as_deref() == Some(&pid) {
                    self.current = None;
                    if let Some(first) = self.playlists.first().map(|p| p.id.clone()) {
                        self.select_playlist(&first);
                    }
                }
            }
            Event::Search(r) => {
                self.search.busy = false;
                self.cache.remember(r.tracks.iter());
                self.search.results = Some(r);
                self.search.view = SearchView::Results;
                self.search.selected.clear();
            }
            Event::SearchMore { tracks, next } => {
                self.search.busy = false;
                self.cache.remember(tracks.iter());
                if let Some(r) = &mut self.search.results {
                    let have: HashSet<String> = r.tracks.iter().map(|t| t.id.clone()).collect();
                    r.tracks.extend(tracks.into_iter().filter(|t| !have.contains(&t.id)));
                    r.tracks_next = next;
                }
            }
            Event::Album { album, tracks } => {
                self.search.busy = false;
                self.cache.remember(tracks.iter());
                self.search.selected.clear();
                self.search.view = SearchView::Album { album, tracks };
            }
            Event::Artist { artist, albums, tracks } => {
                self.search.busy = false;
                self.cache.remember(tracks.iter());
                self.search.selected.clear();
                self.search.view = SearchView::Artist { artist, albums, tracks };
            }
            Event::Progress { job, done, total } => {
                if let Some(j) = self.jobs.iter_mut().find(|j| j.job.id == job) {
                    j.done = done;
                    j.total = total;
                }
            }
            Event::JobDone { job, note } => {
                self.finish_job(job);
                if let Some(n) = note {
                    let text = self.note_text(&n);
                    self.note(text);
                }
            }
            Event::JobFailed { job, error } => {
                let info = self.finish_job(job);
                let label = info.map(|j| j.label).unwrap_or_default();
                if error == "cancelled" {
                    self.note(format!("{label}: {}", tr.cancelled));
                } else {
                    self.error(format!("{label}: {error}"));
                }
            }
            Event::Unavailable(hits) => {
                let hits = hits
                    .into_iter()
                    .map(|mut h| {
                        if h.known.is_none() {
                            h.known = self.cache.get(&h.item.r.id);
                        }
                        h
                    })
                    .collect();
                self.open_dialog(Dialog::unavailable_results(hits));
            }
            Event::Replacements(list) => {
                if let Some(Dialog::Unavailable(u)) = &mut self.dialog {
                    u.set_replacements(list);
                }
            }
            Event::Matches(playlists) => {
                self.open_dialog(Dialog::import_review(playlists, &self.playlists));
            }
            Event::ExportData { data, format, dir, backup } => {
                let r = dialogs::write_export(self, &data, format, &dir, backup);
                match r {
                    Ok(n) => {
                        let text = i18n::fmt(tr.exported_n_to, &[&n.to_string(), &dir.display().to_string()]);
                        self.note(text);
                        if backup {
                            let _ = open::that_detached(&dir);
                        }
                    }
                    Err(e) => self.error(format!("{}: {e}", tr.export_failed)),
                }
            }
            Event::Note(n) => {
                let text = self.note_text(&n);
                self.note(text);
            }
            Event::Error(e) => self.error(e),
            Event::PreviewLoading(title) => self.preview_status = Some(i18n::fmt(tr.loading_preview, &[&title])),
            Event::PreviewFailed(f) => {
                self.preview_status = None;
                let text = match f {
                    PreviewFail::Drm => tr.preview_drm.to_owned(),
                    PreviewFail::Video => tr.preview_video.to_owned(),
                    PreviewFail::Other(e) => format!("{}: {e}", tr.preview_failed),
                };
                self.error(text);
            }
        }
    }

    pub fn note_text(&self, n: &Note) -> String {
        let tr = self.tr;
        match n {
            Note::Added { n, skipped } if *skipped > 0 => {
                i18n::fmt(tr.added_n_skipped, &[&n.to_string(), &skipped.to_string()])
            }
            Note::Added { n, .. } => i18n::fmt(tr.added_n, &[&n.to_string()]),
            Note::Removed(n) => i18n::fmt(tr.removed_n, &[&n.to_string()]),
            Note::Reordered => tr.reordered.into(),
            Note::Created(name) => i18n::fmt(tr.created_playlist, &[name]),
            Note::Duplicated(name) => i18n::fmt(tr.duplicated_as, &[name]),
            Note::Merged { name, n } => i18n::fmt(tr.merged_into, &[&n.to_string(), name]),
            Note::Updated => tr.playlist_saved.into(),
            Note::Deleted => tr.playlist_deleted.into(),
            Note::Restored => tr.restored.into(),
            Note::Replaced(n) => i18n::fmt(tr.replaced_n, &[&n.to_string()]),
            Note::Favourited { n, add: true } => i18n::fmt(tr.favourited_n, &[&n.to_string()]),
            Note::Favourited { n, add: false } => i18n::fmt(tr.unfavourited_n, &[&n.to_string()]),
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.text_edit_focused();
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if self.dialog.is_some() || self.auth != AuthState::LoggedIn {
            return;
        }
        if pressed(cmd(Key::F)) {
            self.focus = Some(FocusTarget::Find);
        }
        if pressed(cmd(Key::K)) || pressed(cmd(Key::L)) {
            self.cfg.show_search = true;
            self.focus = Some(FocusTarget::Search);
        }
        if pressed(cmd(Key::G)) {
            self.focus = Some(FocusTarget::PlaylistFilter);
        }
        if pressed(cmd(Key::N)) {
            self.open_dialog(Dialog::new_playlist(Vec::new()));
        }
        if pressed(cmd(Key::I)) {
            dialogs::start_import(self);
        }
        if pressed(cmd(Key::E)) {
            let sel = self.current.iter().cloned().collect();
            self.open_dialog(Dialog::export(sel));
        }
        if pressed(KeyboardShortcut::new(Modifiers::NONE, Key::F5)) {
            if let Some(pid) = self.current.clone() {
                self.load_items(&pid);
            }
            self.worker.send(Cmd::LoadPlaylists);
        }
        if pressed(KeyboardShortcut::new(Modifiers::NONE, Key::F1)) {
            self.open_dialog(Dialog::Shortcuts);
        }
        if let Some(pid) = self.current.clone() {
            if pressed(cmd(Key::D))
                && let Some(p) = self.playlist(&pid).cloned()
            {
                self.open_dialog(Dialog::duplicate(&p, self.tr));
            }
            if !typing {
                if pressed(cmd(Key::Z)) {
                    self.undo_redo(&pid, false);
                }
                if pressed(cmd(Key::Y)) || pressed(KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
                {
                    self.undo_redo(&pid, true);
                }
            }
        }
        if !typing && pressed(KeyboardShortcut::new(Modifiers::COMMAND, Key::Space)) {
            self.player.send(PlayerCmd::TogglePause);
        }
        // Text size (persisted). eframe's own zoom keys also work; we save whatever results.
        let z = ctx.zoom_factor();
        if (z - self.applied_scale).abs() > 0.001 {
            self.applied_scale = z;
            self.cfg.text_scale = z;
            self.save_cfg();
        }
    }

    pub fn set_text_scale(&mut self, ctx: &egui::Context, s: f32) {
        let s = (s * 10.0).round() / 10.0;
        let s = s.clamp(0.5, 3.0);
        self.cfg.text_scale = s;
        self.applied_scale = s;
        ctx.set_zoom_factor(s);
        self.save_cfg();
    }

    pub fn restyle(&mut self, ctx: &egui::Context) {
        self.tr = i18n::for_language(self.cfg.language);
        self.reduced_motion = style::reduced_motion(&self.cfg);
        style::apply(ctx, &self.cfg, self.reduced_motion);
        self.save_cfg();
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let tr = self.tr;
        let logged_in = self.auth == AuthState::LoggedIn;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr.menu_file, |ui| {
                if ui.add_enabled(logged_in, egui::Button::new(tr.import_dots).shortcut_text("Ctrl+I")).clicked() {
                    dialogs::start_import(self);
                }
                if ui.add_enabled(logged_in, egui::Button::new(tr.export_dots).shortcut_text("Ctrl+E")).clicked() {
                    let sel = self.current.iter().cloned().collect();
                    self.open_dialog(Dialog::export(sel));
                }
                if ui.add_enabled(logged_in && !self.playlists.is_empty(), egui::Button::new(tr.backup_all)).clicked() {
                    dialogs::start_backup(self);
                }
                if ui.add_enabled(logged_in, egui::Button::new(tr.restore_backup_dots)).clicked() {
                    dialogs::start_restore(self);
                }
                ui.separator();
                if ui.button(tr.settings_dots).clicked() {
                    self.open_dialog(Dialog::settings(&self.cfg));
                }
                if logged_in && ui.button(tr.log_out).clicked() {
                    self.worker.send(Cmd::Logout);
                }
                ui.separator();
                if ui.button(tr.quit).clicked() {
                    ui.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            let cur = self.current_playlist().cloned();
            ui.menu_button(tr.menu_playlist, |ui| {
                if ui.add_enabled(logged_in, egui::Button::new(tr.new_playlist_dots).shortcut_text("Ctrl+N")).clicked()
                {
                    self.open_dialog(Dialog::new_playlist(Vec::new()));
                }
                let has = cur.is_some();
                if ui.add_enabled(has, egui::Button::new(tr.edit_details_dots)).clicked()
                    && let Some(p) = &cur
                {
                    self.open_dialog(Dialog::edit(p));
                }
                if ui.add_enabled(has, egui::Button::new(tr.duplicate_dots).shortcut_text("Ctrl+D")).clicked()
                    && let Some(p) = &cur
                {
                    self.open_dialog(Dialog::duplicate(p, tr));
                }
                if ui.add_enabled(logged_in && self.playlists.len() > 1, egui::Button::new(tr.merge_dots)).clicked() {
                    self.open_dialog(Dialog::merge(self.current.iter().cloned().collect(), tr));
                }
                if ui.add_enabled(has, egui::Button::new(tr.sort_permanently_dots)).clicked()
                    && let Some(p) = &cur
                {
                    self.open_dialog(Dialog::sort(&p.id));
                }
                if ui.add_enabled(has, egui::Button::new(tr.shuffle_order)).clicked()
                    && let Some(p) = &cur
                {
                    tracks::shuffle(self, &p.id);
                }
                if ui.add_enabled(has, egui::Button::new(tr.statistics)).clicked()
                    && let Some(p) = &cur
                {
                    self.open_dialog(Dialog::Stats(p.id.clone()));
                }
                if ui.add_enabled(has, egui::Button::new(tr.open_in_tidal)).clicked()
                    && let Some(p) = &cur
                {
                    self.open_in_tidal(p.app_url(), p.web_url());
                }
                if ui.add_enabled(has, egui::Button::new(tr.refresh).shortcut_text("F5")).clicked()
                    && let Some(p) = &cur
                {
                    self.load_items(&p.id);
                }
                ui.separator();
                if ui
                    .add_enabled(cur.as_ref().is_some_and(|p| p.owned), egui::Button::new(tr.delete_playlist_dots))
                    .clicked()
                    && let Some(p) = &cur
                {
                    self.open_dialog(Dialog::ConfirmDelete { pid: p.id.clone(), name: p.name.clone() });
                }
            });
            ui.menu_button(tr.menu_tools, |ui| {
                if ui.add_enabled(logged_in, egui::Button::new(tr.find_unavailable_dots)).clicked() {
                    self.open_dialog(Dialog::unavailable_setup(self.current.clone(), self.cfg.strict_availability));
                }
                if ui.add_enabled(cur.is_some(), egui::Button::new(tr.find_duplicates_dots)).clicked()
                    && let Some(p) = &cur
                {
                    let d = Dialog::duplicates(self, &p.id);
                    self.open_dialog(d);
                }
            });
            ui.menu_button(tr.menu_view, |ui| {
                if ui.checkbox(&mut self.cfg.show_search, tr.show_search_panel).changed() {
                    self.save_cfg();
                }
                if ui.button(tr.text_bigger).clicked() {
                    let s = self.cfg.text_scale + 0.1;
                    self.set_text_scale(ui.ctx(), s);
                }
                if ui.button(tr.text_smaller).clicked() {
                    let s = self.cfg.text_scale - 0.1;
                    self.set_text_scale(ui.ctx(), s);
                }
                if ui.button(tr.text_reset).clicked() {
                    self.set_text_scale(ui.ctx(), 1.0);
                }
                if ui.checkbox(&mut self.show_log, tr.show_log).changed() {}
            });
            ui.menu_button(tr.menu_help, |ui| {
                if ui.add(egui::Button::new(tr.keyboard_shortcuts).shortcut_text("F1")).clicked() {
                    self.open_dialog(Dialog::Shortcuts);
                }
                if ui.button(tr.accessibility_statement).clicked() {
                    self.open_dialog(Dialog::Accessibility);
                }
                if ui.button(tr.about).clicked() {
                    self.open_dialog(Dialog::About);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(u) = &self.user {
                    ui.label(i18n::fmt(tr.signed_in_as_short, &[&u.username, &u.country]));
                }
            });
        });
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let tr = self.tr;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_max_width(720.0);
            ui.heading(tr.app_title);
            ui.label(tr.welcome_text);
            ui.add_space(12.0);
            match self.auth.clone() {
                AuthState::Checking => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr.checking_session);
                    });
                }
                AuthState::LoggingIn { url } => {
                    ui.label(tr.waiting_for_browser);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(tr.open_login_page_again).clicked() {
                            let _ = open::that_detached(&url);
                        }
                        if ui.button(tr.copy_login_link).clicked() {
                            ui.ctx().copy_text(url.clone());
                            self.note(tr.link_copied);
                        }
                        if ui.button(tr.cancel).clicked() {
                            for j in self.jobs.iter().filter(|j| j.kind == JobKind::Login) {
                                j.job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    });
                }
                AuthState::LoggedOut | AuthState::LoggedIn => {
                    if !self.cfg.has_client() {
                        ui.label(tr.need_client_id);
                    }
                    ui.horizontal_wrapped(|ui| {
                        let login = ui.add_enabled(self.cfg.has_client(), egui::Button::new(tr.log_in_with_tidal));
                        if self.dialog.is_none() && self.cfg.has_client() && !ui.ctx().memory(|m| m.focused().is_some())
                        {
                            login.request_focus();
                        }
                        if login.clicked() {
                            let job = self.start_job(tr.log_in_with_tidal, JobKind::Login);
                            self.worker.send(Cmd::Login(self.auth_config(), job));
                        }
                        if ui.button(tr.api_settings_dots).clicked() {
                            self.open_dialog(Dialog::settings(&self.cfg));
                        }
                    });
                }
            }
            ui.add_space(16.0);
            ui.collapsing(tr.how_to_get_client_id, |ui| {
                ui.label(i18n::fmt(tr.client_id_steps, &[&self.cfg.redirect_uri]));
                ui.hyperlink_to(tr.developer_portal, "https://developer.tidal.com/dashboard");
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_events(&ctx);
        self.shortcuts(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| bottom::show(self, ui));
        if self.auth == AuthState::LoggedIn {
            // Side panels scale with the window so the song table keeps room at large text sizes.
            let w = ctx.content_rect().width();
            egui::Panel::left("playlists")
                .resizable(true)
                .default_size(w * 0.18)
                .size_range(140.0..=(w * 0.22).max(160.0))
                .show(ui, |ui| playlists::show(self, ui));
            if self.cfg.show_search {
                egui::Panel::right("search")
                    .resizable(true)
                    .default_size(w * 0.26)
                    .size_range(200.0..=(w * 0.3).max(220.0))
                    .show(ui, |ui| search::show(self, ui));
            }
            egui::CentralPanel::default_margins().show(ui, |ui| tracks::show(self, ui));
        } else {
            egui::CentralPanel::default_margins().show(ui, |ui| self.welcome(ui));
        }
        dialogs::show(self, &ctx);
        if let Some(t) = self.copy_text.take() {
            ctx.copy_text(t);
            self.note(self.tr.link_copied);
        }
        style::paint_focus_ring(&ctx);
        if !self.jobs.is_empty() || self.search.busy || !self.loading.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cache.save_if_dirty(&self.paths);
        self.save_cfg();
    }
}
