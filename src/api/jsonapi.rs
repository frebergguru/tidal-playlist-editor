//! Minimal JSON:API document handling on top of `serde_json::Value`.
//!
//! The TIDAL schema is large and grows; reading through `Value` keeps us
//! tolerant of new fields and enum values (as the API docs ask).

use std::collections::HashMap;

use serde_json::Value;

use super::models::*;

#[derive(Debug, Default, Clone)]
pub struct Doc {
    pub data: Value,
    pub included: HashMap<(String, String), Value>,
    pub next: Option<String>,
}

impl Doc {
    pub fn from_value(mut v: Value) -> Self {
        let mut included = HashMap::new();
        if let Some(Value::Array(arr)) = v.get_mut("included").map(Value::take) {
            for r in arr {
                if let (Some(t), Some(id)) = (str_of(&r, "type"), str_of(&r, "id")) {
                    included.insert((t, id), r);
                }
            }
        }
        let next = v.pointer("/links/next").and_then(Value::as_str).map(str::to_owned);
        Doc { data: v.get_mut("data").map(Value::take).unwrap_or(Value::Null), included, next }
    }

    pub fn data_array(&self) -> &[Value] {
        self.data.as_array().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn get(&self, typ: &str, id: &str) -> Option<&Value> {
        self.included.get(&(typ.to_owned(), id.to_owned()))
    }

    /// Merges the `included` resources of another page into this one.
    pub fn absorb_included(&mut self, other: Doc) {
        self.included.extend(other.included);
    }
}

pub fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn attr<'a>(res: &'a Value, key: &str) -> Option<&'a Value> {
    res.get("attributes").and_then(|a| a.get(key))
}

pub fn attr_str(res: &Value, key: &str) -> Option<String> {
    attr(res, key).and_then(Value::as_str).map(str::to_owned)
}

pub fn attr_u32(res: &Value, key: &str) -> Option<u32> {
    attr(res, key).and_then(Value::as_u64).map(|n| n as u32)
}

/// `(type, id)` pairs of a relationship's linkage.
pub fn rel_ids(res: &Value, rel: &str) -> Vec<(String, String)> {
    let data = res.get("relationships").and_then(|r| r.get(rel)).and_then(|r| r.get("data"));
    let one = |v: &Value| Some((str_of(v, "type")?, str_of(v, "id")?));
    match data {
        Some(Value::Array(a)) => a.iter().filter_map(one).collect(),
        Some(v @ Value::Object(_)) => one(v).into_iter().collect(),
        _ => Vec::new(),
    }
}

pub fn rel_next(res: &Value, rel: &str) -> Option<String> {
    res.pointer(&format!("/relationships/{rel}/links/next")).and_then(Value::as_str).map(str::to_owned)
}

/// Builds a [`Track`] from a `tracks` or `videos` resource, resolving artists
/// and albums from `doc.included` when present.
pub fn track_from(res: &Value, doc: &Doc) -> Option<Track> {
    let kind = ItemKind::from_api(&str_of(res, "type")?)?;
    let id = str_of(res, "id")?;
    let mut artists = Vec::new();
    let mut artist_ids = Vec::new();
    for (t, aid) in rel_ids(res, "artists") {
        if let Some(name) = doc.get(&t, &aid).and_then(|a| attr_str(a, "name")) {
            artists.push(name);
        }
        artist_ids.push(aid);
    }
    let album_link = rel_ids(res, "albums").into_iter().next();
    let album = album_link.as_ref().and_then(|(t, aid)| doc.get(t, aid)).and_then(|a| attr_str(a, "title"));
    let availability = attr(res, "availability")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>());
    Some(Track {
        id,
        kind,
        title: attr_str(res, "title").unwrap_or_default(),
        version: attr_str(res, "version").filter(|v| !v.is_empty()),
        isrc: attr_str(res, "isrc").filter(|v| !v.is_empty()),
        duration_secs: attr_str(res, "duration").map(|d| parse_iso_duration(&d)).unwrap_or(0),
        explicit: attr(res, "explicit").and_then(Value::as_bool).unwrap_or(false),
        artists,
        artist_ids,
        album,
        album_id: album_link.map(|(_, id)| id),
        availability,
    })
}

pub fn playlist_from(res: &Value) -> Option<Playlist> {
    Some(Playlist {
        id: str_of(res, "id")?,
        name: attr_str(res, "name").unwrap_or_default(),
        description: attr_str(res, "description").unwrap_or_default(),
        access_type: AccessType::from_api(&attr_str(res, "accessType").unwrap_or_default()),
        number_of_items: attr_u32(res, "numberOfItems"),
        duration_secs: attr_str(res, "duration").map(|d| parse_iso_duration(&d)),
        last_modified: attr_str(res, "lastModifiedAt"),
        created: attr_str(res, "createdAt"),
        playlist_type: attr_str(res, "playlistType"),
        owned: true,
    })
}

pub fn album_from(res: &Value, doc: &Doc) -> Option<Album> {
    Some(Album {
        id: str_of(res, "id")?,
        title: attr_str(res, "title").unwrap_or_default(),
        artists: rel_ids(res, "artists")
            .iter()
            .filter_map(|(t, id)| doc.get(t, id).and_then(|a| attr_str(a, "name")))
            .collect(),
        release_date: attr_str(res, "releaseDate"),
        number_of_items: attr_u32(res, "numberOfItems"),
        album_type: attr_str(res, "albumType"),
    })
}

pub fn artist_from(res: &Value) -> Option<Artist> {
    Some(Artist { id: str_of(res, "id")?, name: attr_str(res, "name").unwrap_or_default() })
}

/// Playlist item linkage (`data[]` of the items relationship) to [`PlaylistItem`]s.
pub fn playlist_items_from(doc: &Doc) -> Vec<PlaylistItem> {
    doc.data_array()
        .iter()
        .filter_map(|link| {
            let typ = str_of(link, "type")?;
            let id = str_of(link, "id")?;
            let kind = ItemKind::from_api(&typ)?;
            let meta = link.get("meta");
            Some(PlaylistItem {
                item_id: meta.and_then(|m| str_of(m, "itemId")).unwrap_or_default(),
                added_at: meta.and_then(|m| str_of(m, "addedAt")),
                replacement: meta
                    .and_then(|m| m.pointer("/replacement/status"))
                    .and_then(Value::as_str)
                    .filter(|s| *s == "REPLACED")
                    .map(str::to_owned),
                track: doc.get(&typ, &id).and_then(|r| track_from(r, doc)),
                r: ItemRef { kind, id },
            })
        })
        .collect()
}

/// Extracts a readable message from a JSON:API error body.
pub fn error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            let e = v.get("errors")?.get(0)?;
            let detail = str_of(e, "detail").or_else(|| str_of(e, "title"))?;
            Some(match str_of(e, "code") {
                Some(code) => format!("{detail} ({code})"),
                None => detail,
            })
        })
        .unwrap_or_else(|| body.chars().take(300).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_items_with_includes() {
        let v = json!({
            "data": [
                {"id": "1", "type": "tracks", "meta": {"itemId": "a", "addedAt": "2024-01-01T00:00:00Z"}},
                {"id": "2", "type": "tracks", "meta": {"itemId": "b", "replacement": {"status": "REPLACED"}}},
                {"id": "9", "type": "videos", "meta": {"itemId": "c"}}
            ],
            "included": [
                {"id": "1", "type": "tracks", "attributes": {"title": "One", "isrc": "X1", "duration": "PT3M", "explicit": true,
                    "availability": ["STREAM"]},
                 "relationships": {"artists": {"data": [{"id": "10", "type": "artists"}]},
                                   "albums": {"data": [{"id": "20", "type": "albums"}]}}},
                {"id": "10", "type": "artists", "attributes": {"name": "Band"}},
                {"id": "20", "type": "albums", "attributes": {"title": "Record"}},
                {"id": "9", "type": "videos", "attributes": {"title": "Clip", "duration": "PT4M1S"}}
            ],
            "links": {"self": "/x", "next": "/x?page[cursor]=abc"}
        });
        let doc = Doc::from_value(v);
        assert_eq!(doc.next.as_deref(), Some("/x?page[cursor]=abc"));
        let items = playlist_items_from(&doc);
        assert_eq!(items.len(), 3);
        let t = items[0].track.as_ref().unwrap();
        assert_eq!((t.title.as_str(), t.artists[0].as_str(), t.album.as_deref()), ("One", "Band", Some("Record")));
        assert_eq!(t.duration_secs, 180);
        assert!(items[0].available());
        assert!(items[1].track.is_none() && !items[1].available());
        assert_eq!(items[1].replacement.as_deref(), Some("REPLACED"));
        assert_eq!(items[2].r.kind, ItemKind::Video);
        assert_eq!(items[2].track.as_ref().unwrap().duration_secs, 241);
    }

    #[test]
    fn error_messages() {
        let body = r#"{"errors":[{"status":"400","code":"INVALID_INCLUDE","detail":"bad include"}]}"#;
        assert_eq!(error_detail(body), "bad include (INVALID_INCLUDE)");
        assert_eq!(error_detail("plain"), "plain");
    }
}
