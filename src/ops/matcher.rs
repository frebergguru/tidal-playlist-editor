//! Matches imported rows (id / ISRC / artist + title) to tracks in a catalogue
//! (TIDAL, or Spotify when cloning playlists there).

use std::collections::HashMap;
use std::future::Future;

use crate::api::{Client, ItemKind, Result, Track};

/// A music catalogue that rows can be matched against.
pub trait Catalog: Sync {
    /// True when the rows' `id`s belong to this catalogue (TIDAL ids for TIDAL).
    fn native_ids(&self) -> bool {
        true
    }
    fn lookup_ids(
        &self,
        ids: &[String],
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> impl Future<Output = Result<Vec<Track>>> + Send;
    fn by_isrc(&self, isrc: &str) -> impl Future<Output = Result<Vec<Track>>> + Send;
    fn search(&self, row: &ImportRow) -> impl Future<Output = Result<Vec<Track>>> + Send;
}

impl Catalog for Client {
    async fn lookup_ids(&self, ids: &[String], cancel: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Track>> {
        self.tracks_by_ids(ids, cancel).await
    }
    async fn by_isrc(&self, isrc: &str) -> Result<Vec<Track>> {
        self.tracks_by_isrc(isrc).await
    }
    async fn search(&self, row: &ImportRow) -> Result<Vec<Track>> {
        Ok(Client::search(self, &search_text(row)).await?.tracks)
    }
}
use crate::ops::files::ImportRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMethod {
    Id,
    Isrc,
    Search,
    None,
}

#[derive(Clone, Debug)]
pub struct MatchRow {
    pub row: ImportRow,
    pub matched: Option<Track>,
    pub confidence: f32,
    pub method: MatchMethod,
    /// Ticked rows are added; low-confidence matches start unticked.
    pub include: bool,
    pub alternatives: Vec<Track>,
}

pub const AUTO_INCLUDE: f32 = 0.75;
const MIN_MATCH: f32 = 0.5;

/// Lowercases, strips punctuation and "feat." / remaster noise for comparison.
pub fn normalise(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut cut = lower.as_str();
    for marker in [" feat.", " feat ", " ft.", " featuring ", " (feat", " [feat"] {
        if let Some(i) = cut.find(marker) {
            cut = &cut[..i];
        }
    }
    let mut out = String::with_capacity(cut.len());
    let mut depth = 0;
    for c in cut.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() => out.push(c),
            _ => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
        }
    }
    let out = out.trim().to_owned();
    // A title that is only a bracketed part ("(Intro)") would vanish entirely.
    if out.is_empty() { lower.chars().filter(|c| c.is_alphanumeric()).collect() } else { out }
}

/// Words that mark a part of a title as describing the video, not the song.
const VIDEO_WORDS: [&str; 10] =
    ["video", "visualizer", "visualiser", "lyric", "lyrics", "official", "audio", "4k", "hd", "mv"];

fn is_video_part(part: &str) -> bool {
    part.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|w| VIDEO_WORDS.contains(&w))
}

/// The song's title in a music video's title: drops parts such as
/// "(Official Video)", "[4K]", " - Official Music Video" or " | Lyric Video".
/// "Live" and other version words are kept, so a live video finds a live recording.
pub fn music_title(video_title: &str) -> String {
    let mut out = String::with_capacity(video_title.len());
    let mut rest = video_title;
    // Bracketed parts that describe the video.
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest[open..].starts_with('(') { ')' } else { ']' };
        let Some(len) = rest[open..].find(close) else { break };
        let part = &rest[open..open + len + 1];
        out.push_str(&rest[..open]);
        if !is_video_part(part) {
            out.push_str(part);
        }
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    // A trailing " - …" or " | …" that describes the video.
    for sep in [" | ", " - ", " – ", " — "] {
        if let Some(i) = out.rfind(sep)
            && is_video_part(&out[i + sep.len()..])
        {
            out.truncate(i);
        }
    }
    let clean = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() { video_title.trim().to_owned() } else { clean }
}

/// A small extra (under 0.1) for choosing between candidates that score alike,
/// since [`score`] ignores bracketed parts and is satisfied by one shared artist:
/// prefer the exact title (not a remix or live version of it), then the
/// candidate that has more of the row's artists.
pub fn tie_break(row: &ImportRow, t: &Track) -> f32 {
    let exact = t.full_title().trim().eq_ignore_ascii_case(row.title.trim());
    let shared = if row.artists.is_empty() {
        0.0
    } else {
        let have = row.artists.iter().filter(|a| t.artists.iter().any(|b| normalise(a) == normalise(b))).count();
        have as f32 / row.artists.len() as f32
    };
    0.05 * f32::from(u8::from(exact)) + 0.04 * shared
}

fn sim(a: &str, b: &str) -> f32 {
    let (a, b) = (normalise(a), normalise(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let jw = strsim::jaro_winkler(&a, &b) as f32;
    if a.contains(&b) || b.contains(&a) { jw.max(0.9) } else { jw }
}

/// 0..=1 similarity between an import row and a candidate track.
pub fn score(row: &ImportRow, t: &Track) -> f32 {
    let title = sim(&row.title, &t.title).max(sim(&row.title, &t.full_title()));
    let mut s = if row.artists.is_empty() {
        title
    } else {
        let mine = row.artists.join(" ");
        let theirs = t.artists.join(" ");
        let artist = row
            .artists
            .iter()
            .flat_map(|a| t.artists.iter().map(move |b| sim(a, b)))
            .fold(sim(&mine, &theirs), f32::max);
        0.6 * title + 0.4 * artist
    };
    if let (Some(d), true) = (row.duration_secs, t.duration_secs > 0) {
        let diff = d.abs_diff(t.duration_secs);
        if diff <= 3 {
            s += 0.05;
        } else if diff > 20 {
            s -= 0.15;
        }
    }
    if let (Some(a), Some(b)) = (&row.album, &t.album)
        && sim(a, b) > 0.9
    {
        s += 0.03;
    }
    s.clamp(0.0, 1.0)
}

fn best(row: &ImportRow, mut cands: Vec<Track>) -> (Option<Track>, f32, Vec<Track>) {
    cands.sort_by(|a, b| score(row, b).total_cmp(&score(row, a)).then(b.streamable().cmp(&a.streamable())));
    let top = cands.first().map(|t| score(row, t)).unwrap_or(0.0);
    if top < MIN_MATCH {
        return (None, top, cands.into_iter().take(5).collect());
    }
    let mut it = cands.into_iter();
    let first = it.next();
    (first, top, it.take(4).collect())
}

pub fn search_text(row: &ImportRow) -> String {
    let artist = row.artists.first().map(String::as_str).unwrap_or("");
    let title = normalise(&row.title);
    format!("{artist} {title}").trim().to_owned()
}

pub async fn match_rows<C: Catalog>(
    client: &C,
    rows: Vec<ImportRow>,
    progress: &(dyn Fn(usize) + Sync),
    cancel: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<MatchRow>> {
    let mut out: Vec<MatchRow> = rows
        .into_iter()
        .map(|row| MatchRow {
            row,
            matched: None,
            confidence: 0.0,
            method: MatchMethod::None,
            include: false,
            alternatives: Vec::new(),
        })
        .collect();

    // 1. Exact ids (tracks via lookup; videos are accepted as-is) — only when
    //    the ids belong to this catalogue.
    let native = client.native_ids();
    let ids: Vec<String> = if native {
        out.iter().filter(|m| m.row.kind == ItemKind::Track).filter_map(|m| m.row.id.clone()).collect()
    } else {
        Vec::new()
    };
    let found: HashMap<String, Track> =
        client.lookup_ids(&ids, cancel).await?.into_iter().map(|t| (t.id.clone(), t)).collect();
    for m in out.iter_mut().filter(|_| native) {
        let Some(id) = &m.row.id else { continue };
        let hit = match m.row.kind {
            // An id from another service can name an unrelated TIDAL track, so
            // when the row has a title it must also look like the same song.
            ItemKind::Track => {
                found.get(id).filter(|t| m.row.title.is_empty() || score(&m.row, t) >= MIN_MATCH).cloned()
            }
            ItemKind::Video => Some(Track {
                id: id.clone(),
                kind: ItemKind::Video,
                title: m.row.title.clone(),
                artists: m.row.artists.clone(),
                duration_secs: m.row.duration_secs.unwrap_or(0),
                ..Default::default()
            }),
        };
        if let Some(t) = hit {
            m.matched = Some(t);
            m.confidence = 1.0;
            m.method = MatchMethod::Id;
            m.include = true;
        }
    }
    let mut done = out.iter().filter(|m| m.matched.is_some()).count();
    progress(done);

    // 2. ISRC, then 3. free-text search.
    for m in out.iter_mut().filter(|m| m.matched.is_none()) {
        if cancel() {
            return Err(crate::api::ApiError::Cancelled);
        }
        if let Some(isrc) = m.row.isrc.clone() {
            let cands = client.by_isrc(&isrc).await.unwrap_or_default();
            if !cands.is_empty() {
                let (hit, conf, alts) = best(&m.row, cands);
                // Same ISRC is the same recording, so it is a strong match even
                // when titles are written differently.
                let conf = if m.row.title.is_empty() { 0.95 } else { conf.max(0.9) };
                if let Some(t) = hit.or_else(|| alts.first().cloned()) {
                    m.alternatives = alts.into_iter().filter(|a| a.id != t.id).collect();
                    m.matched = Some(t);
                    m.confidence = conf;
                    m.method = MatchMethod::Isrc;
                    m.include = true;
                }
            }
        }
        if m.matched.is_none() && !m.row.title.is_empty() && !search_text(&m.row).is_empty() {
            // A query TIDAL refuses (400/404) only means no match for this row.
            let res = match client.search(&m.row).await {
                Err(e) if matches!(e.status(), Some(400 | 404)) => {
                    tracing::info!("search refused for {:?}: {e}", m.row.title);
                    Vec::new()
                }
                other => other?,
            };
            let (hit, conf, alts) = best(&m.row, res);
            m.alternatives = alts;
            if let Some(t) = hit {
                m.matched = Some(t);
                m.confidence = conf;
                m.method = MatchMethod::Search;
                m.include = conf >= AUTO_INCLUDE;
            }
        }
        done += 1;
        progress(done);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str, dur: u32) -> Track {
        Track {
            id: title.into(),
            title: title.into(),
            artists: vec![artist.into()],
            duration_secs: dur,
            ..Default::default()
        }
    }

    fn row(title: &str, artist: &str, dur: Option<u32>) -> ImportRow {
        ImportRow { title: title.into(), artists: vec![artist.into()], duration_secs: dur, ..Default::default() }
    }

    #[test]
    fn tie_break_prefers_exact_title_and_all_artists() {
        let row = ImportRow {
            kind: crate::api::ItemKind::Track,
            id: None,
            isrc: None,
            title: "It Ain't Me".into(),
            artists: vec!["Kygo".into(), "Selena Gomez".into()],
            album: None,
            duration_secs: None,
        };
        let t = |title: &str, artists: &[&str]| Track {
            title: title.into(),
            artists: artists.iter().map(|a| (*a).into()).collect(),
            ..Default::default()
        };
        let original = t("It Ain't Me", &["Kygo", "Selena Gomez"]);
        let remix = t("It Ain't Me (Tiësto's AFTR:HRS Remix)", &["Kygo", "Selena Gomez"]);
        let one_artist = t("It Ain't Me", &["Selena Gomez"]);
        // The main score cannot tell them apart; the tie-break can.
        assert_eq!(score(&row, &original), score(&row, &remix));
        assert!(tie_break(&row, &original) > tie_break(&row, &remix));
        assert!(tie_break(&row, &original) > tie_break(&row, &one_artist));
    }

    #[test]
    fn music_title_drops_video_words_only() {
        assert_eq!(music_title("Song (Official Video)"), "Song");
        assert_eq!(music_title("Song [4K] (Remastered)"), "Song (Remastered)");
        assert_eq!(music_title("Song - Official Music Video"), "Song");
        assert_eq!(music_title("Song | Lyric Video"), "Song");
        // Real TIDAL video titles.
        assert_eq!(music_title("It Ain't Me (Official Audio)"), "It Ain't Me");
        assert_eq!(
            music_title("Say Say Say (feat. Paul McCartney & Michael Jackson) (Lyric Video)"),
            "Say Say Say (feat. Paul McCartney & Michael Jackson)"
        );
        assert_eq!(music_title("Song (Live at Wembley)"), "Song (Live at Wembley)");
        assert_eq!(music_title("Song - Radio Edit"), "Song - Radio Edit");
        assert_eq!(music_title("Video Killed the Radio Star"), "Video Killed the Radio Star");
        assert_eq!(music_title("(Official Video)"), "(Official Video)");
    }

    #[test]
    fn normalising() {
        assert_eq!(normalise("Halo (Remastered 2011) feat. X"), "halo");
        assert_eq!(normalise("Don't Stop Me Now"), "don t stop me now");
        assert_eq!(normalise("(Intro)"), "intro");
    }

    #[test]
    fn scoring_prefers_right_track() {
        let r = row("Bohemian Rhapsody", "Queen", Some(355));
        let good = t("Bohemian Rhapsody - Remastered 2011", "Queen", 354);
        let cover = t("Bohemian Rhapsody", "Some Cover Band", 300);
        let other = t("Radio Ga Ga", "Queen", 343);
        assert!(score(&r, &good) > 0.85, "{}", score(&r, &good));
        assert!(score(&r, &good) > score(&r, &cover));
        assert!(score(&r, &other) < AUTO_INCLUDE);
        let (hit, conf, _) = best(&r, vec![other, cover, good.clone()]);
        assert_eq!(hit.unwrap().id, good.id);
        assert!(conf >= AUTO_INCLUDE);
    }

    #[test]
    fn no_match_below_threshold() {
        let r = row("Completely Different", "Nobody", None);
        let (hit, _, alts) = best(&r, vec![t("Xyz", "Qqq", 100)]);
        assert!(hit.is_none());
        assert_eq!(alts.len(), 1);
    }
}
