//! Central panel: the selected playlist's items.

use std::collections::HashSet;

use egui::{Key, Modifiers, RichText, Role, WidgetInfo};
use egui_extras::{Column, TableBuilder};

use super::{App, Dialog, FocusTarget, JobKind};
use crate::api::{ItemRef, PlaylistItem, Track, format_duration};
use crate::i18n;
use crate::ops::analysis::{self, SortKey};
use crate::worker::Cmd;

enum RowAction {
    Click { idx: usize, toggle: bool, range: bool },
    Focus(usize),
    Cursor(usize),
    Play(usize),
    Open(usize),
    Remove(HashSet<String>),
    MoveSelection { up: bool },
    DropBefore { ids: Vec<String>, before: usize },
    CopyTo(Vec<ItemRef>),
    MoveTo(HashSet<String>),
    Favourite(Vec<String>),
    Similar(HashSet<String>),
    CopyLink(usize),
    Radio(usize),
    WhereIs(usize),
}

/// Indices into the playlist that are shown, after find filter and view sort.
fn visible(
    items: &[PlaylistItem],
    view: &super::ViewState,
    cache: &dyn Fn(&PlaylistItem) -> Option<Track>,
) -> Vec<usize> {
    let needle = view.find.trim().to_lowercase();
    let mut idx: Vec<usize> = (0..items.len())
        .filter(|&i| {
            needle.is_empty()
                || cache(&items[i]).is_some_and(|t| {
                    let hay = format!(
                        "{} {} {} {}",
                        t.full_title(),
                        t.artist_line(),
                        t.album.as_deref().unwrap_or(""),
                        t.isrc.as_deref().unwrap_or("")
                    )
                    .to_lowercase();
                    needle.split_whitespace().all(|w| hay.contains(w))
                })
                || items[i].r.id == needle
        })
        .collect();
    if let Some((key, asc)) = view.sort {
        let sorted = analysis::sorted(items, key, asc);
        let pos: std::collections::HashMap<&str, usize> =
            sorted.iter().enumerate().map(|(n, i)| (i.item_id.as_str(), n)).collect();
        idx.sort_by_key(|&i| pos.get(items[i].item_id.as_str()).copied().unwrap_or(usize::MAX));
    }
    idx
}

pub fn shuffle(app: &mut App, pid: &str) {
    let Some(items) = app.items.get(pid).cloned() else { return };
    let order = analysis::shuffled(&items);
    apply_order(app, pid, items, order, app.tr.shuffle_order);
}

pub fn apply_order(app: &mut App, pid: &str, current: Vec<PlaylistItem>, order: Vec<PlaylistItem>, label: &str) {
    app.snapshot(pid);
    app.items.insert(pid.to_owned(), order.clone());
    let job = app.start_job(label, JobKind::Plain);
    app.worker.send(Cmd::ApplyOrder { pid: pid.to_owned(), current, order, job });
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let tr = app.tr;
    let Some(pid) = app.current.clone() else {
        ui.label(tr.select_a_playlist);
        return;
    };
    let Some(pl) = app.playlist(&pid).cloned() else {
        ui.label(tr.select_a_playlist);
        return;
    };
    app.views.entry(pid.clone()).or_default();

    // ------------------------------------------------------------ header
    ui.heading(&pl.name);
    if !pl.description.is_empty() {
        ui.add(egui::Label::new(&pl.description).wrap());
    }
    let items_len = app.items.get(&pid).map(Vec::len);
    let total_secs: u32 =
        app.items.get(&pid).map_or(0, |v| v.iter().filter_map(|i| i.track.as_ref()).map(|t| t.duration_secs).sum());
    let unavailable = app.items.get(&pid).map_or(0, |v| v.iter().filter(|i| !i.available()).count());
    ui.horizontal_wrapped(|ui| {
        let n =
            items_len.or(pl.number_of_items.map(|n| n as usize)).map(|n| n.to_string()).unwrap_or_else(|| "…".into());
        ui.label(i18n::fmt(tr.n_items, &[&n]));
        if total_secs > 0 {
            ui.label(format!("· {}", format_duration(total_secs)));
        }
        ui.label(format!("· {}", i18n::access_label(tr, pl.access_type)));
        if unavailable > 0 {
            ui.label(
                RichText::new(format!("· ⚠ {}", i18n::fmt(tr.n_unavailable, &[&unavailable.to_string()]))).strong(),
            );
        }
        if !pl.owned {
            ui.label(format!("· {}", tr.collaborator));
        }
    });

    // ------------------------------------------------------------ toolbar
    let mut actions: Vec<RowAction> = Vec::new();
    let selected_ids: HashSet<String> = app.views[&pid].selected.clone();
    let cursor = app.views[&pid].cursor;
    let sel_count = selected_ids.len();
    ui.horizontal_wrapped(|ui| {
        let label = ui.label(tr.find_in_playlist);
        let view = app.views.get_mut(&pid).expect("view");
        let find = ui
            .add(egui::TextEdit::singleline(&mut view.find).hint_text(tr.find_hint).desired_width(220.0))
            .labelled_by(label.id);
        if app.focus == Some(FocusTarget::Find) {
            find.request_focus();
            app.focus = None;
        }
        if !view.find.is_empty() && ui.button(tr.clear).clicked() {
            view.find.clear();
        }
        let sort_text = match view.sort {
            None => tr.order_playlist.to_owned(),
            Some((k, asc)) => format!("{} {}", i18n::sort_label(tr, k), if asc { "↑" } else { "↓" }),
        };
        let order_label = ui.label(tr.view_order);
        let combo = egui::ComboBox::from_id_salt("view-order").selected_text(sort_text).show_ui(ui, |ui| {
            ui.selectable_value(&mut view.sort, None, tr.order_playlist);
            for k in [SortKey::Title, SortKey::Artist, SortKey::Album, SortKey::Duration, SortKey::Added] {
                for asc in [true, false] {
                    let t = format!("{} {}", i18n::sort_label(tr, k), if asc { "↑" } else { "↓" });
                    ui.selectable_value(&mut view.sort, Some((k, asc)), t);
                }
            }
        });
        combo.response.labelled_by(order_label.id);
    });
    ui.horizontal_wrapped(|ui| {
        let has_sel = sel_count > 0;
        let target_ids: HashSet<String> = if has_sel {
            selected_ids.clone()
        } else {
            cursor
                .and_then(|c| app.items.get(&pid).and_then(|v| v.get(c)))
                .map(|i| i.item_id.clone())
                .into_iter()
                .collect()
        };
        let any = !target_ids.is_empty();
        ui.label(i18n::fmt(tr.n_selected, &[&sel_count.to_string()]));
        if ui.add_enabled(cursor.is_some(), egui::Button::new(tr.play_preview)).on_hover_text("Ctrl+P").clicked()
            && let Some(c) = cursor
        {
            actions.push(RowAction::Play(c));
        }
        if ui.add_enabled(any, egui::Button::new(tr.remove)).on_hover_text(tr.key_delete).clicked() {
            actions.push(RowAction::Remove(target_ids.clone()));
        }
        // Moving follows the playlist order, so it is off while a filter or sort is shown.
        let can_move = any && app.views.get(&pid).is_some_and(|v| v.find.trim().is_empty() && v.sort.is_none());
        for (up, label, key) in [(true, tr.move_up, "Alt+↑"), (false, tr.move_down, "Alt+↓")] {
            if ui.add_enabled(can_move, egui::Button::new(label)).on_hover_text(key).clicked() {
                if !has_sel {
                    app.views.get_mut(&pid).expect("view").selected = target_ids.clone();
                }
                actions.push(RowAction::MoveSelection { up });
            }
        }
        let can_undo = app.undo.get(&pid).is_some_and(|u| !u.is_empty());
        let can_redo = app.redo.get(&pid).is_some_and(|u| !u.is_empty());
        if ui.add_enabled(can_undo, egui::Button::new(tr.undo)).on_hover_text("Ctrl+Z").clicked() {
            app.undo_redo(&pid, false);
        }
        let refs: Vec<ItemRef> = app
            .items
            .get(&pid)
            .map(|v| v.iter().filter(|i| target_ids.contains(&i.item_id)).map(|i| i.r.clone()).collect())
            .unwrap_or_default();
        // Less frequent actions live in a menu so the toolbar stays short at large text sizes.
        ui.menu_button(tr.more_actions, |ui| {
            if ui.add_enabled(can_redo, egui::Button::new(tr.redo).shortcut_text("Ctrl+Y")).clicked() {
                app.undo_redo(&pid, true);
            }
            if ui.add_enabled(cursor.is_some(), egui::Button::new(tr.open_in_tidal).shortcut_text("Ctrl+O")).clicked()
                && let Some(c) = cursor
            {
                actions.push(RowAction::Open(c));
            }
            if ui.add_enabled(any, egui::Button::new(tr.copy_to_dots)).clicked() {
                actions.push(RowAction::CopyTo(refs.clone()));
            }
            if ui.add_enabled(any, egui::Button::new(tr.move_to_dots)).clicked() {
                actions.push(RowAction::MoveTo(target_ids.clone()));
            }
            if ui.add_enabled(any, egui::Button::new(tr.add_to_favourites)).clicked() {
                actions.push(RowAction::Favourite(
                    refs.iter().filter(|r| r.kind == crate::api::ItemKind::Track).map(|r| r.id.clone()).collect(),
                ));
            }
            if ui.add_enabled(any, egui::Button::new(tr.similar_songs_dots)).clicked() {
                actions.push(RowAction::Similar(target_ids.clone()));
            }
            if ui.add(egui::Button::new(tr.select_all).shortcut_text("Ctrl+A")).clicked()
                && let Some(items) = app.items.get(&pid)
            {
                let view = app.views.get_mut(&pid).expect("view");
                let vis = visible(items, view, &|i| i.track.clone());
                view.selected = vis.iter().map(|&i| items[i].item_id.clone()).collect();
            }
            if ui.add_enabled(has_sel, egui::Button::new(tr.select_none)).clicked() {
                app.views.get_mut(&pid).expect("view").selected.clear();
            }
        });
    });
    ui.separator();

    if app.loading.contains(&pid) && !app.items.contains_key(&pid) {
        ui.horizontal(|ui| {
            super::style::busy(ui, app.reduced_motion);
            ui.label(tr.loading);
        });
        return;
    }
    let Some(items) = app.items.get(&pid) else {
        if ui.button(tr.refresh).clicked() {
            app.load_items(&pid);
        }
        return;
    };
    if items.is_empty() {
        ui.label(tr.playlist_empty);
        return;
    }

    let cache = &app.cache;
    let meta = |i: &PlaylistItem| i.track.clone().or_else(|| cache.get(&i.r));
    let view = app.views.get_mut(&pid).expect("view");
    let vis = visible(items, view, &meta);
    let can_reorder = view.find.trim().is_empty() && view.sort.is_none();
    if !can_reorder {
        ui.label(RichText::new(tr.reorder_disabled_hint).italics());
    }
    if vis.is_empty() {
        ui.label(tr.no_matches);
    }
    // The current row in full, wrapped: titles cut short in the table can be
    // read here without a mouse tooltip.
    if let Some(it) = view.cursor.and_then(|c| items.get(c)) {
        let t = meta(it);
        let mut text = format!(
            "{} {}: {}",
            tr.row,
            view.cursor.unwrap_or(0) + 1,
            t.as_ref().map(Track::full_title).unwrap_or_else(|| i18n::fmt(tr.unknown_item, &[&it.r.id]))
        );
        if let Some(t) = &t {
            for part in [t.artist_line(), t.album.clone().unwrap_or_default()] {
                if !part.is_empty() {
                    text.push_str(" — ");
                    text.push_str(&part);
                }
            }
        }
        ui.add(egui::Label::new(RichText::new(text).weak()).wrap());
    }

    // Keyboard handling for the focused row.
    let focused_now = ui.ctx().memory(|m| m.focused());
    let focused_row = match (view.row_focus, view.cursor) {
        (Some(fid), Some(c)) if focused_now == Some(fid) => vis.iter().position(|&i| i == c),
        _ => None,
    };
    if let Some(vpos) = focused_row {
        let (down, up, shift_down, shift_up, alt_down, alt_up, del, home, end, play, open, all) = ui.input_mut(|i| {
            (
                i.key_pressed(Key::ArrowDown) && i.modifiers.is_none(),
                i.key_pressed(Key::ArrowUp) && i.modifiers.is_none(),
                i.consume_key(Modifiers::SHIFT, Key::ArrowDown),
                i.consume_key(Modifiers::SHIFT, Key::ArrowUp),
                i.consume_key(Modifiers::ALT, Key::ArrowDown),
                i.consume_key(Modifiers::ALT, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::NONE, Key::Home),
                i.consume_key(Modifiers::NONE, Key::End),
                i.consume_key(Modifiers::COMMAND, Key::P),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::A),
            )
        });
        let idx = vis[vpos];
        let go = |to: usize, actions: &mut Vec<RowAction>| actions.push(RowAction::Focus(vis[to]));
        if (down || shift_down) && vpos + 1 < vis.len() {
            if shift_down {
                view.selected.insert(items[idx].item_id.clone());
                view.selected.insert(items[vis[vpos + 1]].item_id.clone());
            }
            go(vpos + 1, &mut actions);
        }
        if (up || shift_up) && vpos > 0 {
            if shift_up {
                view.selected.insert(items[idx].item_id.clone());
                view.selected.insert(items[vis[vpos - 1]].item_id.clone());
            }
            go(vpos - 1, &mut actions);
        }
        if home {
            go(0, &mut actions);
        }
        if end {
            go(vis.len() - 1, &mut actions);
        }
        if (alt_up || alt_down) && can_reorder {
            if view.selected.is_empty() {
                view.selected.insert(items[idx].item_id.clone());
            }
            actions.push(RowAction::MoveSelection { up: alt_up });
        }
        if del {
            let ids =
                if view.selected.is_empty() { [items[idx].item_id.clone()].into() } else { view.selected.clone() };
            actions.push(RowAction::Remove(ids));
        }
        if play {
            actions.push(RowAction::Play(idx));
        }
        if open {
            actions.push(RowAction::Open(idx));
        }
        if all {
            view.selected = vis.iter().map(|&i| items[i].item_id.clone()).collect();
        }
    }

    // ------------------------------------------------------------ table
    let row_h = ui.text_style_height(&egui::TextStyle::Body) * 1.6 + 6.0;
    let dragging = egui::DragAndDrop::has_payload_of_type::<Vec<String>>(ui.ctx());
    // Columns adapt to the width (which shrinks as text grows): Added, then Album, are
    // dropped before anything has to be clipped. Their text stays in the row's
    // screen-reader name and tooltip.
    let focus_row = view.focus_row;
    let mut focus_given = false;
    let mut row_focus = None;
    let avail = ui.available_width();
    // Width unit: the body font size (line height would overestimate widths).
    let em = ui.style().text_styles.get(&egui::TextStyle::Body).map_or(14.0, |f| f.size);
    egui::ScrollArea::horizontal().id_salt(("tracks-h", &pid)).auto_shrink([false, false]).show(ui, |ui| {
        let show_added = avail >= em * 52.0;
        let show_album = avail >= em * 40.0;
        // The # cell holds "✓ 1234": a check mark plus the widest row number.
        let num_w = 1.6 + 0.62 * items.len().max(1).to_string().len() as f32;
        let ncols = 6.0 + f32::from(u8::from(show_album)) + f32::from(u8::from(show_added));
        // Column widths plus the gaps between columns and room for the scroll bar.
        let fixed = em * (1.2 + num_w + 3.2 + 8.0 + if show_added { 6.0 } else { 0.0 } + 1.5)
            + ncols * ui.spacing().item_spacing.x;
        let text_w = (avail - fixed).max(em * 12.0);
        let (wt, wa, wl) = if show_album { (0.45, 0.30, 0.25) } else { (0.6, 0.4, 0.0) };
        let mut table = TableBuilder::new(ui)
            .id_salt(("tracks", &pid, (avail / em).round() as i32))
            .striped(true)
            .resizable(true)
            .sense(egui::Sense::click())
            .animate_scrolling(!app.reduced_motion)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(em * 1.2))
            .column(Column::exact(em * num_w))
            .column(Column::initial(text_w * wt).at_least(em * 4.0).clip(true))
            .column(Column::initial(text_w * wa).at_least(em * 3.0).clip(true));
        if show_album {
            table = table.column(Column::initial(text_w * wl).at_least(em * 3.0).clip(true));
        }
        table = table.column(Column::exact(em * 3.2));
        if show_added {
            table = table.column(Column::exact(em * 6.0));
        }
        table = table.column(Column::remainder().at_least(em * 8.0).clip(true));
        if let Some(s) = view.scroll_to.take()
            && let Some(vp) = vis.iter().position(|&i| i == s)
        {
            table = table.scroll_to_row(vp, None);
        }
        let selected = view.selected.clone();
        let cursor = view.cursor;
        table
            .header(row_h, |mut h| {
                let mut heads = vec!["", "#", tr.col_title, tr.col_artist];
                if show_album {
                    heads.push(tr.col_album);
                }
                heads.push(tr.col_time);
                if show_added {
                    heads.push(tr.col_added);
                }
                heads.push(tr.col_status);
                for t in heads {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, vis.len(), |mut row| {
                    let vpos = row.index();
                    let idx = vis[vpos];
                    let it = &items[idx];
                    let t = meta(it);
                    let is_sel = selected.contains(&it.item_id);
                    // The ✓ in the # cell shows selection without relying on colour;
                    // the cursor row is shown by the focus ring.
                    row.set_selected(is_sel);
                    let mut title =
                        t.as_ref().map(Track::full_title).unwrap_or_else(|| i18n::fmt(tr.unknown_item, &[&it.r.id]));
                    // Said in text, so it is not shown by the icon alone.
                    if it.r.kind == crate::api::ItemKind::Video {
                        title = format!("🎬 {}: {title}", tr.video);
                    }
                    let artist = t.as_ref().map(Track::artist_line).unwrap_or_default();
                    let album = t.as_ref().and_then(|t| t.album.clone()).unwrap_or_default();
                    let status = if it.replacement.is_some() {
                        format!("↻ {}", tr.status_replaced)
                    } else if it.available() {
                        format!("✓ {}", tr.status_ok)
                    } else {
                        format!("⚠ {}", tr.status_unavailable)
                    };
                    // Drag handle (mouse only; keyboard users use Alt+↑/↓).
                    row.col(|ui| {
                        if can_reorder {
                            let ids: Vec<String> =
                                if is_sel { selected.iter().cloned().collect() } else { vec![it.item_id.clone()] };
                            ui.dnd_drag_source(egui::Id::unique(("drag", &it.item_id)), ids, |ui| {
                                ui.label("⠿").on_hover_text(tr.drag_to_reorder);
                            });
                        }
                    });
                    row.col(|ui| {
                        let num = if is_sel { format!("✓ {}", idx + 1) } else { format!("{}", idx + 1) };
                        let r = ui.add(egui::Button::selectable(is_sel, num));
                        let a11y = format!(
                            "{} {}, {}{}{}, {}, {}",
                            tr.row,
                            idx + 1,
                            title,
                            if artist.is_empty() { String::new() } else { format!(", {artist}") },
                            if album.is_empty() { String::new() } else { format!(", {album}") },
                            t.as_ref().map(|t| format_duration(t.duration_secs)).unwrap_or_default(),
                            status
                        );
                        r.widget_info(|| WidgetInfo::selected(Role::CheckBox, true, is_sel, &a11y));
                        if focus_row == Some(idx) {
                            r.request_focus();
                            focus_given = true;
                        }
                        if r.has_focus() {
                            row_focus = Some(r.id);
                            if cursor != Some(idx) && focus_row.is_none() {
                                actions.push(RowAction::Cursor(idx));
                            }
                        }
                        if r.clicked() {
                            actions.push(RowAction::Click { idx, toggle: true, range: false });
                        }
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&title).truncate()).on_hover_text(&title);
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(&artist).truncate()).on_hover_text(&artist);
                    });
                    if show_album {
                        row.col(|ui| {
                            ui.add(egui::Label::new(&album).truncate()).on_hover_text(&album);
                        });
                    }
                    row.col(|ui| {
                        ui.label(t.as_ref().map(|t| format_duration(t.duration_secs)).unwrap_or_default());
                    });
                    let added =
                        it.added_at.as_deref().map(|d| d.chars().take(10).collect::<String>()).unwrap_or_default();
                    if show_added {
                        row.col(|ui| {
                            ui.label(&added);
                        });
                    }
                    row.col(|ui| {
                        let mut rt = RichText::new(&status);
                        if !it.available() {
                            rt = rt.strong();
                        }
                        ui.label(rt);
                    });
                    let resp = row.response();
                    if resp.clicked() {
                        let m = resp.ctx.input(|i| i.modifiers);
                        actions.push(RowAction::Click { idx, toggle: m.command, range: m.shift });
                    }
                    if resp.double_clicked() {
                        actions.push(RowAction::Play(idx));
                    }
                    if dragging && can_reorder {
                        if let Some(p) = resp.dnd_hover_payload::<Vec<String>>() {
                            let _ = p;
                            let r = resp.rect;
                            resp.ctx
                                .layer_painter(egui::LayerId::new(
                                    egui::Order::Foreground,
                                    egui::Id::unique("dnd-line"),
                                ))
                                .hline(
                                    r.x_range(),
                                    r.top(),
                                    egui::Stroke::new(
                                        3.0,
                                        super::style::focus_color(resp.ctx.theme() == egui::Theme::Dark),
                                    ),
                                );
                        }
                        if let Some(ids) = resp.dnd_release_payload::<Vec<String>>() {
                            actions.push(RowAction::DropBefore { ids: (*ids).clone(), before: idx });
                        }
                    }
                    resp.context_menu(|ui| {
                        if ui.button(tr.play_preview).clicked() {
                            actions.push(RowAction::Play(idx));
                        }
                        if ui.button(tr.open_in_tidal).clicked() {
                            actions.push(RowAction::Open(idx));
                        }
                        if ui.button(tr.copy_link).clicked() {
                            actions.push(RowAction::CopyLink(idx));
                        }
                        let ids: HashSet<String> = if is_sel { selected.clone() } else { [it.item_id.clone()].into() };
                        let refs: Vec<ItemRef> =
                            items.iter().filter(|i| ids.contains(&i.item_id)).map(|i| i.r.clone()).collect();
                        if ui.button(tr.copy_to_dots).clicked() {
                            actions.push(RowAction::CopyTo(refs.clone()));
                        }
                        if ui.button(tr.move_to_dots).clicked() {
                            actions.push(RowAction::MoveTo(ids.clone()));
                        }
                        if ui.button(tr.add_to_favourites).clicked() {
                            actions.push(RowAction::Favourite(
                                refs.iter()
                                    .filter(|r| r.kind == crate::api::ItemKind::Track)
                                    .map(|r| r.id.clone())
                                    .collect(),
                            ));
                        }
                        if ui.button(tr.similar_songs_dots).clicked() {
                            actions.push(RowAction::Similar(ids.clone()));
                        }
                        if it.r.kind == crate::api::ItemKind::Track
                            && it.track.is_some()
                            && ui.button(tr.song_radio_playlist).clicked()
                        {
                            actions.push(RowAction::Radio(idx));
                        }
                        if it.track.is_some() && ui.button(tr.where_is_dots).clicked() {
                            actions.push(RowAction::WhereIs(idx));
                        }
                        if ui.button(tr.remove).clicked() {
                            actions.push(RowAction::Remove(ids));
                        }
                    });
                });
            });
    });
    let view = app.views.get_mut(&pid).expect("view");
    // The table only draws rows in view, and a jump (Home/End) scrolls there
    // next frame: keep the request (and the scroll) until the row has the focus.
    if let Some(f) = focus_row {
        if focus_given || !vis.contains(&f) {
            view.focus_row = None;
        } else {
            view.scroll_to = Some(f);
            ui.ctx().request_repaint();
        }
    }
    if row_focus.is_some() || focused_now != view.row_focus {
        view.row_focus = row_focus;
    }

    // ------------------------------------------------------------ actions
    for a in actions {
        run_action(app, &pid, a, &vis);
    }
}

fn run_action(app: &mut App, pid: &str, a: RowAction, vis: &[usize]) {
    let tr = app.tr;
    let Some(items) = app.items.get(pid).cloned() else { return };
    // An earlier action this frame (or a reload) may have shortened the list.
    let idx = match &a {
        RowAction::Click { idx, .. }
        | RowAction::Cursor(idx)
        | RowAction::Focus(idx)
        | RowAction::Play(idx)
        | RowAction::Open(idx)
        | RowAction::CopyLink(idx)
        | RowAction::Radio(idx)
        | RowAction::WhereIs(idx) => Some(*idx),
        RowAction::DropBefore { before, .. } => Some(*before),
        _ => None,
    };
    if idx.is_some_and(|i| i >= items.len()) {
        return;
    }
    let view = app.views.entry(pid.to_owned()).or_default();
    match a {
        RowAction::Click { idx, toggle, range } => {
            let id = items[idx].item_id.clone();
            if range {
                let a = view.anchor.or(view.cursor).unwrap_or(idx);
                let (pa, pb) =
                    (vis.iter().position(|&i| i == a).unwrap_or(0), vis.iter().position(|&i| i == idx).unwrap_or(0));
                for &i in &vis[pa.min(pb)..=pa.max(pb)] {
                    view.selected.insert(items[i].item_id.clone());
                }
            } else if toggle {
                if !view.selected.remove(&id) {
                    view.selected.insert(id);
                }
                view.anchor = Some(idx);
            } else {
                view.selected = [id].into();
                view.anchor = Some(idx);
            }
            view.cursor = Some(idx);
        }
        RowAction::Cursor(idx) => view.cursor = Some(idx),
        RowAction::Focus(idx) => {
            view.cursor = Some(idx);
            view.focus_row = Some(idx);
            view.scroll_to = Some(idx);
        }
        RowAction::Play(idx) => match app.track_meta(&items[idx]) {
            Some(t) if items[idx].track.is_some() => app.play(&t),
            _ => app.error(tr.cannot_play_unavailable),
        },
        RowAction::Open(idx) => {
            let it = &items[idx];
            let t = Track { id: it.r.id.clone(), kind: it.r.kind, ..Default::default() };
            app.open_in_tidal(t.app_url(), t.web_url());
        }
        RowAction::CopyLink(idx) => {
            let it = &items[idx];
            let t = Track { id: it.r.id.clone(), kind: it.r.kind, ..Default::default() };
            app.copy_text = Some(t.web_url());
        }
        RowAction::Remove(ids) => app.remove_from_playlist(pid, &ids),
        RowAction::MoveSelection { up } => {
            let sel: Vec<usize> = (0..items.len()).filter(|&i| view.selected.contains(&items[i].item_id)).collect();
            let (Some(&first), Some(&last)) = (sel.first(), sel.last()) else { return };
            let moving: Vec<PlaylistItem> = sel.iter().map(|&i| items[i].clone()).collect();
            let before = if up {
                if first == 0 {
                    return;
                }
                Some(first - 1)
            } else {
                if last + 1 >= items.len() {
                    return;
                }
                (last + 2 < items.len()).then_some(last + 2)
            };
            let cursor_item = view.cursor.and_then(|c| items.get(c)).map(|i| i.item_id.clone());
            move_items(app, pid, &items, moving, before);
            // Keep keyboard focus on the moved item.
            if let (Some(id), Some(list)) = (cursor_item, app.items.get(pid))
                && let Some(n) = list.iter().position(|i| i.item_id == id)
            {
                let v = app.views.get_mut(pid).expect("view");
                v.cursor = Some(n);
                v.focus_row = Some(n);
                v.scroll_to = Some(n);
            }
        }
        RowAction::DropBefore { ids, before } => {
            let set: HashSet<&String> = ids.iter().collect();
            if set.contains(&items[before].item_id) {
                return;
            }
            let moving: Vec<PlaylistItem> = items.iter().filter(|i| set.contains(&i.item_id)).cloned().collect();
            move_items(app, pid, &items, moving, Some(before));
        }
        RowAction::CopyTo(refs) => app.open_dialog(Dialog::add_to(refs, Some(pid.to_owned()), None)),
        RowAction::MoveTo(ids) => {
            let refs = items.iter().filter(|i| ids.contains(&i.item_id)).map(|i| i.r.clone()).collect();
            app.open_dialog(Dialog::add_to(refs, Some(pid.to_owned()), Some(ids)));
        }
        RowAction::Favourite(ids) => app.worker.send(Cmd::Favourite { ids, add: true }),
        RowAction::Similar(ids) => app.recommend(pid, ids, &[]),
        RowAction::WhereIs(idx) => {
            let t = app.track_meta(&items[idx]);
            app.open_across(t);
        }
        RowAction::Radio(idx) => {
            if let Some(t) = app.track_meta(&items[idx]) {
                app.save_radio(crate::api::RadioSeed::Track(t.id.clone()), &t.full_title());
            }
        }
    }
}

/// Moves `moving` (in playlist order) before `items[before]`, or to the end.
/// Updates the local list immediately; the worker reloads afterwards.
fn move_items(app: &mut App, pid: &str, items: &[PlaylistItem], moving: Vec<PlaylistItem>, before: Option<usize>) {
    let anchor = before.map(|b| items[b].item_id.clone());
    let ids: HashSet<&String> = moving.iter().map(|i| &i.item_id).collect();
    let mut rest: Vec<PlaylistItem> = items.iter().filter(|i| !ids.contains(&i.item_id)).cloned().collect();
    let at = anchor.as_ref().and_then(|a| rest.iter().position(|i| &i.item_id == a)).unwrap_or(rest.len());
    rest.splice(at..at, moving.iter().cloned());
    app.snapshot(pid);
    app.items.insert(pid.to_owned(), rest);
    app.worker.send(Cmd::MoveItems {
        pid: pid.to_owned(),
        items: moving.into_iter().map(|i| (i.r, i.item_id)).collect(),
        before: anchor,
    });
}
