//! Better versions of songs: higher sound quality (same recording, by ISRC)
//! and clean versions of explicit songs.

use crate::api::{ItemKind, Track};
use crate::ops::files::ImportRow;
use crate::ops::matcher;

/// Sound quality as a rank: 0 = lossy (no tag), 1 = lossless (CD),
/// 2 = hi-res lossless. With `atmos`, Dolby Atmos outranks every stereo version.
pub fn quality(t: &Track, atmos: bool) -> u8 {
    let has = |tag: &str| t.media_tags.iter().any(|m| m == tag);
    let stereo = if has("HIRES_LOSSLESS") {
        2
    } else if has("LOSSLESS") {
        1
    } else {
        0
    };
    if atmos && has("DOLBY_ATMOS") { 3 } else { stereo }
}

/// True when nothing can rank above this track.
pub fn is_best(t: &Track, atmos: bool) -> bool {
    quality(t, atmos) >= if atmos { 3 } else { 2 }
}

/// A match row describing `t`, for scoring candidates against it.
pub fn row_of(t: &Track) -> ImportRow {
    ImportRow {
        kind: ItemKind::Track,
        id: None,
        isrc: t.isrc.clone(),
        title: t.full_title(),
        artists: t.artists.clone(),
        album: t.album.clone(),
        duration_secs: (t.duration_secs > 0).then_some(t.duration_secs),
    }
}

/// The best-sounding playable track among `cands` (versions of the same
/// recording) that sounds better than `orig`. Among equals it prefers the same
/// title and album (so «Song» does not become «Song (Single Version)»), then
/// the first in TIDAL's order.
pub fn better_quality(orig: &Track, cands: Vec<Track>, atmos: bool) -> Option<Track> {
    let row = row_of(orig);
    let have = quality(orig, atmos);
    cands
        .into_iter()
        .filter(|t| t.id != orig.id && t.kind == ItemKind::Track && t.streamable() && quality(t, atmos) > have)
        // Same recording, but not a single edit or another cut of it.
        .filter(|t| orig.duration_secs == 0 || t.duration_secs.abs_diff(orig.duration_secs) <= 5)
        .map(|t| ((quality(&t, atmos), matcher::score(&row, &t) + matcher::tie_break(&row, &t)), t))
        // `min_by` with the order reversed keeps the first of equals.
        .min_by(|(a, _), (b, _)| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)))
        .map(|(_, t)| t)
}

/// The clean (not explicit) version of `orig` among `cands`: the same song by
/// the same artists, about as long. `None` when no candidate is close enough.
pub fn clean_version(orig: &Track, cands: Vec<Track>) -> Option<Track> {
    let row = row_of(orig);
    let artists: Vec<String> = orig.artists.iter().map(|a| matcher::normalise(a)).collect();
    cands
        .into_iter()
        .filter(|t| t.id != orig.id && t.kind == ItemKind::Track && !t.explicit && t.streamable())
        .filter(|t| orig.duration_secs == 0 || t.duration_secs.abs_diff(orig.duration_secs) <= 15)
        // A cover by someone else is not the clean version.
        .filter(|t| orig.artists.is_empty() || t.artists.iter().any(|a| artists.contains(&matcher::normalise(a))))
        .map(|t| (matcher::score(&row, &t), t))
        .filter(|(s, _)| *s >= 0.85)
        .map(|(s, t)| (s + matcher::tie_break(&row, &t), t))
        .min_by(|a, b| b.0.total_cmp(&a.0))
        .map(|(_, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, title: &str, tags: &[&str], secs: u32) -> Track {
        Track {
            id: id.into(),
            title: title.into(),
            artists: vec!["Queen".into()],
            duration_secs: secs,
            media_tags: tags.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn quality_ranks() {
        assert_eq!(quality(&t("1", "S", &[], 1), false), 0);
        assert_eq!(quality(&t("1", "S", &["LOSSLESS"], 1), false), 1);
        assert_eq!(quality(&t("1", "S", &["HIRES_LOSSLESS", "LOSSLESS"], 1), false), 2);
        let atmos = t("1", "S", &["LOSSLESS", "DOLBY_ATMOS"], 1);
        assert_eq!((quality(&atmos, false), quality(&atmos, true)), (1, 3));
        assert!(is_best(&t("1", "S", &["HIRES_LOSSLESS"], 1), false));
        assert!(!is_best(&t("1", "S", &["HIRES_LOSSLESS"], 1), true));
    }

    #[test]
    fn picks_better_sound_of_the_same_cut() {
        let orig = t("1", "Bohemian Rhapsody", &["LOSSLESS"], 355);
        let cands = vec![
            orig.clone(),
            t("2", "Bohemian Rhapsody", &["LOSSLESS"], 355),
            t("3", "Bohemian Rhapsody (Single Version)", &["HIRES_LOSSLESS"], 355),
            t("4", "Bohemian Rhapsody", &["HIRES_LOSSLESS"], 300),
            t("5", "Bohemian Rhapsody", &["HIRES_LOSSLESS"], 356),
            t("6", "Bohemian Rhapsody", &["HIRES_LOSSLESS"], 355),
        ];
        // 4 is another cut; 5 and 6 tie on title, 5 comes first.
        assert_eq!(better_quality(&orig, cands.clone(), false).map(|t| t.id), Some("5".into()));
        let best = t("9", "Bohemian Rhapsody", &["HIRES_LOSSLESS"], 355);
        assert_eq!(better_quality(&best, cands.clone(), false), None);
        let mut atmos = t("7", "Bohemian Rhapsody", &["LOSSLESS", "DOLBY_ATMOS"], 355);
        let with_atmos: Vec<Track> = cands.iter().cloned().chain([atmos.clone()]).collect();
        assert_eq!(better_quality(&orig, with_atmos.clone(), true).map(|t| t.id), Some("7".into()));
        assert_eq!(better_quality(&orig, with_atmos, false).map(|t| t.id), Some("5".into()));
        atmos.availability = Some(vec!["DJ".into()]);
        assert_eq!(better_quality(&orig, vec![atmos], true), None, "not playable");
    }

    #[test]
    fn finds_the_clean_version() {
        let mut orig = t("1", "Song", &[], 200);
        orig.explicit = true;
        let mut other_artist = t("3", "Song", &[], 200);
        other_artist.artists = vec!["Someone Else".into()];
        let cands = vec![
            orig.clone(),
            t("2", "Completely Different", &[], 200),
            other_artist,
            t("4", "Song", &[], 260),
            t("5", "Song (Clean)", &[], 201),
            t("6", "Song", &[], 199),
        ];
        assert_eq!(clean_version(&orig, cands.clone()).map(|t| t.id), Some("6".into()));
        let without: Vec<Track> = cands.into_iter().filter(|t| t.id != "6" && t.id != "5").collect();
        assert_eq!(clean_version(&orig, without), None);
    }
}
