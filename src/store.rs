//! Settings and local caches under the per-user config/data directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::{ItemKind, ItemRef, Track};

/// Writes `data` to `path` atomically (temp file + rename), readable only by
/// the current user, so a crash never leaves a half-written file behind.
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    // A leftover temp file would keep its old permissions; start fresh.
    let _ = std::fs::remove_file(&tmp);
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, path)
}

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
    /// Unavailable-songs scan also asks for playback (finds GEO_RESTRICTED).
    pub geo_check: bool,
    pub spotify_client_id: String,
    /// Spotify requires a loopback IP literal for local apps (not "localhost").
    pub spotify_redirect_uri: String,
    /// Linux: keep a menu entry and icon in `~/.local/share` (the window icon on Wayland).
    pub menu_entry: bool,
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
            geo_check: false,
            spotify_client_id: String::new(),
            spotify_redirect_uri: "http://127.0.0.1:8766/callback".into(),
            menu_entry: true,
        }
    }
}

pub struct Paths {
    pub config_file: PathBuf,
    pub token_file: PathBuf,
    pub spotify_token_file: PathBuf,
    pub cache_file: PathBuf,
    pub backup_dir: PathBuf,
    /// Where desktops look for menu entries and icons (`~/.local/share`).
    pub share_dir: PathBuf,
}

impl Paths {
    /// Per-user directories, or `$TIDAL_EDITOR_HOME` when set (portable use and tests).
    pub fn new() -> Self {
        if let Some(home) = std::env::var_os("TIDAL_EDITOR_HOME") {
            let home = PathBuf::from(home);
            return Self::from_dirs(home.join("config"), home.join("data"), home.join("share"));
        }
        let dirs = directories::ProjectDirs::from("guru", "freberg", "tidal-playlist-editor");
        let (cfg, data) = match &dirs {
            Some(d) => (d.config_dir().to_path_buf(), d.data_dir().to_path_buf()),
            None => (PathBuf::from("."), PathBuf::from(".")),
        };
        let share = directories::BaseDirs::new().map(|b| b.data_dir().to_path_buf()).unwrap_or_else(|| data.clone());
        Self::from_dirs(cfg, data, share)
    }

    fn from_dirs(cfg: PathBuf, data: PathBuf, share: PathBuf) -> Self {
        Self {
            config_file: cfg.join("config.toml"),
            token_file: cfg.join("tokens.json"),
            spotify_token_file: cfg.join("spotify-tokens.json"),
            cache_file: data.join("track-cache.json"),
            backup_dir: data.join("backups"),
            share_dir: share,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_config_is_kept_aside_and_saves_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::from_dirs(dir.path().join("c"), dir.path().join("d"), dir.path().join("s"));
        std::fs::create_dir_all(dir.path().join("c")).unwrap();
        std::fs::write(&paths.config_file, "client_id = \"keep me\"\nvolume = \"loud\"\n").unwrap();
        let cfg = Config::load(&paths);
        assert_eq!(cfg.client_id, "");
        let bak = std::fs::read_to_string(paths.config_file.with_extension("toml.bak")).unwrap();
        assert!(bak.contains("keep me"));
        cfg.save(&paths);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&paths.config_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn cache_keeps_tracks_and_videos_apart() {
        let mut c = TrackCache::default();
        let t = Track { id: "1".into(), title: "Song".into(), ..Default::default() };
        let v = Track { id: "1".into(), kind: ItemKind::Video, title: "Clip".into(), ..Default::default() };
        c.remember([&t, &v]);
        assert_eq!(c.get(&ItemRef::track("1")).unwrap().title, "Song");
        assert_eq!(c.get(&ItemRef { kind: ItemKind::Video, id: "1".into() }).unwrap().title, "Clip");
    }
}

impl Config {
    /// Loads the settings. A file that exists but cannot be parsed is kept as
    /// `config.toml.bak` so the next save does not destroy the user's ids.
    pub fn load(paths: &Paths) -> Self {
        let Ok(text) = std::fs::read_to_string(&paths.config_file) else { return Self::default() };
        match toml::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("could not read config, keeping a copy as config.toml.bak: {e}");
                let _ = std::fs::copy(&paths.config_file, paths.config_file.with_extension("toml.bak"));
                Self::default()
            }
        }
    }

    /// Saves atomically with owner-only permissions (it may hold a client secret).
    pub fn save(&self, paths: &Paths) {
        let write = || -> std::io::Result<()> {
            let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
            write_private(&paths.config_file, text.as_bytes())
        };
        if let Err(e) = write() {
            tracing::warn!("could not save config: {e}");
        }
    }

    pub fn has_client(&self) -> bool {
        !self.client_id.trim().is_empty()
    }

    pub fn has_spotify(&self) -> bool {
        !self.spotify_client_id.trim().is_empty()
    }
}

/// Remembers track metadata so items that later become unavailable (and are
/// then no longer returned by the API) can still be named and replaced.
/// Tracks are keyed by id, videos by `video:<id>` (ids are per kind).
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
            let key = Self::key(t.kind, &t.id);
            if self.tracks.get(&key) != Some(t) {
                self.tracks.insert(key, t.clone());
                self.dirty = true;
            }
        }
    }

    fn key(kind: ItemKind, id: &str) -> String {
        match kind {
            ItemKind::Track => id.to_owned(),
            ItemKind::Video => format!("video:{id}"),
        }
    }

    pub fn get(&self, r: &ItemRef) -> Option<Track> {
        self.tracks.get(&Self::key(r.kind, &r.id)).filter(|t| t.kind == r.kind).cloned()
    }

    pub fn save_if_dirty(&mut self, paths: &Paths) {
        if !self.dirty {
            return;
        }
        // Stays dirty on failure, so the next save tries again.
        match write_private(&paths.cache_file, &serde_json::to_vec(self).expect("json")) {
            Ok(()) => self.dirty = false,
            Err(e) => tracing::warn!("could not save track cache: {e}"),
        }
    }
}
