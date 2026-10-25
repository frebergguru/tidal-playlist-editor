//! Pure playlist analysis: duplicates, diffs, restore ordering and statistics.

use std::collections::{HashMap, HashSet};

use crate::api::{ItemRef, PlaylistItem, Track};
use crate::ops::matcher::normalise;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DupMode {
    /// Same TIDAL id.
    Exact,
    /// Same id or same ISRC (same recording on another release).
    Isrc,
    /// Also same normalised artist + title.
    Loose,
}

#[derive(Clone, Debug)]
pub struct DupGroup {
    /// Index of the kept (first) occurrence.
    pub keep: usize,
    /// Later occurrences that would be removed.
    pub extra: Vec<usize>,
}

/// The bracketed parts of a title and a " - suffix", e.g. "Song - Live" or
/// "Song (Acoustic) [Remix]" → "live" / "acoustic|remix".
fn bracketed(title: &str) -> String {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in title.chars() {
        match c {
            '(' | '[' => {
                depth += 1;
                if depth == 1 {
                    cur.clear();
                }
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    parts.push(std::mem::take(&mut cur));
                }
            }
            c if depth > 0 => cur.push(c),
            _ => {}
        }
    }
    if let Some((_, tail)) = title.split_once(" - ") {
        parts.push(tail.to_owned());
    }
    parts.join("|")
}

/// A version text reduced to what tells recordings apart: remaster and
/// "feat." notes are dropped, the rest is lower-case words.
fn version_key(v: &str) -> String {
    v.split('|')
        .map(|p| p.to_lowercase())
        .filter(|p| {
            let p = p.trim_start();
            !p.contains("remaster") && !["feat", "ft.", "ft ", "featuring", "with "].iter().any(|m| p.starts_with(m))
        })
        .map(|p| p.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("|")
}

pub fn find_duplicates(
    items: &[PlaylistItem],
    mode: DupMode,
    cache: &dyn Fn(&ItemRef) -> Option<Track>,
) -> Vec<DupGroup> {
    let mut first_by_key: HashMap<String, usize> = HashMap::new();
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, it) in items.iter().enumerate() {
        let meta = it.track.clone().or_else(|| cache(&it.r));
        let mut keys = vec![format!("id:{:?}:{}", it.r.kind, it.r.id)];
        if mode != DupMode::Exact
            && let Some(isrc) = meta.as_ref().and_then(|t| t.isrc.clone())
        {
            keys.push(format!("isrc:{}", isrc.to_uppercase()));
        }
        if mode == DupMode::Loose
            && let Some(t) = &meta
        {
            let title = normalise(&t.title);
            if !title.is_empty() {
                // Keep live, acoustic and remix versions apart: the version and
                // any bracketed part of the title (other than remaster/feat.
                // noise, which `normalise` drops) belong to the key.
                let version = t.version.as_deref().map(version_key).unwrap_or_default();
                let tag = version_key(&bracketed(&t.title));
                keys.push(format!("ta:{}|{}|{}|{}", normalise(&t.artist_line()), title, version, tag));
            }
        }
        match keys.iter().find_map(|k| first_by_key.get(k).copied()) {
            Some(keep) => {
                groups.entry(keep).or_default().push(i);
                for k in keys {
                    first_by_key.entry(k).or_insert(keep);
                }
            }
            None => {
                for k in keys {
                    first_by_key.entry(k).or_insert(i);
                }
            }
        }
    }
    let mut out: Vec<DupGroup> = groups.into_iter().map(|(keep, extra)| DupGroup { keep, extra }).collect();
    out.sort_by_key(|g| g.keep);
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    /// In the target (backup) but not in the playlist now.
    pub missing: Vec<ItemRef>,
    /// In the playlist now but not in the target.
    pub extra: Vec<ItemRef>,
    pub order_differs: bool,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty() && !self.order_differs
    }
}

/// Multiset difference, preserving order of first appearance.
fn minus(a: &[ItemRef], b: &[ItemRef]) -> Vec<ItemRef> {
    let mut left: HashMap<&ItemRef, usize> = HashMap::new();
    for r in b {
        *left.entry(r).or_default() += 1;
    }
    a.iter()
        .filter(|r| match left.get_mut(r) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false
            }
            _ => true,
        })
        .cloned()
        .collect()
}

pub fn diff(current: &[ItemRef], target: &[ItemRef]) -> Diff {
    let missing = minus(target, current);
    let extra = minus(current, target);
    let order_differs = missing.is_empty() && extra.is_empty() && current != target;
    Diff { missing, extra, order_differs }
}

/// Occurrences to delete so that `current` holds at most what `target` needs.
/// Later duplicates are removed first.
pub fn removals(current: &[PlaylistItem], target: &[ItemRef]) -> Vec<PlaylistItem> {
    let mut need: HashMap<&ItemRef, usize> = HashMap::new();
    for r in target {
        *need.entry(r).or_default() += 1;
    }
    current
        .iter()
        .filter(|it| match need.get_mut(&it.r) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false
            }
            _ => true,
        })
        .cloned()
        .collect()
}

/// Maps the target order onto actual occurrences of `current`. Occurrences not
/// claimed by the target are appended at the end, in their present order.
pub fn order_for(current: &[PlaylistItem], target: &[ItemRef]) -> Vec<PlaylistItem> {
    let mut pools: HashMap<&ItemRef, std::collections::VecDeque<&PlaylistItem>> = HashMap::new();
    for it in current {
        pools.entry(&it.r).or_default().push_back(it);
    }
    let mut used = HashSet::new();
    let mut out = Vec::with_capacity(current.len());
    for r in target {
        if let Some(it) = pools.get_mut(r).and_then(|p| p.pop_front()) {
            used.insert(it.item_id.clone());
            out.push(it.clone());
        }
    }
    out.extend(current.iter().filter(|it| !used.contains(&it.item_id)).cloned());
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    Duration,
    Added,
}

pub fn sorted(items: &[PlaylistItem], key: SortKey, ascending: bool) -> Vec<PlaylistItem> {
    let mut v = items.to_vec();
    let text =
        |it: &PlaylistItem, f: &dyn Fn(&Track) -> String| it.track.as_ref().map(f).unwrap_or_default().to_lowercase();
    v.sort_by(|a, b| {
        let o = match key {
            SortKey::Title => text(a, &|t| t.full_title()).cmp(&text(b, &|t| t.full_title())),
            SortKey::Artist => text(a, &|t| t.artist_line()).cmp(&text(b, &|t| t.artist_line())).then_with(|| {
                text(a, &|t| t.album.clone().unwrap_or_default())
                    .cmp(&text(b, &|t| t.album.clone().unwrap_or_default()))
            }),
            SortKey::Album => text(a, &|t| t.album.clone().unwrap_or_default())
                .cmp(&text(b, &|t| t.album.clone().unwrap_or_default())),
            SortKey::Duration => {
                a.track.as_ref().map_or(0, |t| t.duration_secs).cmp(&b.track.as_ref().map_or(0, |t| t.duration_secs))
            }
            SortKey::Added => a.added_at.cmp(&b.added_at),
        };
        if ascending { o } else { o.reverse() }
    });
    v
}

/// Deterministic-per-call shuffle without pulling in a RNG crate.
pub fn shuffled(items: &[PlaylistItem]) -> Vec<PlaylistItem> {
    let mut v = items.to_vec();
    let mut seed = u128::from_le_bytes(*uuid::Uuid::new_v4().as_bytes());
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let j = (seed % (i as u128 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub items: usize,
    pub tracks: usize,
    pub videos: usize,
    pub unavailable: usize,
    pub explicit: usize,
    pub total_secs: u64,
    pub artists: usize,
    pub albums: usize,
    pub top_artists: Vec<(String, usize)>,
    pub duplicates: usize,
}

pub fn stats(items: &[PlaylistItem]) -> Stats {
    let mut s = Stats { items: items.len(), ..Default::default() };
    let mut artists: HashMap<String, usize> = HashMap::new();
    let mut albums = HashSet::new();
    for it in items {
        match it.r.kind {
            crate::api::ItemKind::Track => s.tracks += 1,
            crate::api::ItemKind::Video => s.videos += 1,
        }
        if !it.available() {
            s.unavailable += 1;
        }
        if let Some(t) = &it.track {
            s.total_secs += u64::from(t.duration_secs);
            s.explicit += usize::from(t.explicit);
            for a in &t.artists {
                *artists.entry(a.clone()).or_default() += 1;
            }
            if let Some(al) = &t.album_id {
                albums.insert(al.clone());
            }
        }
    }
    s.artists = artists.len();
    s.albums = albums.len();
    let mut top: Vec<(String, usize)> = artists.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(10);
    s.top_artists = top;
    s.duplicates = find_duplicates(items, DupMode::Exact, &|_| None).iter().map(|g| g.extra.len()).sum();
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ItemKind, Track};

    fn item(n: usize, id: &str, isrc: Option<&str>, title: &str) -> PlaylistItem {
        PlaylistItem {
            item_id: format!("occ{n}"),
            r: ItemRef { kind: ItemKind::Track, id: id.into() },
            added_at: Some(format!("2024-01-0{n}")),
            track: Some(Track {
                id: id.into(),
                title: title.into(),
                isrc: isrc.map(Into::into),
                artists: vec!["A".into()],
                ..Default::default()
            }),
            replacement: None,
        }
    }

    #[test]
    fn duplicates_by_mode() {
        let items = vec![
            item(1, "1", Some("I1"), "Song"),
            item(2, "1", Some("I1"), "Song"),
            item(3, "2", Some("I1"), "Song (Remaster)"),
            item(4, "3", Some("I3"), "Song"),
            item(5, "4", None, "Other"),
        ];
        let none = |_: &ItemRef| None;
        let exact = find_duplicates(&items, DupMode::Exact, &none);
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].extra, vec![1]);
        let isrc = find_duplicates(&items, DupMode::Isrc, &none);
        assert_eq!(isrc[0].extra, vec![1, 2]);
        let loose = find_duplicates(&items, DupMode::Loose, &none);
        assert_eq!(loose[0].extra, vec![1, 2, 3]);
    }

    #[test]
    fn loose_keeps_different_versions_apart() {
        let items = vec![
            item(1, "1", None, "Song"),
            item(2, "2", None, "Song (Acoustic)"),
            item(3, "3", None, "Song - Live"),
            item(4, "4", None, "Song (2011 Remaster)"),
            item(5, "5", None, "Song (feat. Someone)"),
            item(6, "6", None, "Song (Acoustic)"),
        ];
        let loose = find_duplicates(&items, DupMode::Loose, &|_: &ItemRef| None);
        let groups: Vec<(usize, Vec<usize>)> = loose.into_iter().map(|g| (g.keep, g.extra)).collect();
        assert_eq!(groups, vec![(0, vec![3, 4]), (1, vec![5])]);
    }

    #[test]
    fn diff_and_restore_order() {
        let cur =
            vec![item(1, "a", None, "a"), item(2, "b", None, "b"), item(3, "b", None, "b"), item(4, "c", None, "c")];
        let target: Vec<ItemRef> = ["c", "b", "d", "a"].iter().map(|s| ItemRef::track(*s)).collect();
        let refs: Vec<ItemRef> = cur.iter().map(|i| i.r.clone()).collect();
        let d = diff(&refs, &target);
        assert_eq!(d.missing, vec![ItemRef::track("d")]);
        assert_eq!(d.extra, vec![ItemRef::track("b")]);
        let rm = removals(&cur, &target);
        assert_eq!(rm.iter().map(|i| i.item_id.as_str()).collect::<Vec<_>>(), vec!["occ3"]);
        // After removal + adding "d" at the end:
        let mut after: Vec<PlaylistItem> = cur.iter().filter(|i| i.item_id != "occ3").cloned().collect();
        after.push(item(5, "d", None, "d"));
        let order = order_for(&after, &target);
        assert_eq!(order.iter().map(|i| i.r.id.as_str()).collect::<Vec<_>>(), vec!["c", "b", "d", "a"]);
        let d2 = diff(&order.iter().map(|i| i.r.clone()).collect::<Vec<_>>(), &target);
        assert!(d2.is_empty());
        let only_order = diff(&[ItemRef::track("x"), ItemRef::track("y")], &[ItemRef::track("y"), ItemRef::track("x")]);
        assert!(only_order.order_differs);
    }

    #[test]
    fn sorting_and_shuffle_keep_items() {
        let items = vec![item(1, "1", None, "b"), item(2, "2", None, "a"), item(3, "3", None, "c")];
        let s = sorted(&items, SortKey::Title, true);
        assert_eq!(s.iter().map(|i| i.r.id.as_str()).collect::<Vec<_>>(), vec!["2", "1", "3"]);
        let s = sorted(&items, SortKey::Added, false);
        assert_eq!(s[0].r.id, "3");
        let mut sh: Vec<String> = shuffled(&items).into_iter().map(|i| i.r.id).collect();
        sh.sort();
        assert_eq!(sh, vec!["1", "2", "3"]);
        let st = stats(&items);
        assert_eq!((st.items, st.artists, st.duplicates), (3, 1, 0));
    }
}
