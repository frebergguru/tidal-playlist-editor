//! Domain types built from TIDAL JSON:API resources.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    #[default]
    Track,
    Video,
}

impl ItemKind {
    pub fn api_type(self) -> &'static str {
        match self {
            ItemKind::Track => "tracks",
            ItemKind::Video => "videos",
        }
    }

    pub fn from_api(s: &str) -> Option<Self> {
        match s {
            "tracks" | "track" => Some(ItemKind::Track),
            "videos" | "video" => Some(ItemKind::Video),
            _ => None,
        }
    }
}

/// A reference to a track or video in the catalogue.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemRef {
    pub kind: ItemKind,
    pub id: String,
}

impl ItemRef {
    #[cfg(test)]
    pub fn track(id: impl Into<String>) -> Self {
        Self { kind: ItemKind::Track, id: id.into() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Track {
    pub id: String,
    #[serde(default = "default_kind")]
    pub kind: ItemKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isrc: Option<String>,
    #[serde(default)]
    pub duration_secs: u32,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artist_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    /// Deprecated `availability` attribute (STREAM, DJ, STEM). `None` = not reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<Vec<String>>,
    /// Sound quality tags such as `LOSSLESS`, `HIRES_LOSSLESS`, `DOLBY_ATMOS`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_tags: Vec<String>,
    /// TIDAL's popularity score, 0–1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popularity: Option<f32>,
}

fn default_kind() -> ItemKind {
    ItemKind::Track
}

impl Track {
    pub fn item_ref(&self) -> ItemRef {
        ItemRef { kind: self.kind, id: self.id.clone() }
    }

    /// Title including version, e.g. "Song (Remastered 2011)".
    pub fn full_title(&self) -> String {
        match &self.version {
            Some(v) if !v.is_empty() && !self.title.contains(v.as_str()) => {
                format!("{} ({v})", self.title)
            }
            _ => self.title.clone(),
        }
    }

    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }

    /// False when TIDAL reports the item cannot be streamed.
    pub fn streamable(&self) -> bool {
        match &self.availability {
            None => true,
            Some(a) => a.iter().any(|s| s == "STREAM"),
        }
    }

    pub fn web_url(&self) -> String {
        format!("https://tidal.com/browse/{}/{}", singular(self.kind), self.id)
    }

    pub fn app_url(&self) -> String {
        format!("tidal://{}/{}", singular(self.kind), self.id)
    }
}

pub fn singular(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Track => "track",
        ItemKind::Video => "video",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlaylistItem {
    /// Occurrence id inside the playlist (`meta.itemId`), needed for delete/move.
    pub item_id: String,
    pub r: ItemRef,
    pub added_at: Option<String>,
    /// `None` when the resource was not returned, i.e. it is not available to us.
    pub track: Option<Track>,
    /// `meta.replacement.status` when TIDAL substituted another release.
    pub replacement: Option<String>,
}

impl PlaylistItem {
    pub fn available(&self) -> bool {
        self.track.as_ref().is_some_and(|t| t.streamable())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AccessType {
    Public,
    #[default]
    Unlisted,
    Private,
}

impl AccessType {
    pub fn api(self) -> &'static str {
        match self {
            AccessType::Public => "PUBLIC",
            AccessType::Unlisted => "UNLISTED",
            AccessType::Private => "PRIVATE",
        }
    }

    pub fn from_api(s: &str) -> Self {
        match s {
            "PUBLIC" => AccessType::Public,
            "PRIVATE" => AccessType::Private,
            _ => AccessType::Unlisted,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub access_type: AccessType,
    #[serde(default)]
    pub number_of_items: Option<u32>,
    #[serde(default)]
    pub number_of_tracks: Option<u32>,
    #[serde(default)]
    pub number_of_videos: Option<u32>,
    #[serde(default)]
    pub duration_secs: Option<u32>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub playlist_type: Option<String>,
    /// True when the logged-in user owns it (otherwise collaborator).
    #[serde(default = "yes")]
    pub owned: bool,
}

fn yes() -> bool {
    true
}

impl Playlist {
    pub fn web_url(&self) -> String {
        format!("https://tidal.com/browse/playlist/{}", self.id)
    }
    pub fn app_url(&self) -> String {
        format!("tidal://playlist/{}", self.id)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub id: String,
    pub username: String,
    pub country: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Album {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub release_date: Option<String>,
    pub number_of_items: Option<u32>,
    pub album_type: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Artist {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Genre {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub tracks: Vec<Track>,
    pub tracks_next: Option<String>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub did_you_mean: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub uri: String,
    pub full: bool,
    pub drm: bool,
}

/// Parses ISO 8601 durations such as `PT3M5S`, `PT1H2M3.5S` or `P0DT4M`.
pub fn parse_iso_duration(s: &str) -> u32 {
    let mut total = 0f64;
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            'P' => {}
            'T' => in_time = true,
            '0'..='9' | '.' | ',' => num.push(if c == ',' { '.' } else { c }),
            unit => {
                let v: f64 = num.parse().unwrap_or(0.0);
                num.clear();
                total += match (unit, in_time) {
                    ('D', _) => v * 86400.0,
                    ('W', false) => v * 604800.0,
                    ('H', true) => v * 3600.0,
                    ('M', true) => v * 60.0,
                    ('S', true) => v,
                    _ => 0.0,
                };
            }
        }
    }
    total.round() as u32
}

pub fn format_duration(secs: u32) -> String {
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_iso_duration("PT3M5S"), 185);
        assert_eq!(parse_iso_duration("PT1H2M3.6S"), 3724);
        assert_eq!(parse_iso_duration("P1DT1S"), 86401);
        assert_eq!(parse_iso_duration("PT45S"), 45);
        assert_eq!(parse_iso_duration(""), 0);
        assert_eq!(format_duration(185), "3:05");
        assert_eq!(format_duration(3724), "1:02:04");
    }

    #[test]
    fn titles_and_availability() {
        let mut t = Track { title: "Song".into(), version: Some("Live".into()), ..Default::default() };
        assert_eq!(t.full_title(), "Song (Live)");
        assert!(t.streamable());
        t.availability = Some(vec!["DJ".into()]);
        assert!(!t.streamable());
        t.availability = Some(vec!["STREAM".into()]);
        assert!(t.streamable());
    }
}
