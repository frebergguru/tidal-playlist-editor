//! Headless UI tests through the AccessKit tree (egui_kittest).

use egui::{Key, Modifiers, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use super::{App, AuthState};
use crate::api::{ItemKind, ItemRef, Playlist, PlaylistItem, Track, User};

fn test_home() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let home = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-home");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("config")).unwrap();
        std::fs::write(home.join("config/config.toml"), "language = \"English\"\nmotion = \"Reduced\"\n").unwrap();
        // SAFETY: set once, before any test reads it; only `Paths::new` reads this variable.
        unsafe { std::env::set_var("TIDAL_EDITOR_HOME", &home) };
    });
}

fn harness() -> Harness<'static, App> {
    test_home();
    Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|cc| App::new(cc))
}

fn item(n: usize, title: &str, available: bool) -> PlaylistItem {
    PlaylistItem {
        item_id: format!("occ{n}"),
        r: ItemRef { kind: ItemKind::Track, id: format!("{n}") },
        added_at: Some("2024-05-01T00:00:00Z".into()),
        track: available.then(|| Track {
            id: format!("{n}"),
            title: title.into(),
            artists: vec!["Artist".into()],
            album: Some("Album".into()),
            duration_secs: 180 + n as u32,
            ..Default::default()
        }),
        replacement: None,
    }
}

fn logged_in(h: &mut Harness<'static, App>) {
    let app = h.state_mut();
    app.auth = AuthState::LoggedIn;
    app.user = Some(User { id: "u".into(), username: "tester".into(), country: "NO".into() });
    app.playlists = vec![
        Playlist {
            id: "p1".into(),
            name: "Road trip".into(),
            number_of_items: Some(3),
            owned: true,
            ..Default::default()
        },
        Playlist { id: "p2".into(), name: "Calm".into(), number_of_items: Some(0), owned: false, ..Default::default() },
    ];
    app.playlists_loaded = true;
    app.current = Some("p1".into());
    app.items.insert("p1".into(), vec![item(1, "First song", true), item(2, "Second song", true), item(3, "", false)]);
    app.items.insert("p2".into(), Vec::new());
    h.run();
}

#[test]
fn welcome_screen_is_labelled_and_keyboard_reachable() {
    let mut h = harness();
    // No client id in the test config, so no session restore runs in the background.
    h.state_mut().cfg.client_id = "test-client".into();
    h.run();
    let login = h.get_by_role_and_label(Role::Button, "Log in with TIDAL");
    assert!(login.accesskit_node().is_focused(), "login button should start focused");
    h.get_by_label("API settings…");
    h.key_press(Key::Tab);
    h.run();
    assert!(h.get_by_label("API settings…").accesskit_node().is_focused());
}

#[test]
fn every_control_has_a_name() {
    let mut h = harness();
    logged_in(&mut h);
    for role in [Role::Button, Role::CheckBox, Role::TextInput, Role::Tab, Role::RadioButton] {
        for n in h.query_all_by_role(role) {
            let label = n.accesskit_node().label().unwrap_or_default();
            assert!(!label.trim().is_empty(), "{role:?} without a name: {n:?}");
        }
    }
}

#[test]
fn rows_are_described_and_status_is_text() {
    let mut h = harness();
    logged_in(&mut h);
    h.get_by_label_contains("Row 1, First song, Artist, Album, 3:01");
    h.get_by_label_contains("Row 3, Unknown item (TIDAL id 3)");
    // Unavailable status is shown as text, not only colour.
    assert!(h.query_all_by_label_contains("Unavailable").count() >= 1);
}

#[test]
fn keyboard_moves_and_removes_rows() {
    let mut h = harness();
    logged_in(&mut h);
    h.get_by_label_contains("Row 1, First song").focus();
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().views["p1"].cursor, Some(1));
    assert!(h.get_by_label_contains("Row 2, Second song").accesskit_node().is_focused());
    // Alt+Up moves "Second song" to the top (locally at once; the API call goes to the worker).
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowUp);
    h.run();
    let order: Vec<String> = h.state().items["p1"].iter().map(|i| i.item_id.clone()).collect();
    assert_eq!(order, vec!["occ2", "occ1", "occ3"]);
    assert_eq!(h.state().views["p1"].cursor, Some(0));
    assert!(h.state().undo.get("p1").is_some_and(|u| u.len() == 1), "move is undoable");
    // Delete removes the selected row.
    h.key_press(Key::Delete);
    h.run();
    let left: Vec<String> = h.state().items["p1"].iter().map(|i| i.item_id.clone()).collect();
    assert_eq!(left, vec!["occ1", "occ3"]);
}

#[test]
fn dialogs_open_from_keyboard_and_close_with_escape() {
    let mut h = harness();
    logged_in(&mut h);
    h.key_press(Key::F1);
    h.run();
    assert!(h.state().dialog.is_some());
    h.get_by_label_contains("Alt+Up / Alt+Down — move the selected songs");
    assert!(h.get_by_role_and_label(Role::Button, "Close").accesskit_node().is_focused());
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().dialog.is_none());

    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run();
    let name = h.get_by_role_and_label(Role::TextInput, "Name");
    assert!(name.accesskit_node().is_focused(), "name field gets focus when the dialog opens");
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().dialog.is_none());
}

#[test]
fn find_filters_and_disables_reordering() {
    let mut h = harness();
    logged_in(&mut h);
    h.state_mut().views.get_mut("p1").unwrap().find = "second".into();
    h.run();
    assert!(h.query_by_label_contains("Row 1, First song").is_none());
    h.get_by_label_contains("Row 2, Second song");
    h.get_by_label_contains("Moving songs is turned off");
}

#[test]
fn text_scale_persists() {
    let mut h = harness();
    logged_in(&mut h);
    let ctx = h.ctx.clone();
    h.state_mut().set_text_scale(&ctx, 1.5);
    h.run();
    assert_eq!(crate::store::Config::load(&crate::store::Paths::new()).text_scale, 1.5);
    h.state_mut().set_text_scale(&ctx, 1.0);
}

/// Renders screenshots to `target/screens/` for manual review:
/// `cargo test screenshots -- --ignored`
#[test]
#[ignore]
fn screenshots() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness();
    h.state_mut().cfg.client_id = "test-client".into();
    h.run();
    h.render().unwrap().save(dir.join("welcome.png")).unwrap();
    logged_in(&mut h);
    {
        let app = h.state_mut();
        let r = crate::api::SearchResults {
            tracks: (10..16).map(|n| item(n, &format!("Search hit {n}"), true).track.unwrap()).collect(),
            ..Default::default()
        };
        app.search.query = "hits".into();
        app.search.results = Some(r);
    }
    h.get_by_label_contains("Row 2, Second song").focus();
    h.run();
    h.render().unwrap().save(dir.join("main-dark.png")).unwrap();
    h.key_press(Key::F1);
    h.run();
    h.render().unwrap().save(dir.join("shortcuts.png")).unwrap();
    h.key_press(Key::Escape);
    let ctx = h.ctx.clone();
    {
        let app = h.state_mut();
        app.cfg.theme = crate::store::ThemeChoice::Light;
        app.restyle(&ctx);
        app.set_text_scale(&ctx, 1.6);
    }
    h.run();
    h.render().unwrap().save(dir.join("main-light-160.png")).unwrap();
    {
        let app = h.state_mut();
        let recs = (60..72)
            .map(|n| crate::ops::recommend::Recommendation {
                track: item(n, &format!("Suggested song {n}"), true).track.unwrap(),
                seeds: 1 + n % 3,
            })
            .collect();
        app.open_dialog(super::Dialog::recommend("p1".into(), recs, Default::default()));
    }
    h.run();
    h.render().unwrap().save(dir.join("recommend-160.png")).unwrap();
    h.key_press(Key::Escape);
    h.run();
    {
        let app = h.state_mut();
        app.open_dialog(super::Dialog::generate(false));
    }
    h.run();
    h.get_by_label("Rock").click();
    h.run();
    h.get_by_label("Jazz").click();
    h.run();
    h.render().unwrap().save(dir.join("generate-160.png")).unwrap();
    h.key_press(Key::Escape);
    h.run();
    h.state_mut().set_text_scale(&ctx, 1.0);
    let app = h.state_mut();
    app.cfg.theme = crate::store::ThemeChoice::System;
    app.restyle(&ctx);
}

#[test]
fn settings_fields_accept_pasted_text() {
    let mut h = harness();
    h.state_mut().auth = AuthState::LoggedOut;
    h.run();
    h.get_by_label("API settings…").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Paste: Client ID");
    h.get_by_role_and_label(Role::Button, "Paste: Client secret (usually not needed)");
    // Ctrl+V reaches egui as a Paste event; make sure the field takes it.
    let field = h.get_by_role_and_label(Role::TextInput, "Client ID");
    field.focus();
    h.run();
    h.event(egui::Event::Paste("  abc123-long-client-id  ".into()));
    h.run();
    match &h.state().dialog {
        Some(super::Dialog::Settings(f)) => assert_eq!(f.cfg.client_id, "  abc123-long-client-id  "),
        _ => panic!("settings dialog should still be open"),
    }
    h.key_press(Key::Escape);
    h.run();
}

#[test]
fn removes_all_geo_restricted_at_once() {
    use crate::worker::{Reason, UnavailableHit};
    let mut h = harness();
    logged_in(&mut h);
    {
        let app = h.state_mut();
        let items = app.items["p1"].clone();
        let hit = |n: usize, reason| UnavailableHit {
            pid: "p1".into(),
            playlist: "Road trip".into(),
            item: items[n].clone(),
            reason,
            known: items[n].track.clone(),
        };
        let hits = vec![hit(0, Reason::GeoRestricted), hit(1, Reason::GeoRestricted), hit(2, Reason::Missing)];
        app.open_dialog(super::Dialog::unavailable_results(hits));
    }
    h.run();
    h.get_by_label("Remove all GEO_RESTRICTED (2)").click();
    h.run();
    h.get_by_label_contains("Remove 2 GEO_RESTRICTED songs from 1 playlists?");
    h.get_by_label("Yes, remove").click();
    h.run();
    let left: Vec<String> = h.state().items["p1"].iter().map(|i| i.item_id.clone()).collect();
    assert_eq!(left, vec!["occ3"], "both GEO_RESTRICTED songs removed, the other one kept");
    match &h.state().dialog {
        Some(super::Dialog::Unavailable(u)) => {
            assert_eq!(u.hits.len(), 1);
            assert_eq!((u.remove.len(), u.use_repl.len()), (1, 1));
        }
        _ => panic!("results stay open for the remaining songs"),
    }
    assert!(h.query_by_label_contains("Remove all GEO_RESTRICTED").is_none());
    assert_eq!(h.state().undo["p1"].len(), 1, "undoable");
}

/// Runs frames until a log line starting with `prefix` appears (worker events are async).
fn wait_for_log(h: &mut Harness<'static, App>, prefix: &str) {
    for _ in 0..200 {
        h.run();
        if h.state().log.iter().any(|l| l.text.starts_with(prefix)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!(
        "no log line starting with {prefix:?}; log: {:?}",
        h.state().log.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

fn connect_spotify(h: &mut Harness<'static, App>) {
    let app = h.state_mut();
    app.cfg.spotify_client_id = "sp-client".into();
    app.spotify = super::SpState::On(crate::spotify::SpUser { id: "me".into(), display_name: "Tester".into() });
    app.sp_playlists = vec![
        crate::spotify::SpPlaylist { name: "Liked Songs".into(), ..Default::default() },
        crate::spotify::SpPlaylist {
            id: "s1".into(),
            name: "Summer".into(),
            owner_id: "me".into(),
            total: Some(12),
            ..Default::default()
        },
        crate::spotify::SpPlaylist {
            id: "s2".into(),
            name: "Someone else's".into(),
            owner_id: "other".into(),
            ..Default::default()
        },
    ];
}

#[test]
fn spotify_menu_asks_for_client_id_first() {
    let mut h = harness();
    logged_in(&mut h);
    h.state_mut().spotify_action(None);
    h.run();
    assert!(matches!(h.state().dialog, Some(super::Dialog::Settings(_))));
    h.get_by_role_and_label(Role::TextInput, "Spotify Client ID");
    h.get_by_role_and_label(Role::Button, "Paste: Spotify Client ID");
    // «localhost» is refused for Spotify; Save is disabled until it is fixed.
    if let Some(super::Dialog::Settings(f)) = &mut h.state_mut().dialog {
        f.cfg.spotify_redirect_uri = "http://localhost:8766/callback".into();
    }
    h.run();
    h.get_by_label_contains("«localhost» is not allowed");
    assert!(h.get_by_role_and_label(Role::Button, "Save").accesskit_node().is_disabled());
    h.key_press(Key::Escape);
    h.run();
}

#[test]
fn copy_from_spotify_lists_playlists_with_attribution() {
    let mut h = harness();
    logged_in(&mut h);
    connect_spotify(&mut h);
    h.state_mut().spotify_action(Some(super::Dialog::FromSpotify { selected: Default::default() }));
    h.run();
    h.get_by_label("Playlist and song information from Spotify.");
    h.get_by_role_and_label(Role::CheckBox, "Liked Songs");
    h.get_by_role_and_label(Role::CheckBox, "Summer (12 items)").click();
    h.run();
    h.get_by_role_and_label(Role::Button, "Find matches").click();
    h.run();
    assert!(h.state().dialog.is_none());
    // The test worker has no TIDAL login, so the job ends at once with that error.
    wait_for_log(&mut h, "Reading Spotify playlists and finding the songs on TIDAL");
    assert!(matches!(h.state().spotify, super::SpState::On(_)), "a TIDAL error must not disconnect Spotify");
}

#[test]
fn review_copy_to_spotify() {
    use crate::ops::files::{ImportPlaylist, ImportRow};
    use crate::ops::matcher::{MatchMethod, MatchRow};
    let mut h = harness();
    logged_in(&mut h);
    connect_spotify(&mut h);
    let row = ImportRow { title: "First song".into(), artists: vec!["Artist".into()], ..Default::default() };
    let hit =
        Track { id: "spid1".into(), title: "First song".into(), artists: vec!["Artist".into()], ..Default::default() };
    let rows = vec![
        MatchRow {
            row: row.clone(),
            matched: Some(hit),
            confidence: 0.97,
            method: MatchMethod::Isrc,
            include: true,
            alternatives: vec![],
        },
        MatchRow {
            row,
            matched: None,
            confidence: 0.0,
            method: MatchMethod::None,
            include: false,
            alternatives: vec![],
        },
    ];
    let pl = ImportPlaylist { name: "Road trip".into(), rows: vec![], ..Default::default() };
    h.state_mut().open_dialog(super::Dialog::import_review(crate::worker::Dest::Spotify, vec![(pl, rows)]));
    h.run();
    h.get_by_label("Check the copy to Spotify");
    h.get_by_label("Playlist and song information from Spotify.");
    h.get_by_label("Open in Spotify");
    h.get_by_role_and_label(Role::CheckBox, "Public on Spotify");
    // Only playlists the user owns on Spotify are offered as targets (not Liked Songs, not others').
    h.get_by_role_and_label(Role::Button, "Copy 1 songs to Spotify").click();
    h.run();
    assert!(h.state().dialog.is_none());
    wait_for_log(&mut h, "Copying to Spotify: Road trip");
    // The worker had no Spotify connection: that is a Spotify login error → disconnected.
    assert!(matches!(h.state().spotify, super::SpState::Off));
}

#[test]
fn deleting_the_last_row_keeps_the_cursor_in_range() {
    let mut h = harness();
    logged_in(&mut h);
    h.get_by_label_contains("Row 3, Unknown item").focus();
    h.run();
    assert_eq!(h.state().views["p1"].cursor, Some(2));
    h.key_press(Key::Delete);
    h.run();
    assert_eq!(h.state().items["p1"].len(), 2);
    assert!(h.state().views["p1"].cursor.is_some_and(|c| c < 2), "cursor clamped");
    // Play and Open act on the clamped cursor instead of panicking.
    h.key_press_modifiers(Modifiers::COMMAND, Key::P);
    h.run();
    h.get_by_role_and_label(Role::Button, "Play preview").click();
    h.run();
}

#[test]
fn move_to_removes_from_the_source_only_through_one_job() {
    let mut h = harness();
    logged_in(&mut h);
    let ids: std::collections::HashSet<String> = ["occ1".to_owned()].into();
    h.state_mut().move_to_playlists("p1", &ids, vec!["p2".into()], None, false);
    h.run();
    let left: Vec<String> = h.state().items["p1"].iter().map(|i| i.item_id.clone()).collect();
    assert_eq!(left, vec!["occ2", "occ3"], "rows leave the list at once");
    // A single Move job (no separate add and remove jobs racing each other).
    let labels: Vec<String> = h.state().jobs.iter().map(|j| j.label.clone()).collect();
    assert!(labels.iter().all(|l| !l.starts_with("Adding") && !l.starts_with("Removing")), "{labels:?}");
    assert!(h.state().undo.get("p1").is_some_and(|u| !u.is_empty()), "source is undoable");
}

#[test]
fn duplicates_dialog_follows_rows_not_positions() {
    let mut h = harness();
    logged_in(&mut h);
    let mut dup = item(1, "First song", true);
    dup.item_id = "occ1b".into();
    h.state_mut().items.get_mut("p1").unwrap().push(dup);
    let d = super::Dialog::duplicates(h.state(), "p1");
    h.state_mut().open_dialog(d);
    h.run();
    // A reload puts the rows in another order while the dialog is open.
    h.state_mut().items.get_mut("p1").unwrap().reverse();
    h.run();
    let Some(super::Dialog::Duplicates { remove, .. }) = &h.state().dialog else { panic!("dialog closed") };
    assert_eq!(remove.len(), 1);
    // After the reverse, "occ1b" comes first and is the one kept; "occ1" is removed.
    assert!(remove.contains("occ1"), "{remove:?}");
}

#[test]
fn recommendations_are_added_to_the_playlist() {
    use crate::ops::recommend::Recommendation;
    let mut h = harness();
    logged_in(&mut h);
    {
        let rec = |id: &str, title: &str, seeds| Recommendation {
            track: Track { id: id.into(), title: title.into(), artists: vec!["New".into()], ..Default::default() },
            seeds,
        };
        let recs = vec![rec("50", "Fresh one", 3), rec("51", "Fresh two", 1)];
        h.state_mut().open_dialog(super::Dialog::recommend("p1".into(), recs, Default::default()));
    }
    h.run();
    h.get_by_label_contains("Songs like those in «Road trip»");
    // Every row has a labelled preview button and a checkbox that says why it was suggested.
    h.get_by_label("Play preview: Fresh one — New");
    h.get_by_label_contains("Fresh one — New · suggested by 3 of your songs").click();
    h.run();
    h.get_by_label("Add 1 to the playlist").click();
    h.run();
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().undo["p1"].len(), 1, "undoable");
    wait_for_log(&mut h, "Adding 1 to Road trip");
}

#[test]
fn generate_dialog_needs_a_genre() {
    let mut h = harness();
    logged_in(&mut h);
    {
        let app = h.state_mut();
        app.open_dialog(super::Dialog::generate(false));
    }
    h.run();
    assert!(h.get_by_label("Generate").accesskit_node().is_disabled(), "disabled until a genre is chosen");
    h.get_by_label("Pop").click();
    h.run();
    h.get_by_label("Jazz").click();
    h.run();
    // With two genres the user chooses one mixed playlist or one per genre.
    h.get_by_label("One playlist per genre").click();
    h.run();
    h.get_by_label("Songs per playlist");
    // A typed genre is added to the list and ticked.
    h.get_by_role_and_label(Role::TextInput, "Other genre (type any genre, mood or style)").focus();
    h.run();
    h.event(egui::Event::Text("Bossa nova".into()));
    h.run();
    h.get_by_label("Add genre").click();
    h.run();
    h.get_by_label("Genres (3 chosen)");
    h.get_by_label("Generate").click();
    h.run();
    assert!(h.state().dialog.is_none());
    wait_for_log(&mut h, "Generating playlists: Pop, Jazz, Bossa nova");
}

/// Emoji in names must lay out (the test harness panics on a missing glyph).
#[test]
fn emoji_in_names_are_drawn() {
    let mut h = harness();
    logged_in(&mut h);
    {
        let app = h.state_mut();
        app.playlists[0].name = "Road trip 🚗🎶 😀😂".into();
        app.items.insert("p1".into(), vec![item(1, "Happy 😊 song 💔", true)]);
    }
    h.run();
    assert!(h.query_all_by_label_contains("Road trip 🚗🎶 😀😂").count() >= 1);
    assert!(h.query_all_by_label_contains("Happy 😊 song 💔").count() >= 1);
}

#[test]
#[ignore = "writes a screenshot for checking emoji by eye"]
fn emoji_screenshot() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screens");
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = harness();
    logged_in(&mut h);
    {
        let app = h.state_mut();
        app.playlists[0].name = "Road trip 🚗🎶 😀😂🥰".into();
        app.playlists[1].name = "Chill ☕️ ❤️ ✨ 🇳🇴 👍🏽 🧑‍🎤 🫶 🥹 🪩".into();
        let names = [
            "Happy 😊 song",
            "Love ❤️ ❤ ♥ 💔 ☕️ ☀️ ✔️ 1️⃣",
            "Flags 🇳🇴🇸🇪🇺🇸",
            "Skin 👋🏻👋🏿",
            "New 🫠🫶🪿🩷",
            "Sym ★ ♪ ☀ ✔ ⚡",
        ];
        let items = names.iter().enumerate().map(|(n, t)| item(n + 1, t, true)).collect();
        app.items.insert("p1".into(), items);
    }
    h.run();
    h.render().unwrap().save(dir.join("emoji.png")).unwrap();
    let video = {
        let mut v = item(9, "Live clip", true);
        v.r.kind = ItemKind::Video;
        v
    };
    h.state_mut().items.get_mut("p1").unwrap().push(video.clone());
    h.run();
    h.render().unwrap().save(dir.join("video-row.png")).unwrap();
    let p = h.state().playlists[0].clone();
    h.state_mut().open_dialog(super::Dialog::videos_results(vec![(p, video)]));
    h.run();
    h.render().unwrap().save(dir.join("videos.png")).unwrap();
}

#[test]
fn videos_are_marked_and_listed() {
    let mut h = harness();
    logged_in(&mut h);
    let video = {
        let mut v = item(9, "Live clip", true);
        v.r.kind = ItemKind::Video;
        v.track.as_mut().unwrap().kind = ItemKind::Video;
        v
    };
    {
        let app = h.state_mut();
        app.items.get_mut("p1").unwrap().push(video.clone());
    }
    h.run();
    // The row says it is a video in text (read out by screen readers too).
    assert!(h.query_all_by_label_contains("Video: Live clip").count() >= 1);
    {
        let app = h.state_mut();
        let p = app.playlists[0].clone();
        app.open_dialog(super::Dialog::videos_results(vec![(p, video)]));
    }
    h.run();
    h.get_by_label("Found 1 video(s) in 1 playlist(s).");
    h.get_by_label("Open in TIDAL: Live clip — Artist");
    h.get_by_role_and_label(Role::CheckBox, "Live clip — Artist").click();
    h.run();
    h.get_by_label("Replace 0, remove 1").click();
    h.run();
    assert!(h.state().dialog.is_none());
    assert!(!h.state().items["p1"].iter().any(|i| i.item_id == "occ9"), "video removed");
    assert_eq!(h.state().undo["p1"].len(), 1, "undoable");
}

#[test]
fn videos_can_be_swapped_for_their_song() {
    let mut h = harness();
    logged_in(&mut h);
    let video = |n: usize, title: &str| {
        let mut v = item(n, title, true);
        v.r.kind = ItemKind::Video;
        v
    };
    let (clip, other) = (video(8, "Hit song (Official Video)"), video(9, "Rare clip"));
    {
        let app = h.state_mut();
        app.items.get_mut("p1").unwrap().extend([clip.clone(), other.clone()]);
        let p = app.playlists[0].clone();
        app.open_dialog(super::Dialog::videos_results(vec![(p.clone(), clip), (p, other)]));
    }
    h.run();
    h.get_by_label("Find music versions").click();
    h.run();
    // Not logged in here, so the search job fails (and «Searching…» clears);
    // give the dialog the worker's answer directly.
    let song =
        Track { id: "500".into(), title: "Hit song".into(), artists: vec!["Artist".into()], ..Default::default() };
    match &mut h.state_mut().dialog {
        Some(super::Dialog::Videos(v)) => {
            v.set_music(vec![("occ8".into(), Some(song)), ("occ9".into(), None)]);
        }
        _ => panic!("videos dialog should stay open"),
    }
    h.run();
    h.get_by_label("Play preview: Hit song — Artist");
    h.get_by_label("No music version found.");
    // The video with a song is ticked to be replaced; the other is left alone.
    h.get_by_label("Replace 1, remove 0").click();
    h.run();
    assert!(h.state().dialog.is_none());
    wait_for_log(&mut h, "Replacing 1");
    assert_eq!(h.state().undo["p1"].len(), 1, "undoable");
}

fn better_hit(app: &App, n: usize, tags: &[&str], explicit: bool, better: Option<Track>) -> crate::worker::BetterHit {
    let item = app.items["p1"][n].clone();
    let mut track = item.track.clone().unwrap();
    track.media_tags = tags.iter().map(|s| (*s).to_owned()).collect();
    track.explicit = explicit;
    crate::worker::BetterHit { playlist: app.playlists[0].clone(), item, track, better }
}

#[test]
fn better_sound_is_listed_with_its_quality_and_swapped() {
    let mut h = harness();
    logged_in(&mut h);
    {
        let app = h.state_mut();
        let hires = Track {
            id: "700".into(),
            title: "First song".into(),
            artists: vec!["Artist".into()],
            album: Some("Deluxe".into()),
            media_tags: vec!["HIRES_LOSSLESS".into(), "LOSSLESS".into()],
            ..Default::default()
        };
        let hit = better_hit(app, 0, &["LOSSLESS"], false, Some(hires));
        let mode = crate::worker::BetterMode::Quality { atmos: false };
        app.open_dialog(super::Dialog::better_results(mode, vec![hit]));
    }
    h.run();
    h.get_by_label("1 songs in 1 playlists can sound better.");
    // The quality is written out, not only shown by an icon or colour.
    h.get_by_role_and_label(Role::CheckBox, "First song — Artist (Lossless)");
    h.get_by_label("Replace with First song — Artist (Deluxe) · Hi-Res lossless");
    h.get_by_label("Play preview: First song — Artist (Lossless)");
    h.get_by_label("Play preview: First song — Artist (Deluxe) · Hi-Res lossless");
    h.get_by_label("Open in TIDAL: First song — Artist (Deluxe) · Hi-Res lossless");
    h.get_by_label("Replace 1").click();
    h.run();
    assert!(h.state().dialog.is_none());
    wait_for_log(&mut h, "Replacing 1");
    assert_eq!(h.state().undo["p1"].len(), 1, "undoable");
}

#[test]
fn explicit_songs_are_swapped_or_removed() {
    let mut h = harness();
    logged_in(&mut h);
    {
        let app = h.state_mut();
        let clean = Track {
            id: "800".into(),
            title: "First song".into(),
            artists: vec!["Artist".into()],
            ..Default::default()
        };
        let hits = vec![better_hit(app, 0, &[], true, Some(clean)), better_hit(app, 1, &[], true, None)];
        app.open_dialog(super::Dialog::better_results(crate::worker::BetterMode::Clean, hits));
    }
    h.run();
    h.get_by_label("2 explicit songs in 1 playlists.");
    h.get_by_label("No clean version found.");
    // Only the song with a clean version starts ticked; tick the other to remove it.
    h.get_by_label("Replace 1, remove 0");
    h.get_by_role_and_label(Role::CheckBox, "Second song — Artist").click();
    h.run();
    h.get_by_label("Replace 1, remove 1").click();
    h.run();
    assert!(h.state().dialog.is_none());
    assert!(!h.state().items["p1"].iter().any(|i| i.item_id == "occ2"), "removed");
    wait_for_log(&mut h, "Replacing 1");
}

#[test]
fn better_setup_offers_atmos_only_for_sound_quality() {
    let mut h = harness();
    logged_in(&mut h);
    let mode = crate::worker::BetterMode::Quality { atmos: false };
    h.state_mut().open_dialog(super::Dialog::better_setup(mode, Some("p1".into())));
    h.run();
    assert!(h.get_by_label("Only «Road trip»").accesskit_node().is_focused());
    h.get_by_role_and_label(Role::CheckBox, "Count Dolby Atmos as the best quality");
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().dialog.is_none());
    h.state_mut().open_dialog(super::Dialog::better_setup(crate::worker::BetterMode::Clean, None));
    h.run();
    assert_eq!(h.query_all_by_label("Count Dolby Atmos as the best quality").count(), 0);
}
