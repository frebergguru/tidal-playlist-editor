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
    h.state_mut().set_text_scale(&ctx, 1.0);
    let app = h.state_mut();
    app.cfg.theme = crate::store::ThemeChoice::System;
    app.restyle(&ctx);
}
