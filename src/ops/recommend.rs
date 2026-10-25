//! Song recommendations: seeds from a playlist, ranked from TIDAL's similar tracks.

use std::collections::{HashMap, HashSet};

use crate::api::{ItemKind, PlaylistItem, Track};
use crate::ops::{analysis, matcher::normalise};

/// TIDAL is asked about at most this many songs per search.
pub const MAX_SEEDS: usize = 20;
/// The result list is cut to this length.
pub const MAX_RESULTS: usize = 50;

#[derive(Clone, Debug)]
pub struct Recommendation {
    pub track: Track,
    /// How many of the seed songs led to this one.
    pub seeds: usize,
}

/// Seed track ids: the selected rows if any, else the whole playlist; only
/// playable tracks, in random order so a new search gives new suggestions.
pub fn pick_seeds(items: &[PlaylistItem], selected: &HashSet<String>) -> Vec<String> {
    let pool: Vec<PlaylistItem> = items
        .iter()
        .filter(|i| selected.is_empty() || selected.contains(&i.item_id))
        .filter(|i| i.r.kind == ItemKind::Track && i.available())
        .cloned()
        .collect();
    let mut seen = HashSet::new();
    analysis::shuffled(&pool).into_iter().map(|i| i.r.id).filter(|id| seen.insert(id.clone())).take(MAX_SEEDS).collect()
}

/// Same song in another form: same ISRC or same artist and title.
pub fn keys(t: &Track) -> Vec<String> {
    let mut k = vec![format!("id:{}", t.id)];
    if let Some(i) = t.isrc.as_deref().filter(|i| !i.is_empty()) {
        k.push(format!("isrc:{}", i.to_uppercase()));
    }
    if let Some(a) = t.artists.first() {
        k.push(format!("song:{}|{}", normalise(a), normalise(&t.title)));
    }
    k
}

/// Merges the similar-track lists (one per seed). Songs suggested by more
/// seeds come first, then those ranked higher by TIDAL. Songs matching
/// `have` (already in the playlist or already shown) and songs that cannot
/// be streamed are left out.
pub fn rank(lists: Vec<Vec<Track>>, have: &[Track]) -> Vec<Recommendation> {
    let known: HashSet<String> = have.iter().flat_map(keys).collect();
    // key → index into `found`, so one song under two ids counts once.
    let mut by_key: HashMap<String, usize> = HashMap::new();
    let mut found: Vec<(Recommendation, usize)> = Vec::new();
    for list in lists {
        let mut this_seed = HashSet::new();
        for (pos, t) in list.into_iter().enumerate() {
            let k = keys(&t);
            if !t.streamable() || k.iter().any(|k| known.contains(k)) {
                continue;
            }
            match k.iter().find_map(|k| by_key.get(k).copied()) {
                Some(n) => {
                    if this_seed.insert(n) {
                        found[n].0.seeds += 1;
                        found[n].1 = found[n].1.min(pos);
                    }
                }
                None => {
                    let n = found.len();
                    this_seed.insert(n);
                    for key in k {
                        by_key.insert(key, n);
                    }
                    found.push((Recommendation { track: t, seeds: 1 }, pos));
                }
            }
        }
    }
    // Stable sort keeps first-seen order among equals.
    found.sort_by_key(|(r, pos)| (std::cmp::Reverse(r.seeds), *pos));
    found.into_iter().map(|(r, _)| r).take(MAX_RESULTS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ItemRef;

    fn t(id: &str, artist: &str, title: &str, isrc: Option<&str>) -> Track {
        Track {
            id: id.into(),
            title: title.into(),
            artists: vec![artist.into()],
            isrc: isrc.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn ranks_by_seed_count_then_position() {
        let lists = vec![
            vec![t("1", "A", "One", None), t("2", "B", "Two", None), t("3", "C", "Three", None)],
            vec![t("3", "C", "Three", None), t("4", "D", "Four", None)],
            // Same song as 2 under another id (same ISRC): still counts as 2.
            vec![t("20", "B", "Two (Remaster)", Some("X")), t("2", "B", "Two", Some("X"))],
        ];
        let r = rank(lists, &[]);
        let ids: Vec<&str> = r.iter().map(|r| r.track.id.as_str()).collect();
        // 2 and 3 tie (two seeds, top of a list); 2 was seen first.
        assert_eq!(ids, ["2", "3", "1", "4"]);
        assert_eq!(r[0].seeds, 2);
    }

    #[test]
    fn leaves_out_owned_and_unplayable_songs() {
        let mut gone = t("5", "E", "Five", None);
        gone.availability = Some(vec!["DJ".into()]);
        let lists =
            vec![vec![t("1", "A", "One", None), t("9", "a", "ONE", None), t("2", "B", "Two", Some("I2")), gone]];
        let have = [t("1", "A", "One", None), t("7", "Z", "Other", Some("i2"))];
        assert!(rank(lists, &have).is_empty());
    }

    #[test]
    fn seeds_come_from_the_selection_and_are_unique() {
        let item = |n: usize, id: &str| PlaylistItem {
            item_id: format!("occ{n}"),
            r: ItemRef::track(id),
            added_at: None,
            track: Some(t(id, "A", id, None)),
            replacement: None,
        };
        let items: Vec<PlaylistItem> = (0..30).map(|n| item(n, &(n % 25).to_string())).collect();
        let all = pick_seeds(&items, &HashSet::new());
        assert_eq!(all.len(), MAX_SEEDS);
        assert_eq!(all.iter().collect::<HashSet<_>>().len(), MAX_SEEDS);
        let sel: HashSet<String> = ["occ3".into(), "occ4".into()].into();
        let mut some = pick_seeds(&items, &sel);
        some.sort();
        assert_eq!(some, ["3", "4"]);
    }
}
