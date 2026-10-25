//! New playlists from genres: picking fresh songs out of the candidates TIDAL gives.

use std::collections::{HashMap, HashSet};

use crate::api::Track;
use crate::ops::{analysis, recommend::keys};

/// Most songs one generated playlist may get.
pub const MAX_SONGS: usize = 500;

/// Splits `total` songs over `n` genres as evenly as possible.
pub fn split(total: usize, n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    (0..n).map(|i| total / n + usize::from(i < total % n)).collect()
}

/// Streamable candidates in random order, one per song (by id, ISRC and
/// artist + title), without songs whose keys are in `known`.
pub fn fresh(candidates: Vec<Track>, known: &HashSet<String>) -> Vec<Track> {
    let mut seen = HashSet::new();
    let out: Vec<Track> = candidates
        .into_iter()
        .filter(|t| t.streamable())
        .filter(|t| {
            let k = keys(t);
            if k.iter().any(|k| known.contains(k) || seen.contains(k)) {
                return false;
            }
            seen.extend(k);
            true
        })
        .collect();
    analysis::shuffled(&out)
}

/// Keeps tracks whose genres include `genre`; tracks TIDAL gave no genres for
/// count as matching, since the genre search already chose them.
pub fn matches_genre(t: &Track, genre: &str, genres: &HashMap<String, Vec<String>>) -> bool {
    genres.get(&t.id).is_none_or(|g| g.iter().any(|g| g == genre))
}

/// One song from each list in turn, so a mixed playlist alternates genres.
pub fn interleave(lists: Vec<Vec<Track>>) -> Vec<Track> {
    let mut iters: Vec<_> = lists.into_iter().map(Vec::into_iter).collect();
    let mut out = Vec::new();
    loop {
        let before = out.len();
        out.extend(iters.iter_mut().filter_map(Iterator::next));
        if out.len() == before {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, artist: &str, title: &str) -> Track {
        Track { id: id.into(), title: title.into(), artists: vec![artist.into()], ..Default::default() }
    }

    #[test]
    fn splits_evenly() {
        assert_eq!(split(10, 3), [4, 3, 3]);
        assert_eq!(split(2, 3), [1, 1, 0]);
        assert!(split(5, 0).is_empty());
    }

    #[test]
    fn fresh_drops_known_duplicate_and_unplayable() {
        let mut gone = t("4", "D", "Four");
        gone.availability = Some(vec![]);
        let known: HashSet<String> = keys(&t("1", "A", "One")).into_iter().collect();
        let out =
            fresh(vec![t("1", "A", "One"), t("2", "B", "Two"), t("3", "b", "TWO"), gone, t("5", "E", "Five")], &known);
        let mut ids: Vec<&str> = out.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, ["2", "5"]);
    }

    #[test]
    fn genre_check_and_interleave() {
        let g: HashMap<String, Vec<String>> =
            [("1".into(), vec!["rock".into()]), ("2".into(), vec!["pop".into()])].into();
        assert!(matches_genre(&t("1", "A", "x"), "rock", &g));
        assert!(!matches_genre(&t("2", "A", "x"), "rock", &g));
        assert!(matches_genre(&t("3", "A", "x"), "rock", &g));
        let out = interleave(vec![vec![t("a1", "", ""), t("a2", "", "")], vec![t("b1", "", "")]]);
        let ids: Vec<&str> = out.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a1", "b1", "a2"]);
    }
}
