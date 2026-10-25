//! Playlist export and import formats: JSON (full fidelity), CSV, M3U8, XSPF.

use std::path::Path;

use quick_xml::events::Event as XmlEvent;
use serde::{Deserialize, Serialize};

use crate::api::{AccessType, ItemKind, ItemRef, Playlist, PlaylistItem, Track};

pub const FORMAT_ID: &str = "tidal-playlist-editor";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Csv,
    M3u8,
    Xspf,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 4] = [ExportFormat::Json, ExportFormat::Csv, ExportFormat::M3u8, ExportFormat::Xspf];

    pub fn ext(self) -> &'static str {
        match self {
            ExportFormat::Json => "json",
            ExportFormat::Csv => "csv",
            ExportFormat::M3u8 => "m3u8",
            ExportFormat::Xspf => "xspf",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Json => "JSON",
            ExportFormat::Csv => "CSV",
            ExportFormat::M3u8 => "M3U8",
            ExportFormat::Xspf => "XSPF",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExportFile {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub playlists: Vec<ExportPlaylist>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExportPlaylist {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub access_type: AccessType,
    pub items: Vec<ExportItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExportItem {
    #[serde(rename = "type")]
    pub kind: ItemKind,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub isrc: Option<String>,
    #[serde(default)]
    pub duration_secs: u32,
    #[serde(default)]
    pub added_at: Option<String>,
    #[serde(default = "yes")]
    pub available: bool,
}

fn yes() -> bool {
    true
}

impl ExportItem {
    pub fn from_item(i: &PlaylistItem, cached: Option<&Track>) -> Self {
        let t = i.track.as_ref().or(cached);
        ExportItem {
            kind: i.r.kind,
            id: i.r.id.clone(),
            title: t.map(Track::full_title).unwrap_or_default(),
            artists: t.map(|t| t.artists.clone()).unwrap_or_default(),
            album: t.and_then(|t| t.album.clone()),
            isrc: t.and_then(|t| t.isrc.clone()),
            duration_secs: t.map_or(0, |t| t.duration_secs),
            added_at: i.added_at.clone(),
            available: i.available(),
        }
    }

    pub fn item_ref(&self) -> ItemRef {
        ItemRef { kind: self.kind, id: self.id.clone() }
    }

    fn url(&self) -> String {
        format!("https://tidal.com/browse/{}/{}", crate::api::models::singular(self.kind), self.id)
    }
}

impl ExportPlaylist {
    pub fn new(p: &Playlist, items: &[PlaylistItem], cache: &dyn Fn(&ItemRef) -> Option<Track>) -> Self {
        ExportPlaylist {
            id: Some(p.id.clone()),
            name: p.name.clone(),
            description: p.description.clone(),
            access_type: p.access_type,
            items: items.iter().map(|i| ExportItem::from_item(i, cache(&i.r).as_ref())).collect(),
        }
    }

    pub fn refs(&self) -> Vec<ItemRef> {
        self.items.iter().map(ExportItem::item_ref).collect()
    }
}

pub fn export_file(playlists: Vec<ExportPlaylist>) -> ExportFile {
    ExportFile { format: FORMAT_ID.into(), version: 1, exported_at: chrono::Utc::now().to_rfc3339(), playlists }
}

pub fn to_json(f: &ExportFile) -> String {
    serde_json::to_string_pretty(f).expect("serialisable")
}

pub fn to_csv(p: &ExportPlaylist) -> String {
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record([
        "position",
        "type",
        "tidal_id",
        "title",
        "artists",
        "album",
        "isrc",
        "duration_secs",
        "added_at",
        "available",
        "url",
    ])
    .expect("csv");
    for (n, i) in p.items.iter().enumerate() {
        w.write_record([
            (n + 1).to_string(),
            crate::api::models::singular(i.kind).to_owned(),
            i.id.clone(),
            i.title.clone(),
            i.artists.join("; "),
            i.album.clone().unwrap_or_default(),
            i.isrc.clone().unwrap_or_default(),
            i.duration_secs.to_string(),
            i.added_at.clone().unwrap_or_default(),
            i.available.to_string(),
            i.url(),
        ])
        .expect("csv");
    }
    String::from_utf8(w.into_inner().expect("csv flush")).expect("utf8")
}

pub fn to_m3u8(p: &ExportPlaylist) -> String {
    let mut s = format!("#EXTM3U\n#PLAYLIST:{}\n", p.name.replace('\n', " "));
    for i in &p.items {
        let who = i.artists.join(", ");
        let label = if who.is_empty() { i.title.clone() } else { format!("{who} - {}", i.title) };
        s.push_str(&format!("#EXTINF:{},{}\n{}\n", i.duration_secs, label.replace('\n', " "), i.url()));
    }
    s
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn to_xspf(p: &ExportPlaylist) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<playlist version=\"1\" xmlns=\"http://xspf.org/ns/0/\">\n",
    );
    s.push_str(&format!("  <title>{}</title>\n", xml_escape(&p.name)));
    if !p.description.is_empty() {
        s.push_str(&format!("  <annotation>{}</annotation>\n", xml_escape(&p.description)));
    }
    s.push_str("  <trackList>\n");
    for i in &p.items {
        s.push_str("    <track>\n");
        s.push_str(&format!("      <location>{}</location>\n", xml_escape(&i.url())));
        if let Some(isrc) = &i.isrc {
            s.push_str(&format!("      <identifier>isrc:{}</identifier>\n", xml_escape(isrc)));
        }
        s.push_str(&format!("      <title>{}</title>\n", xml_escape(&i.title)));
        s.push_str(&format!("      <creator>{}</creator>\n", xml_escape(&i.artists.join(", "))));
        if let Some(a) = &i.album {
            s.push_str(&format!("      <album>{}</album>\n", xml_escape(a)));
        }
        if i.duration_secs > 0 {
            s.push_str(&format!("      <duration>{}</duration>\n", u64::from(i.duration_secs) * 1000));
        }
        s.push_str("    </track>\n");
    }
    s.push_str("  </trackList>\n</playlist>\n");
    s
}

pub fn render(p: &ExportPlaylist, fmt: ExportFormat) -> String {
    match fmt {
        ExportFormat::Json => to_json(&export_file(vec![p.clone()])),
        ExportFormat::Csv => to_csv(p),
        ExportFormat::M3u8 => to_m3u8(p),
        ExportFormat::Xspf => to_xspf(p),
    }
}

/// A file-name-safe version of a playlist name.
/// Also valid on Windows: no reserved device names, no trailing dot or space.
pub fn safe_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let s: String = s.trim().trim_matches('.').chars().take(100).collect();
    let s = s.trim_end_matches([' ', '.']).to_owned();
    if s.is_empty() {
        return "playlist".into();
    }
    let stem = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved { format!("_{s}") } else { s }
}

// ------------------------------------------------------------------ import

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportRow {
    pub kind: ItemKind,
    pub id: Option<String>,
    pub isrc: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_secs: Option<u32>,
}

impl ImportRow {
    pub fn describe(&self) -> String {
        let who = self.artists.join(", ");
        let mut s = match (who.is_empty(), self.title.is_empty()) {
            (false, false) => format!("{who} – {}", self.title),
            (true, false) => self.title.clone(),
            (false, true) => who,
            (true, true) => String::new(),
        };
        if s.is_empty() {
            s = self
                .isrc
                .clone()
                .map(|i| format!("ISRC {i}"))
                .or(self.id.clone().map(|i| format!("TIDAL {i}")))
                .unwrap_or_default();
        }
        s
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportPlaylist {
    pub name: String,
    pub description: String,
    pub access_type: AccessType,
    pub rows: Vec<ImportRow>,
    /// Set when the file is our JSON and every item carries a TIDAL id.
    pub exact: bool,
}

/// Pulls a TIDAL track/video id out of a URL like `https://tidal.com/browse/track/123`.
/// Also takes app links (`tidal://track/123`). Local file paths never match.
pub fn tidal_id_from_url(s: &str) -> Option<(ItemKind, String)> {
    let u = url::Url::parse(s.trim()).ok()?;
    let mut segs: Vec<&str> = u.path_segments().map(|p| p.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    match u.scheme() {
        "https" | "http" => {
            let host = u.host_str()?.to_ascii_lowercase();
            if host != "tidal.com" && !host.ends_with(".tidal.com") {
                return None;
            }
        }
        // `tidal://track/123`: the kind is parsed as the host.
        "tidal" => segs.insert(0, u.host_str()?),
        _ => return None,
    }
    segs.windows(2).find_map(|w| {
        let kind = match w[0] {
            "track" => ItemKind::Track,
            "video" => ItemKind::Video,
            _ => return None,
        };
        let id = w[1];
        (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then(|| (kind, id.to_owned()))
    })
}

fn split_artist_title(s: &str) -> (Vec<String>, String) {
    for sep in [" - ", " – ", " — "] {
        if let Some((a, t)) = s.split_once(sep) {
            return (vec![a.trim().to_owned()], t.trim().to_owned());
        }
    }
    (Vec::new(), s.trim().to_owned())
}

fn split_artists(s: &str) -> Vec<String> {
    s.split(';').map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect()
}

fn parse_duration_text(s: &str, millis: bool) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.contains(':') {
        let mut secs = 0u32;
        for part in s.split(':') {
            secs = secs.checked_mul(60)?.checked_add(part.trim().parse::<f32>().ok()? as u32)?;
        }
        return Some(secs);
    }
    let n: f64 = s.parse().ok()?;
    Some(if millis { (n / 1000.0).round() as u32 } else { n.round() as u32 })
}

/// The text of an import file: UTF-8 (with or without a BOM), UTF-16 with a BOM
/// (Excel's "Unicode text", Apple Music's .txt), or else Windows-1252, which
/// older players write plain .m3u files in.
pub fn decode_text(b: &[u8]) -> String {
    let utf16 = |b: &[u8], from: fn([u8; 2]) -> u16| {
        let units = b.as_chunks::<2>().0.iter().map(|&c| from(c));
        char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
    };
    if let Some(rest) = b.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_owned(),
        Err(_) => b.iter().map(|&c| cp1252(c)).collect(),
    }
}

fn cp1252(c: u8) -> char {
    const HIGH: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D,
        0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
        0x0153, 0x9D, 0x017E, 0x0178,
    ];
    match c {
        0x80..=0x9F => char::from_u32(u32::from(HIGH[usize::from(c - 0x80)])).unwrap_or(char::REPLACEMENT_CHARACTER),
        _ => char::from(c),
    }
}

/// "Artist - Title" from a playlist entry's file name: the last part of a Unix
/// or Windows path or a `file:` URL (percent-decoded), without its extension.
fn label_from_entry(line: &str) -> String {
    let name = line.rsplit(['/', '\\']).next().unwrap_or(line);
    let name = if line.starts_with("file:") {
        String::from_utf8_lossy(&crate::player::dash::percent_decode(name)).into_owned()
    } else {
        name.to_owned()
    };
    let stem = Path::new(&name).file_stem().and_then(|s| s.to_str()).unwrap_or(&name).to_owned();
    stem.replace('_', " ")
}

/// Like [`split_artist_title`], but a leading track number ("01 - Title",
/// "01 - Artist - Title") is not taken for the artist.
fn split_file_label(s: &str) -> (Vec<String>, String) {
    let (artists, title) = split_artist_title(s);
    match artists.first() {
        Some(a) if a.len() <= 3 && a.chars().all(|c| c.is_ascii_digit()) => split_artist_title(&title),
        _ => (artists, title),
    }
}

pub fn parse_import(path: &Path, content: &str) -> Result<Vec<ImportPlaylist>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported").to_owned();
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();
    if ext == "json" || trimmed.starts_with('{') {
        return parse_json(trimmed);
    }
    if matches!(ext.as_str(), "xspf" | "xml") || trimmed.starts_with("<?xml") || trimmed.starts_with("<playlist") {
        return parse_xspf(trimmed, &stem).map(|p| vec![p]);
    }
    if matches!(ext.as_str(), "m3u" | "m3u8") || trimmed.starts_with("#EXTM3U") {
        return Ok(vec![parse_m3u(trimmed, &stem)]);
    }
    if matches!(ext.as_str(), "csv" | "tsv" | "txt") {
        return parse_csv(trimmed, &stem).map(|p| vec![p]);
    }
    Err(format!("unknown file type: .{ext}"))
}

pub fn parse_json(content: &str) -> Result<Vec<ImportPlaylist>, String> {
    let f: ExportFile = serde_json::from_str(content).map_err(|e| format!("JSON: {e}"))?;
    if f.format != FORMAT_ID {
        return Err(format!("JSON: unexpected format {:?}", f.format));
    }
    Ok(f.playlists
        .into_iter()
        .map(|p| ImportPlaylist {
            name: p.name,
            description: p.description,
            access_type: p.access_type,
            exact: true,
            rows: p
                .items
                .into_iter()
                .map(|i| ImportRow {
                    kind: i.kind,
                    id: Some(i.id),
                    isrc: i.isrc,
                    title: i.title,
                    artists: i.artists,
                    album: i.album,
                    duration_secs: (i.duration_secs > 0).then_some(i.duration_secs),
                })
                .collect(),
        })
        .collect())
}

pub fn parse_m3u(content: &str, name: &str) -> ImportPlaylist {
    let mut out = ImportPlaylist { name: name.to_owned(), ..Default::default() };
    let mut pending: Option<(Option<u32>, String)> = None;
    for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("#PLAYLIST:") {
            out.name = rest.trim().to_owned();
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (dur, label) = rest.split_once(',').unwrap_or((rest, ""));
            let dur = dur.split_whitespace().next().and_then(|d| d.parse::<i64>().ok()).filter(|d| *d > 0);
            pending = Some((dur.map(|d| d as u32), label.trim().to_owned()));
        } else if !line.starts_with('#') {
            let (dur, label) = pending.take().unwrap_or((None, String::new()));
            let (artists, title) =
                if label.is_empty() { split_file_label(&label_from_entry(line)) } else { split_artist_title(&label) };
            let (kind, id) = match tidal_id_from_url(line) {
                Some((k, id)) => (k, Some(id)),
                None => (ItemKind::Track, None),
            };
            out.rows.push(ImportRow { kind, id, title, artists, duration_secs: dur, ..Default::default() });
        }
    }
    out
}

/// Text of an entity or character reference event (`&amp;`, `&#233;`).
pub fn ref_text(r: &quick_xml::events::BytesRef) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match r.xml10_content().as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
    .to_owned()
}

pub fn parse_xspf(content: &str, name: &str) -> Result<ImportPlaylist, String> {
    let mut out = ImportPlaylist { name: name.to_owned(), ..Default::default() };
    let mut reader = quick_xml::Reader::from_str(content);
    let mut path: Vec<String> = Vec::new();
    let mut row: Option<ImportRow> = None;
    let mut buf = String::new();
    loop {
        match reader.read_event().map_err(|e| format!("XSPF: {e}"))? {
            XmlEvent::Start(e) => {
                let tag = e.local_name().as_ref().to_owned();
                if tag == "track" {
                    row = Some(ImportRow::default());
                }
                path.push(tag);
                buf.clear();
            }
            XmlEvent::Text(t) => buf.push_str(&t.xml10_content()),
            XmlEvent::CData(t) => buf.push_str(&t.into_inner()),
            XmlEvent::GeneralRef(r) => buf.push_str(&ref_text(&r)),
            XmlEvent::End(_) => {
                let text = buf.trim().to_owned();
                buf.clear();
                let tag = path.last().map(String::as_str).unwrap_or("");
                let parent = path.len().checked_sub(2).map(|i| path[i].as_str()).unwrap_or("");
                match (&mut row, parent, tag) {
                    (Some(r), "track", "title") => r.title = text,
                    (Some(r), "track", "creator") => r.artists = split_artists(&text),
                    (Some(r), "track", "album") => r.album = Some(text),
                    (Some(r), "track", "duration") => r.duration_secs = parse_duration_text(&text, true),
                    (Some(r), "track", "location") | (Some(r), "track", "identifier") => {
                        if let Some((k, id)) = tidal_id_from_url(&text) {
                            r.kind = k;
                            r.id = Some(id);
                        } else if let Some(isrc) = text.strip_prefix("isrc:") {
                            r.isrc = Some(isrc.to_owned());
                        }
                    }
                    (None, "playlist", "title") => out.name = text,
                    (None, "playlist", "annotation") => out.description = text,
                    _ => {}
                }
                if path.pop().as_deref() == Some("track")
                    && let Some(r) = row.take()
                {
                    out.rows.push(r);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn norm_header(h: &str) -> String {
    h.to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == '(' || *c == ')').collect()
}

pub fn parse_csv(content: &str, name: &str) -> Result<ImportPlaylist, String> {
    let first = content.lines().next().unwrap_or("");
    // Ties (including "no delimiter at all") resolve to the last candidate, a comma.
    let delim = ['\t', ';', ','].into_iter().max_by_key(|d| first.matches(*d).count()).unwrap_or(',') as u8;
    let mut rdr = csv::ReaderBuilder::new().delimiter(delim).flexible(true).from_reader(content.as_bytes());
    let headers: Vec<String> = rdr.headers().map_err(|e| format!("CSV: {e}"))?.iter().map(norm_header).collect();
    let find = |names: &[&str]| headers.iter().position(|h| names.contains(&h.as_str()));
    let c_id = find(&["tidalid", "tidaltrackid", "trackid", "id"]);
    let c_type = find(&["type", "kind"]);
    let c_isrc = find(&["isrc"]);
    let c_title = find(&["title", "trackname", "name", "track", "song", "songname"]);
    let c_artist = find(&["artists", "artist", "artistname(s)", "artistnames", "artistname", "creator"]);
    let c_album = find(&["album", "albumname", "albumtitle"]);
    let c_dur_ms = find(&["durationms", "duration(ms)", "tracklengthms"]);
    let c_dur = find(&["durationsecs", "duration", "length", "time"]);
    let c_url = find(&["url", "link", "location", "trackurl"]);
    if c_id.is_none() && c_isrc.is_none() && c_title.is_none() && c_url.is_none() {
        return Err("CSV: no id, ISRC, URL or title column found".into());
    }
    let mut out = ImportPlaylist { name: name.to_owned(), ..Default::default() };
    for rec in rdr.records() {
        let rec = rec.map_err(|e| format!("CSV: {e}"))?;
        let get =
            |c: Option<usize>| c.and_then(|i| rec.get(i)).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let mut row = ImportRow {
            kind: get(c_type).and_then(|t| ItemKind::from_api(&t.to_lowercase())).unwrap_or(ItemKind::Track),
            id: get(c_id).filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')),
            isrc: get(c_isrc),
            title: get(c_title).unwrap_or_default(),
            artists: get(c_artist).map(|a| split_artists(&a)).unwrap_or_default(),
            album: get(c_album),
            duration_secs: get(c_dur_ms)
                .and_then(|d| parse_duration_text(&d, true))
                .or_else(|| get(c_dur).and_then(|d| parse_duration_text(&d, false))),
        };
        if let Some((k, id)) = get(c_url).as_deref().and_then(tidal_id_from_url) {
            row.kind = k;
            row.id.get_or_insert(id);
        }
        if row.id.is_some() || row.isrc.is_some() || !row.title.is_empty() {
            out.rows.push(row);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ExportPlaylist {
        ExportPlaylist {
            id: Some("p1".into()),
            name: "Mix & <Match>".into(),
            description: "desc".into(),
            access_type: AccessType::Public,
            items: vec![
                ExportItem {
                    kind: ItemKind::Track,
                    id: "123".into(),
                    title: "Song, \"One\"".into(),
                    artists: vec!["Earth, Wind & Fire".into(), "B".into()],
                    album: Some("LP".into()),
                    isrc: Some("USAAA0000001".into()),
                    duration_secs: 185,
                    added_at: Some("2024-01-01T00:00:00Z".into()),
                    available: true,
                },
                ExportItem {
                    kind: ItemKind::Video,
                    id: "77".into(),
                    title: "Clip".into(),
                    artists: vec!["C".into()],
                    album: None,
                    isrc: None,
                    duration_secs: 0,
                    added_at: None,
                    available: false,
                },
            ],
        }
    }

    #[test]
    fn json_roundtrip_is_exact() {
        let p = sample();
        let text = render(&p, ExportFormat::Json);
        let back = parse_import(Path::new("x.json"), &text).unwrap();
        assert_eq!(back.len(), 1);
        assert!(back[0].exact);
        assert_eq!(back[0].name, p.name);
        assert_eq!(back[0].rows.len(), 2);
        assert_eq!(back[0].rows[0].id.as_deref(), Some("123"));
        assert_eq!(back[0].rows[1].kind, ItemKind::Video);
        assert_eq!(back[0].rows[0].artists, p.items[0].artists);
    }

    #[test]
    fn csv_roundtrip() {
        let p = sample();
        let back = parse_import(Path::new("x.csv"), &render(&p, ExportFormat::Csv)).unwrap();
        let r = &back[0].rows;
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].id.as_deref(), Some("123"));
        assert_eq!(r[0].title, "Song, \"One\"");
        assert_eq!(r[0].artists, vec!["Earth, Wind & Fire".to_owned(), "B".into()]);
        assert_eq!(r[0].isrc.as_deref(), Some("USAAA0000001"));
        assert_eq!(r[0].duration_secs, Some(185));
        assert_eq!(r[1].kind, ItemKind::Video);
    }

    #[test]
    fn m3u_roundtrip() {
        let p = sample();
        let back = parse_import(Path::new("x.m3u8"), &render(&p, ExportFormat::M3u8)).unwrap();
        assert_eq!(back[0].name, p.name);
        let r = &back[0].rows;
        assert_eq!(r[0].id.as_deref(), Some("123"));
        assert_eq!(r[0].duration_secs, Some(185));
        assert_eq!(r[1].id.as_deref(), Some("77"));
        assert_eq!(r[1].kind, ItemKind::Video);
    }

    #[test]
    fn xspf_roundtrip() {
        let p = sample();
        let back = parse_import(Path::new("x.xspf"), &render(&p, ExportFormat::Xspf)).unwrap();
        assert_eq!(back[0].name, p.name);
        assert_eq!(back[0].description, "desc");
        let r = &back[0].rows;
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].id.as_deref(), Some("123"));
        assert_eq!(r[0].isrc.as_deref(), Some("USAAA0000001"));
        assert_eq!(r[0].title, "Song, \"One\"");
        assert_eq!(r[0].duration_secs, Some(185));
    }

    #[test]
    fn spotify_style_csv() {
        let csv = "Track URI,Track Name,Artist Name(s),Album Name,Duration (ms),ISRC\n\
                   spotify:track:x,Halo,Beyoncé,I Am... Sasha Fierce,261640,USSM10804554\n\
                   spotify:track:y,No Isrc,Someone;Other,,200000,\n";
        let p = parse_import(Path::new("liked.csv"), csv).unwrap();
        assert_eq!(p[0].name, "liked");
        let r = &p[0].rows;
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].title, "Halo");
        assert_eq!(r[0].isrc.as_deref(), Some("USSM10804554"));
        assert_eq!(r[0].duration_secs, Some(262));
        assert_eq!(r[1].artists, vec!["Someone".to_owned(), "Other".into()]);
        assert!(r[1].id.is_none());
    }

    #[test]
    fn plain_m3u_and_urls() {
        let m = "#EXTM3U\n#EXTINF:200,Artist - Title\n/music/a.flac\n/music/Other - Thing.mp3\nhttps://listen.tidal.com/track/42?u\n";
        let p = parse_m3u(m, "x");
        assert_eq!(p.rows[0].artists, vec!["Artist".to_owned()]);
        assert_eq!(p.rows[0].title, "Title");
        assert_eq!(p.rows[1].title, "Thing");
        assert_eq!(p.rows[2].id.as_deref(), Some("42"));
        // Windows paths, file: URLs and track numbers in file names.
        let m = "C:\\Music\\Abba - Waterloo.mp3\nfile:///home/u/Beyonc%C3%A9%20-%20Halo.flac\n/m/01 - Intro.ogg\n/m/02. x/03 - Queen - Bohemian Rhapsody.flac\n";
        let p = parse_m3u(m, "x");
        let got: Vec<(Vec<String>, String)> = p.rows.iter().map(|r| (r.artists.clone(), r.title.clone())).collect();
        assert_eq!(
            got,
            [
                (vec!["Abba".to_owned()], "Waterloo".to_owned()),
                (vec!["Beyoncé".to_owned()], "Halo".to_owned()),
                (vec![], "Intro".to_owned()),
                (vec!["Queen".to_owned()], "Bohemian Rhapsody".to_owned()),
            ]
        );
        assert_eq!(decode_text(b"Beyonc\xe9 \x96 Halo"), "Beyoncé – Halo");
        assert_eq!(decode_text("Beyoncé".as_bytes()), "Beyoncé");
        assert_eq!(decode_text(b"\xff\xfeA\x00\xe9\x00"), "Aé");
        assert_eq!(decode_text(b"\xfe\xff\x00A\x00\xe9"), "Aé");
        assert_eq!(tidal_id_from_url("https://example.com/track/1"), None);
        assert_eq!(safe_file_name("a/b: c?"), "a_b_ c_");
        assert_eq!(safe_file_name("con"), "_con");
        assert_eq!(safe_file_name("COM1.list"), "_COM1.list");
        assert_eq!(safe_file_name("Console"), "Console");
        assert_eq!(tidal_id_from_url("tidal://video/77"), Some((ItemKind::Video, "77".into())));
        assert_eq!(tidal_id_from_url("https://tidal.com/browse/track/5"), Some((ItemKind::Track, "5".into())));
        assert_eq!(tidal_id_from_url("/home/u/tidal-dl/music video/clip.mp4"), None);
        assert_eq!(tidal_id_from_url("https://nottidal.com/track/5"), None);
        assert_eq!(parse_duration_text("999999999:99", false), None);
    }

    #[test]
    fn xspf_cdata_titles() {
        let x = "<playlist><trackList><track><title><![CDATA[A & B]]></title><creator>X</creator></track></trackList></playlist>";
        let p = parse_xspf(x, "x").unwrap();
        assert_eq!(p.rows[0].title, "A & B");
    }
}
