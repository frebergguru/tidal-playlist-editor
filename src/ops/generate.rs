//! New playlists from genres: picking fresh songs out of the candidates TIDAL gives.

use std::collections::HashSet;

use crate::api::{Genre, Track};
use crate::ops::{analysis, recommend::keys};

/// Genres offered in the dialog: (search term, English name, Norwegian name).
/// TIDAL's own genre list and genre tags come back empty for third-party
/// apps, so the songs are found by searching TIDAL for the term (in English,
/// as TIDAL names its editorial playlists).
const GENRES: &[(&str, &str, &str)] = &[
    ("Pop", "Pop", "Pop"),
    ("Rock", "Rock", "Rock"),
    ("Hip-Hop", "Hip-hop", "Hiphop"),
    ("R&B", "R&B", "R&B"),
    ("Soul", "Soul", "Soul"),
    ("Jazz", "Jazz", "Jazz"),
    ("Blues", "Blues", "Blues"),
    ("Classical", "Classical", "Klassisk"),
    ("Electronic", "Electronic", "Elektronisk"),
    ("Dance", "Dance", "Dans"),
    ("House", "House", "House"),
    ("Techno", "Techno", "Techno"),
    ("Country", "Country", "Country"),
    ("Folk", "Folk", "Folk"),
    ("Singer-Songwriter", "Singer-songwriter", "Visesang"),
    ("Indie", "Indie", "Indie"),
    ("Alternative", "Alternative", "Alternativ"),
    ("Metal", "Metal", "Metal"),
    ("Punk", "Punk", "Punk"),
    ("Reggae", "Reggae", "Reggae"),
    ("Latin", "Latin", "Latin"),
    ("K-Pop", "K-pop", "K-pop"),
    ("Afrobeats", "Afrobeats", "Afrobeats"),
    ("Funk", "Funk", "Funk"),
    ("Disco", "Disco", "Disco"),
    ("Gospel", "Gospel", "Gospel"),
    ("Ambient", "Ambient", "Ambient"),
    ("Lo-Fi", "Lo-fi", "Lo-fi"),
    ("Soundtrack", "Film and TV", "Film og TV"),
    ("Kids", "Children's music", "Barnemusikk"),
    ("Christmas", "Christmas", "Jul"),
    ("Norsk", "Norwegian", "Norsk"),
];

/// The built-in genres with names in the UI language; `id` is the search term.
pub fn genres(norwegian: bool) -> Vec<Genre> {
    GENRES
        .iter()
        .map(|(q, en, nb)| Genre { id: (*q).into(), name: (if norwegian { nb } else { en }).to_string() })
        .collect()
}

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
    fn interleaves_and_lists_genres() {
        let g = genres(true);
        assert!(g.iter().any(|g| g.id == "Classical" && g.name == "Klassisk"));
        assert_eq!(g.iter().map(|g| &g.id).collect::<HashSet<_>>().len(), g.len(), "unique ids");
        let out = interleave(vec![vec![t("a1", "", ""), t("a2", "", "")], vec![t("b1", "", "")]]);
        let ids: Vec<&str> = out.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a1", "b1", "a2"]);
    }
}
