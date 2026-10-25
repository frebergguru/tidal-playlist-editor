#![cfg_attr(windows, windows_subsystem = "windows")]

mod api;
mod i18n;
mod ops;
mod player;
mod store;
mod ui;
mod worker;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "tidal_editor=info,warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TIDAL Playlist Editor")
            .with_app_id("tidal-playlist-editor")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native("TIDAL Playlist Editor", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc)))))
}
