//! Settings and local caches under the per-user config/data directories.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::Track;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    Auto,
    English,
    Norwegian,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum MotionChoice {
    #[default]
    System,
    Reduced,
    Full,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum OpenIn {
    #[default]
    App,
    Web,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub language: Language,
    pub theme: ThemeChoice,
    pub motion: MotionChoice,
    pub text_scale: f32,
    pub open_in: OpenIn,
    pub volume: f32,
    pub last_playlist: Option<String>,
    pub export_dir: Option<PathBuf>,
    pub show_search: bool,
    pub strict_availability: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            redirect_uri: "http://localhost:8765/callback".into(),
            language: Language::Auto,
            theme: ThemeChoice::System,
            motion: MotionChoice::System,
            text_scale: 1.0,
            open_in: OpenIn::App,
            volume: 0.8,
            last_playlist: None,
            export_dir: None,
            show_search: true,
            strict_availability: false,
        }
    }
}

pub struct Paths {
    pub config_file: PathBuf,
    pub token_file: PathBuf,
    pub cache_file: PathBuf,
    pub backup_dir: PathBuf,
}

impl Paths {
    /// Per-user directories, or `$TIDAL_EDITOR_HOME` when set (portable use and tests).
    pub fn new() -> Self {
        if let Some(home) = std::env::var_os("TIDAL_EDITOR_HOME") {
            let home = PathBuf::from(home);
            return Self::from_dirs(home.join("config"), home.join("data"));
        }
        let dirs = directories::ProjectDirs::from("guru", "freberg", "tidal-playlist-editor");
        let (cfg, data) = match &dirs {
            Some(d) => (d.config_dir().to_path_buf(), d.data_dir().to_path_buf()),
            None => (PathBuf::from("."), PathBuf::from(".")),
        };
        Self::from_dirs(cfg, data)
    }

    fn from_dirs(cfg: PathBuf, data: PathBuf) -> Self {
        Self {
            config_file: cfg.join("config.toml"),
            token_file: cfg.join("tokens.json"),
            cache_file: data.join("track-cache.json"),
            backup_dir: data.join("backups"),
        }
    }
}

impl Config {
    pub fn load(paths: &Paths) -> Self {
        std::fs::read_to_string(&paths.config_file).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) {
        let write = || -> std::io::Result<()> {
            if let Some(d) = paths.config_file.parent() {
                std::fs::create_dir_all(d)?;
            }
            let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
            std::fs::write(&paths.config_file, text)
        };
        if let Err(e) = write() {
            tracing::warn!("could not save config: {e}");
        }
    }

    pub fn has_client(&self) -> bool {
        !self.client_id.trim().is_empty()
    }
}

/// Remembers track metadata so items that later become unavailable (and are
/// then no longer returned by the API) can still be named and replaced.
#[derive(Default, Serialize, Deserialize)]
pub struct TrackCache {
    pub tracks: HashMap<String, Track>,
    #[serde(skip)]
    dirty: bool,
}

impl TrackCache {
    pub fn load(paths: &Paths) -> Self {
        std::fs::read(&paths.cache_file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn remember<'a>(&mut self, tracks: impl IntoIterator<Item = &'a Track>) {
        for t in tracks {
            if self.tracks.get(&t.id) != Some(t) {
                self.tracks.insert(t.id.clone(), t.clone());
                self.dirty = true;
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<Track> {
        self.tracks.get(id).cloned()
    }

    pub fn save_if_dirty(&mut self, paths: &Paths) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let r = paths
            .cache_file
            .parent()
            .map(std::fs::create_dir_all)
            .transpose()
            .and_then(|_| std::fs::write(&paths.cache_file, serde_json::to_vec(self).expect("json")));
        if let Err(e) = r {
            tracing::warn!("could not save track cache: {e}");
        }
    }
}
