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
            ItemKind::Track => found.get(id).cloned(),
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
        if m.matched.is_none() && !m.row.title.is_empty() {
            let res = client.search(&m.row).await?;
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
