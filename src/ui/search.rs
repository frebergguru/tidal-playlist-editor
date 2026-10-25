//! Right panel: find new songs in the TIDAL catalogue and add them.

use std::collections::HashSet;

use egui::{RichText, Role, WidgetInfo};

use super::{App, Dialog, FocusTarget, SearchTab, SearchView};
use crate::api::{Album, Artist, ItemRef, Track, format_duration};
use crate::i18n;
use crate::worker::Cmd;

enum Act {
    Search,
    More(String),
    Play(Track),
    Open(Track),
    AddHere(Vec<ItemRef>),
    AddTo(Vec<ItemRef>),
    OpenAlbum(Album),
    OpenArtist(Artist),
    Back,
    Toggle(String),
    SelectAll(Vec<String>),
    Fav(Vec<String>),
    Radio(Artist),
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let tr = app.tr;
    let mut acts = Vec::new();
    ui.add_space(4.0);
    ui.heading(tr.find_new_songs);
    let label = ui.label(tr.search_tidal);
    ui.horizontal(|ui| {
        let button_w = ui.spacing().interact_size.x + 40.0;
        let field = ui
            .add(
                egui::TextEdit::singleline(&mut app.search.query)
                    .hint_text(tr.search_hint)
                    .desired_width((ui.available_width() - button_w).max(80.0)),
            )
            .labelled_by(label.id);
        if app.focus == Some(FocusTarget::Search) {
            field.request_focus();
            app.focus = None;
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.add_enabled(!app.search.query.trim().is_empty(), egui::Button::new(tr.search)).clicked() || enter {
            acts.push(Act::Search);
        }
    });
    let target = app.current_playlist().map(|p| p.name.clone());
    match &target {
        Some(name) => ui.label(i18n::fmt(tr.adds_go_to, &[name])),
        None => ui.label(tr.select_playlist_to_add),
    };
    if app.search.busy {
        ui.horizontal(|ui| {
            super::style::busy(ui, app.reduced_motion);
            ui.label(tr.searching);
        });
    }
    ui.separator();

    let in_playlist: HashSet<String> = app
        .current
        .as_ref()
        .and_then(|p| app.items.get(p))
        .map(|v| v.iter().map(|i| i.r.id.clone()).collect())
        .unwrap_or_default();
    let has_target = target.is_some();
    let selected = app.search.selected.clone();

    match &app.search.view {
        SearchView::Album { album, tracks } => {
            if ui.button(format!("← {}", tr.back)).clicked() {
                acts.push(Act::Back);
            }
            ui.strong(format!("{} — {}", album.title, album.artists.join(", ")));
            if let Some(d) = &album.release_date {
                ui.label(d);
            }
            track_list(ui, tr, tracks, &in_playlist, &selected, has_target, &mut acts, "album");
        }
        SearchView::Artist { artist, albums, tracks } => {
            if ui.button(format!("← {}", tr.back)).clicked() {
                acts.push(Act::Back);
            }
            ui.strong(&artist.name);
            if ui.button(tr.artist_radio_playlist).clicked() {
                acts.push(Act::Radio(artist.clone()));
            }
            egui::ScrollArea::vertical().id_salt("artist-scroll").auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new(tr.top_tracks).strong());
                track_rows(ui, tr, tracks, &in_playlist, &selected, has_target, &mut acts);
                ui.add_space(8.0);
                ui.label(RichText::new(tr.albums).strong());
                album_rows(ui, tr, albums, &mut acts);
            });
        }
        SearchView::Results => match &app.search.results {
            None => {
                ui.label(tr.search_intro);
            }
            Some(r) => {
                if let Some(d) = &r.did_you_mean
                    && ui.link(i18n::fmt(tr.did_you_mean, &[d])).clicked()
                {
                    app.search.query = d.clone();
                    acts.push(Act::Search);
                }
                ui.horizontal(|ui| {
                    for (tab, name, n) in [
                        (SearchTab::Tracks, tr.tracks, r.tracks.len()),
                        (SearchTab::Albums, tr.albums, r.albums.len()),
                        (SearchTab::Artists, tr.artists, r.artists.len()),
                    ] {
                        let sel = app.search.tab == tab;
                        let resp = ui.add(egui::Button::selectable(sel, format!("{name} ({n})")));
                        resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, sel, format!("{name} ({n})")));
                        if resp.clicked() {
                            app.search.tab = tab;
                        }
                    }
                });
                match app.search.tab {
                    SearchTab::Tracks => {
                        track_list(ui, tr, &r.tracks, &in_playlist, &selected, has_target, &mut acts, "results");
                        if let Some(next) = &r.tracks_next
                            && ui.button(tr.load_more).clicked()
                        {
                            acts.push(Act::More(next.clone()));
                        }
                    }
                    SearchTab::Albums => {
                        egui::ScrollArea::vertical().id_salt("album-scroll").auto_shrink([false, false]).show(
                            ui,
                            |ui| {
                                album_rows(ui, tr, &r.albums, &mut acts);
                            },
                        );
                    }
                    SearchTab::Artists => {
                        egui::ScrollArea::vertical().id_salt("artist-list").auto_shrink([false, false]).show(
                            ui,
                            |ui| {
                                for a in &r.artists {
                                    ui.horizontal(|ui| {
                                        ui.label(&a.name);
                                        if ui
                                            .button(tr.open)
                                            .on_hover_text(i18n::fmt(tr.open_artist, &[&a.name]))
                                            .clicked()
                                        {
                                            acts.push(Act::OpenArtist(a.clone()));
                                        }
                                    });
                                }
                            },
                        );
                    }
                }
            }
        },
    }

    for a in acts {
        match a {
            Act::Search => {
                let q = app.search.query.trim().to_owned();
                if !q.is_empty() {
                    app.search.busy = true;
                    app.search.tab = SearchTab::Tracks;
                    app.worker.send(Cmd::Search(q));
                }
            }
            Act::More(next) => {
                app.search.busy = true;
                app.worker.send(Cmd::SearchMore(next));
            }
            Act::Play(t) => app.play(&t),
            Act::Open(t) => app.open_in_tidal(t.app_url(), t.web_url()),
            Act::AddHere(refs) => {
                if let Some(pid) = app.current.clone() {
                    app.add_to_playlist(&pid, refs, false);
                    app.search.selected.clear();
                }
            }
            Act::AddTo(refs) => app.open_dialog(Dialog::add_to(refs, None, None)),
            Act::OpenAlbum(al) => {
                app.search.busy = true;
                app.worker.send(Cmd::AlbumTracks(al));
            }
            Act::OpenArtist(ar) => {
                app.search.busy = true;
                app.worker.send(Cmd::ArtistDetails(ar));
            }
            Act::Radio(a) => app.save_radio(crate::api::RadioSeed::Artist(a.id), &a.name),
            Act::Back => {
                app.search.view = SearchView::Results;
                app.search.selected.clear();
            }
            Act::Toggle(id) => {
                if !app.search.selected.remove(&id) {
                    app.search.selected.insert(id);
                }
            }
            Act::SelectAll(ids) => app.search.selected = ids.into_iter().collect(),
            Act::Fav(ids) => app.worker.send(Cmd::Favourite { ids, add: true }),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn track_list(
    ui: &mut egui::Ui,
    tr: &'static crate::i18n::Tr,
    tracks: &[Track],
    in_playlist: &HashSet<String>,
    selected: &HashSet<String>,
    has_target: bool,
    acts: &mut Vec<Act>,
    salt: &str,
) {
    let chosen: Vec<ItemRef> = tracks.iter().filter(|t| selected.contains(&t.id)).map(Track::item_ref).collect();
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr.select_all).clicked() {
            acts.push(Act::SelectAll(tracks.iter().map(|t| t.id.clone()).collect()));
        }
        let n = chosen.len();
        if ui.add_enabled(has_target && n > 0, egui::Button::new(i18n::fmt(tr.add_n_here, &[&n.to_string()]))).clicked()
        {
            acts.push(Act::AddHere(chosen.clone()));
        }
        if ui.add_enabled(n > 0, egui::Button::new(tr.add_to_dots)).clicked() {
            acts.push(Act::AddTo(chosen.clone()));
        }
        if ui.add_enabled(has_target && !tracks.is_empty(), egui::Button::new(tr.add_all_here)).clicked() {
            acts.push(Act::AddHere(tracks.iter().map(Track::item_ref).collect()));
        }
        if ui.add_enabled(n > 0, egui::Button::new(tr.add_to_favourites)).clicked() {
            acts.push(Act::Fav(chosen.iter().map(|r| r.id.clone()).collect()));
        }
    });
    egui::ScrollArea::vertical().id_salt(("tracks-scroll", salt)).auto_shrink([false, false]).show(ui, |ui| {
        track_rows(ui, tr, tracks, in_playlist, selected, has_target, acts);
    });
}

fn track_rows(
    ui: &mut egui::Ui,
    tr: &'static crate::i18n::Tr,
    tracks: &[Track],
    in_playlist: &HashSet<String>,
    selected: &HashSet<String>,
    has_target: bool,
    acts: &mut Vec<Act>,
) {
    for t in tracks {
        ui.push_id(("st", &t.id), |ui| {
            ui.horizontal(|ui| {
                let mut on = selected.contains(&t.id);
                let mut desc = format!("{} — {}", t.full_title(), t.artist_line());
                if let Some(a) = &t.album {
                    desc.push_str(&format!(" ({a})"));
                }
                desc.push_str(&format!(" · {}", format_duration(t.duration_secs)));
                if t.explicit {
                    desc.push_str(&format!(" · {}", tr.explicit));
                }
                if in_playlist.contains(&t.id) {
                    desc.push_str(&format!(" · ✓ {}", tr.already_in_playlist));
                }
                if !t.streamable() {
                    desc.push_str(&format!(" · ⚠ {}", tr.status_unavailable));
                }
                // Icon-only controls get full text labels for screen readers.
                let cb = ui.checkbox(&mut on, "").on_hover_text(tr.select);
                cb.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, on, format!("{}: {desc}", tr.select)));
                if cb.changed() {
                    acts.push(Act::Toggle(t.id.clone()));
                }
                let play = ui.button("▶").on_hover_text(tr.play_preview);
                play.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, format!("{}: {}", tr.play_preview, t.full_title()))
                });
                if play.clicked() {
                    acts.push(Act::Play(t.clone()));
                }
                let add = ui.add_enabled(has_target, egui::Button::new(tr.add));
                add.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, has_target, format!("{}: {}", tr.add, t.full_title()))
                });
                if add.clicked() {
                    acts.push(Act::AddHere(vec![t.item_ref()]));
                }
                let open = ui.button("↗").on_hover_text(tr.open_in_tidal);
                open.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, format!("{}: {}", tr.open_in_tidal, t.full_title()))
                });
                if open.clicked() {
                    acts.push(Act::Open(t.clone()));
                }
                ui.add(egui::Label::new(desc).wrap());
            });
        });
    }
}

fn album_rows(ui: &mut egui::Ui, tr: &'static crate::i18n::Tr, albums: &[Album], acts: &mut Vec<Act>) {
    for a in albums {
        ui.push_id(("al", &a.id), |ui| {
            ui.horizontal(|ui| {
                let open = ui.button(tr.open);
                open.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("{}: {}", tr.open, a.title)));
                if open.clicked() {
                    acts.push(Act::OpenAlbum(a.clone()));
                }
                let mut desc = format!("{} — {}", a.title, a.artists.join(", "));
                if let Some(d) = &a.release_date {
                    desc.push_str(&format!(" · {}", d.chars().take(4).collect::<String>()));
                }
                if let Some(n) = a.number_of_items {
                    desc.push_str(&format!(" · {}", i18n::fmt(tr.n_items, &[&n.to_string()])));
                }
                ui.add(egui::Label::new(desc).wrap());
            });
        });
    }
}
