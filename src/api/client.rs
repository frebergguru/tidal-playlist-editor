//! Async TIDAL API v2 client: auth refresh, retries, pagination and batching.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::auth::{self, AuthConfig, TokenSet};
use super::jsonapi::{self, Doc};
use super::models::*;
use super::{ApiError, Result};

pub const API_BASE: &str = "https://openapi.tidal.com/v2";
const USER_AGENT: &str = concat!("tidal-playlist-editor/", env!("CARGO_PKG_VERSION"));
const JSONAPI: &str = "application/vnd.api+json";

/// Spec limits (maxItems) per request body / filter.
pub const ADD_BATCH: usize = 50;
pub const REMOVE_BATCH: usize = 50;
pub const MOVE_BATCH: usize = 20;
pub const FILTER_BATCH: usize = 20;
const MAX_ATTEMPTS: u32 = 6;

pub type Query = Vec<(String, String)>;

pub fn q(pairs: &[(&str, &str)]) -> Query {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    auth: AuthConfig,
    tokens: Mutex<TokenSet>,
    token_path: Option<PathBuf>,
    country: std::sync::RwLock<String>,
}

impl Client {
    pub fn http() -> reqwest::Client {
        // ring keeps the Windows cross-build free of aws-lc's C/CMake toolchain.
        static PROVIDER: std::sync::Once = std::sync::Once::new();
        PROVIDER.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .build()
            .expect("http client")
    }

    pub fn new(auth: AuthConfig, tokens: TokenSet, token_path: Option<PathBuf>) -> Self {
        Self::with_base(API_BASE, auth, tokens, token_path)
    }

    pub fn with_base(base: &str, auth: AuthConfig, tokens: TokenSet, token_path: Option<PathBuf>) -> Self {
        Self {
            http: Self::http(),
            base: base.trim_end_matches('/').to_owned(),
            auth,
            tokens: Mutex::new(tokens),
            token_path,
            country: std::sync::RwLock::new("US".into()),
        }
    }

    pub fn country(&self) -> String {
        self.country.read().expect("lock").clone()
    }

    pub fn set_country(&self, c: &str) {
        if !c.is_empty() {
            *self.country.write().expect("lock") = c.to_owned();
        }
    }

    /// Absolute URLs (from `links.next`) must point at our own API origin, so
    /// the bearer token never goes to another host or over plain HTTP.
    fn url_for(&self, path: &str) -> Result<String> {
        Ok(if path.starts_with("http://") || path.starts_with("https://") {
            if !same_origin(path, &self.base) {
                return Err(ApiError::Other(format!("refusing link to another host: {}", strip_query(path))));
            }
            path.to_owned()
        } else if let Some(rest) = path.strip_prefix("/v2/") {
            format!("{}/{rest}", self.base)
        } else {
            format!("{}/{}", self.base, path.trim_start_matches('/'))
        })
    }

    /// The current token, refreshed when it is about to expire. `rejected` is a
    /// token that just got a 401: it is refreshed unless another request has
    /// already replaced it.
    async fn access_token(&self, rejected: Option<&str>) -> Result<String> {
        let mut t = self.tokens.lock().await;
        let force_refresh = rejected.is_some_and(|r| r == t.access_token);
        if force_refresh || t.expires_soon() {
            let Some(rt) = t.refresh_token.clone() else {
                return if force_refresh { Err(ApiError::NotLoggedIn) } else { Ok(t.access_token.clone()) };
            };
            let fresh = auth::refresh(&self.http, &self.auth, &rt).await?;
            if let Some(p) = &self.token_path
                && let Err(e) = auth::save_tokens(p, &fresh)
            {
                tracing::warn!("could not save tokens: {e}");
            }
            *t = fresh;
        }
        Ok(t.access_token.clone())
    }

    /// Sends one request with retries on 429/5xx and a single re-auth on 401.
    /// Mutations carry an `Idempotency-Key` that stays the same across retries.
    pub async fn send(&self, method: Method, path: &str, query: &Query, body: Option<&Value>) -> Result<Value> {
        let url = self.url_for(path)?;
        let idem = (method != Method::GET).then(|| uuid::Uuid::new_v4().to_string());
        let mut refreshed = false;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.access_token(None).await?;
            let mut req =
                self.http.request(method.clone(), &url).bearer_auth(&token).header("Accept", JSONAPI).query(query);
            if let Some(k) = &idem {
                req = req.header("Idempotency-Key", k);
            }
            if let Some(b) = body {
                req = req.header("Content-Type", JSONAPI).body(serde_json::to_vec(b).expect("json"));
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if attempt < MAX_ATTEMPTS && (e.is_timeout() || e.is_connect()) => {
                    tokio::time::sleep(backoff(attempt)).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let status = resp.status();
            if status == StatusCode::UNAUTHORIZED && !refreshed {
                refreshed = true;
                self.access_token(Some(&token)).await?;
                continue;
            }
            if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) && attempt < MAX_ATTEMPTS {
                let wait = retry_after(resp.headers(), 120).unwrap_or_else(|| backoff(attempt));
                tracing::debug!("{status} on {url}, retrying in {wait:?}");
                tokio::time::sleep(wait).await;
                continue;
            }
            let text = resp.text().await?;
            if !status.is_success() {
                return Err(ApiError::Http {
                    status: status.as_u16(),
                    detail: jsonapi::error_detail(&text),
                    code: jsonapi::error_code(&text),
                });
            }
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&text).map_err(|e| ApiError::Other(format!("bad JSON from {url}: {e}")));
        }
    }

    pub async fn get(&self, path: &str, query: &Query) -> Result<Doc> {
        Ok(Doc::from_value(self.send(Method::GET, path, query, None).await?))
    }

    /// Follows `links.next` until exhausted; returns all primary data in one doc.
    pub async fn get_all(&self, path: &str, query: &Query, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Doc> {
        let mut doc = self.get(path, query).await?;
        let mut all = std::mem::take(&mut doc.data).as_array().cloned().unwrap_or_default();
        let mut next = doc.next.take();
        let mut seen = HashSet::new();
        while let Some(n) = next {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            // A repeated cursor would loop forever; partial data is not safe
            // either (a sync would think the rest is missing), so fail.
            if !seen.insert(n.clone()) {
                return Err(ApiError::Other(format!("paging repeated itself on {path}")));
            }
            let mut page = self.get(&n, &missing_params(&n, query)).await?;
            all.extend(std::mem::take(&mut page.data).as_array().cloned().unwrap_or_default());
            next = page.next.take();
            doc.absorb_included(page);
        }
        doc.data = Value::Array(all);
        Ok(doc)
    }

    /// Tries each `include` variant in turn, falling back on HTTP 400 (unsupported path).
    async fn get_all_with_includes(
        &self,
        path: &str,
        base: &Query,
        includes: &[&str],
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<Doc> {
        let mut last = None;
        for inc in includes {
            let mut query = base.clone();
            if !inc.is_empty() {
                query.push(("include".into(), (*inc).into()));
            }
            match self.get_all(path, &query, cancel).await {
                Err(e) if e.status() == Some(400) => {
                    tracing::debug!("include {inc:?} rejected on {path}: {e}");
                    last = Some(e);
                }
                other => return other,
            }
        }
        Err(last.unwrap_or_else(|| ApiError::Other("no include variant".into())))
    }

    fn cc(&self) -> (String, String) {
        ("countryCode".into(), self.country())
    }

    // ---------------------------------------------------------------- user

    pub async fn me(&self) -> Result<User> {
        let doc = self.get("/users/me", &Query::new()).await?;
        let d = &doc.data;
        let user = User {
            id: jsonapi::str_of(d, "id").unwrap_or_default(),
            username: jsonapi::attr_str(d, "username").unwrap_or_default(),
            country: jsonapi::attr_str(d, "country").unwrap_or_default(),
        };
        self.set_country(&user.country);
        Ok(user)
    }

    // ----------------------------------------------------------- playlists

    pub async fn my_playlists(&self) -> Result<Vec<Playlist>> {
        let never = || false;
        let mut out = Vec::new();
        let mut ids = HashSet::new();
        for (filter, owned) in [("filter[owners.id]", true), ("filter[collaborators.id]", false)] {
            let query = vec![(filter.into(), "me".into()), self.cc(), ("sort".into(), "-lastModifiedAt".into())];
            let doc = match self.get_all("/playlists", &query, &never).await {
                Ok(d) => d,
                // Collaborator listing is optional; do not fail the whole load on it.
                Err(e) if !owned => {
                    tracing::info!("collaborator playlists unavailable: {e}");
                    continue;
                }
                Err(e) => return Err(e),
            };
            for r in doc.data_array() {
                if let Some(mut p) = jsonapi::playlist_from(r)
                    && ids.insert(p.id.clone())
                {
                    p.owned = owned;
                    out.push(p);
                }
            }
        }
        Ok(out)
    }

    pub async fn playlist(&self, id: &str) -> Result<Playlist> {
        let doc = self.get(&format!("/playlists/{id}"), &vec![self.cc()]).await?;
        jsonapi::playlist_from(&doc.data).ok_or_else(|| ApiError::Other("playlist missing".into()))
    }

    pub async fn playlist_items(&self, id: &str, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Vec<PlaylistItem>> {
        let path = format!("/playlists/{id}/relationships/items");
        let doc = self
            .get_all_with_includes(
                &path,
                &vec![self.cc()],
                &[
                    "items,items.artists,items.tracks:albums",
                    "items,items.artists,items.albums",
                    "items,items.artists",
                    "items",
                ],
                cancel,
            )
            .await?;
        Ok(jsonapi::playlist_items_from(&doc))
    }

    pub async fn create_playlist(&self, name: &str, description: &str, access: AccessType) -> Result<Playlist> {
        let access = if access == AccessType::Private { AccessType::Unlisted } else { access };
        let body = json!({"data": {"type": "playlists", "attributes": {
            "name": truncate(name, 250), "description": truncate(description, 500), "accessType": access.api()}}});
        let v = self.send(Method::POST, "/playlists", &vec![self.cc()], Some(&body)).await?;
        jsonapi::playlist_from(v.get("data").unwrap_or(&Value::Null))
            .ok_or_else(|| ApiError::Other("create playlist: no data in response".into()))
    }

    pub async fn update_playlist(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        access: Option<AccessType>,
    ) -> Result<()> {
        let mut attrs = serde_json::Map::new();
        if let Some(n) = name {
            attrs.insert("name".into(), truncate(n, 250).into());
        }
        if let Some(d) = description {
            attrs.insert("description".into(), truncate(d, 500).into());
        }
        if let Some(a) = access.filter(|a| *a != AccessType::Private) {
            attrs.insert("accessType".into(), a.api().into());
        }
        let body = json!({"data": {"type": "playlists", "id": id, "attributes": attrs}});
        self.send(Method::PATCH, &format!("/playlists/{id}"), &Query::new(), Some(&body)).await?;
        Ok(())
    }

    pub async fn delete_playlist(&self, id: &str) -> Result<()> {
        self.send(Method::DELETE, &format!("/playlists/{id}"), &Query::new(), None).await?;
        Ok(())
    }

    /// Adds items in order. With `before`, every batch is inserted before that
    /// occurrence, which keeps the overall order. Returns the number of skipped duplicates.
    pub async fn add_items(
        &self,
        pid: &str,
        refs: &[ItemRef],
        before: Option<&str>,
        skip_duplicates: bool,
        progress: &(dyn Fn(usize) + Sync),
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<usize> {
        let path = format!("/playlists/{pid}/relationships/items");
        let (mut skipped, mut done) = (0, 0);
        for chunk in refs.chunks(ADD_BATCH) {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let data: Vec<Value> = chunk.iter().map(|r| json!({"type": r.kind.api_type(), "id": r.id})).collect();
            let mut meta = serde_json::Map::new();
            if skip_duplicates {
                meta.insert("onDuplicates".into(), "SKIP".into());
            }
            if let Some(b) = before {
                meta.insert("positionBefore".into(), b.into());
            }
            let mut body = json!({"data": data});
            if !meta.is_empty() {
                body["meta"] = Value::Object(meta);
            }
            let v = self.send(Method::POST, &path, &Query::new(), Some(&body)).await?;
            skipped += v.pointer("/meta/skipped").and_then(Value::as_array).map_or(0, Vec::len);
            done += chunk.len();
            progress(done);
        }
        Ok(skipped)
    }

    pub async fn remove_items(
        &self,
        pid: &str,
        items: &[(ItemRef, String)],
        progress: &(dyn Fn(usize) + Sync),
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<()> {
        let path = format!("/playlists/{pid}/relationships/items");
        let mut done = 0;
        for chunk in items.chunks(REMOVE_BATCH) {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let data: Vec<Value> = chunk
                .iter()
                .map(|(r, item_id)| json!({"type": r.kind.api_type(), "id": r.id, "meta": {"itemId": item_id}}))
                .collect();
            self.send(Method::DELETE, &path, &Query::new(), Some(&json!({"data": data}))).await?;
            done += chunk.len();
            progress(done);
        }
        Ok(())
    }

    /// Moves the given occurrences (in the given order) before `before`, or to the end.
    pub async fn move_items(
        &self,
        pid: &str,
        items: &[(ItemRef, String)],
        before: Option<&str>,
        progress: &(dyn Fn(usize) + Sync),
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<()> {
        let path = format!("/playlists/{pid}/relationships/items");
        let mut done = 0;
        for chunk in items.chunks(MOVE_BATCH) {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let data: Vec<Value> = chunk
                .iter()
                .map(|(r, item_id)| json!({"type": r.kind.api_type(), "id": r.id, "meta": {"itemId": item_id}}))
                .collect();
            let mut body = json!({"data": data});
            if let Some(b) = before {
                body["meta"] = json!({"positionBefore": b});
            }
            self.send(Method::PATCH, &path, &Query::new(), Some(&body)).await?;
            done += chunk.len();
            progress(done);
        }
        Ok(())
    }

    /// Reorders a playlist to `order` (all current occurrences, new order).
    /// Keeps the longest already-correct prefix and moves the rest to the end in order.
    pub async fn apply_order(
        &self,
        pid: &str,
        current: &[PlaylistItem],
        order: &[PlaylistItem],
        progress: &(dyn Fn(usize) + Sync),
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<usize> {
        let keep = current.iter().zip(order).take_while(|(a, b)| a.item_id == b.item_id).count();
        let rest: Vec<(ItemRef, String)> = order[keep..].iter().map(|i| (i.r.clone(), i.item_id.clone())).collect();
        self.move_items(pid, &rest, None, progress, cancel).await?;
        Ok(rest.len())
    }

    // -------------------------------------------------------------- catalog

    /// Looks up tracks by id (batched by 20), with artists and albums.
    pub async fn tracks_by_ids(&self, ids: &[String], cancel: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Track>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(FILTER_BATCH) {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let mut query: Query = chunk.iter().map(|id| ("filter[id]".to_owned(), id.clone())).collect();
            query.push(self.cc());
            let doc = self.get_all_with_includes("/tracks", &query, &["artists,albums", "artists"], cancel).await?;
            out.extend(doc.data_array().iter().filter_map(|r| jsonapi::track_from(r, &doc)));
        }
        Ok(out)
    }

    pub async fn tracks_by_isrc(&self, isrc: &str) -> Result<Vec<Track>> {
        let query = vec![("filter[isrc]".to_owned(), isrc.to_owned()), self.cc()];
        let doc = self.get_all_with_includes("/tracks", &query, &["artists,albums", "artists"], &|| false).await?;
        Ok(doc.data_array().iter().filter_map(|r| jsonapi::track_from(r, &doc)).collect())
    }

    /// Track ids (from `ids`) that have no streaming usage rule in the user's country.
    pub async fn not_streamable(&self, ids: &[String], cancel: &(dyn Fn() -> bool + Sync)) -> Result<HashSet<String>> {
        let mut bad = HashSet::new();
        for chunk in ids.chunks(FILTER_BATCH) {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let mut query: Query = chunk.iter().map(|id| ("filter[id]".to_owned(), id.clone())).collect();
            query.push(self.cc());
            query.push(("include".into(), "usageRules".into()));
            let doc = self.get_all("/tracks", &query, cancel).await?;
            let returned: HashSet<String> = doc.data_array().iter().filter_map(|r| jsonapi::str_of(r, "id")).collect();
            for r in doc.data_array() {
                let Some(id) = jsonapi::str_of(r, "id") else { continue };
                let rules = jsonapi::rel_ids(r, "usageRules");
                let Some((t, rid)) = rules.first() else { continue };
                let Some(rule) = doc.get(t, rid) else { continue };
                let any = ["subscription", "free", "paid"]
                    .iter()
                    .any(|k| jsonapi::attr(rule, k).and_then(Value::as_array).is_some_and(|a| !a.is_empty()));
                if !any {
                    bad.insert(id);
                }
            }
            bad.extend(chunk.iter().filter(|id| !returned.contains(*id)).cloned());
        }
        Ok(bad)
    }

    pub async fn search(&self, query_text: &str) -> Result<SearchResults> {
        let base = vec![
            ("filter[query]".to_owned(), query_text.to_owned()),
            self.cc(),
            ("deviceType".into(), "DESKTOP".into()),
        ];
        let incs = [
            "tracks,tracks.artists,tracks.albums,albums,albums.artists,artists",
            "tracks,tracks.artists,albums,artists",
            "tracks,albums,artists",
        ];
        let mut doc = None;
        let mut last = None;
        for inc in incs {
            let mut query = base.clone();
            query.push(("include".into(), inc.into()));
            match self.get("/searchResults", &query).await {
                Err(e) if e.status() == Some(400) => last = Some(e),
                other => {
                    doc = Some(other?);
                    break;
                }
            }
        }
        let doc = doc.ok_or_else(|| last.unwrap_or_else(|| ApiError::Other("search failed".into())))?;
        // The collection holds exactly one searchResults resource.
        let res = doc.data_array().first().cloned().unwrap_or_else(|| doc.data.clone());
        let pick = |rel: &str| -> Vec<Value> {
            jsonapi::rel_ids(&res, rel).iter().filter_map(|(t, id)| doc.get(t, id).cloned()).collect()
        };
        Ok(SearchResults {
            tracks: pick("tracks").iter().filter_map(|r| jsonapi::track_from(r, &doc)).collect(),
            tracks_next: jsonapi::rel_next(&res, "tracks"),
            albums: pick("albums").iter().filter_map(|r| jsonapi::album_from(r, &doc)).collect(),
            artists: pick("artists").iter().filter_map(jsonapi::artist_from).collect(),
            did_you_mean: jsonapi::attr_str(&res, "didYouMean"),
        })
    }

    /// Next page of a search's tracks relationship.
    pub async fn search_more_tracks(&self, next: &str) -> Result<(Vec<Track>, Option<String>)> {
        let extra =
            missing_params(next, &vec![self.cc(), ("include".into(), "tracks,tracks.artists,tracks.albums".into())]);
        let doc = self.get(next, &extra).await?;
        let tracks = doc
            .data_array()
            .iter()
            .filter_map(|l| {
                let (t, id) = (jsonapi::str_of(l, "type")?, jsonapi::str_of(l, "id")?);
                jsonapi::track_from(doc.get(&t, &id)?, &doc)
            })
            .collect();
        Ok((tracks, doc.next.clone()))
    }

    pub async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        let doc = self
            .get_all_with_includes(
                &format!("/albums/{album_id}/relationships/items"),
                &vec![self.cc()],
                &["items,items.artists,items.albums", "items,items.artists", "items"],
                &|| false,
            )
            .await?;
        Ok(linked_tracks(&doc))
    }

    pub async fn artist_albums(&self, artist_id: &str) -> Result<Vec<Album>> {
        let doc = self
            .get_all_with_includes(
                &format!("/artists/{artist_id}/relationships/albums"),
                &vec![self.cc()],
                &["albums,albums.artists", "albums"],
                &|| false,
            )
            .await?;
        Ok(doc
            .data_array()
            .iter()
            .filter_map(|l| {
                let (t, id) = (jsonapi::str_of(l, "type")?, jsonapi::str_of(l, "id")?);
                jsonapi::album_from(doc.get(&t, &id)?, &doc)
            })
            .collect())
    }

    pub async fn artist_tracks(&self, artist_id: &str) -> Result<Vec<Track>> {
        let path = format!("/artists/{artist_id}/relationships/tracks");
        let query = vec![self.cc(), ("collapseBy".into(), "FINGERPRINT".into())];
        // One page is enough here (top tracks); fall back through include variants.
        for inc in ["tracks,tracks.artists,tracks.albums", "tracks,tracks.artists", "tracks"] {
            let mut qq = query.clone();
            qq.push(("include".into(), inc.into()));
            match self.get(&path, &qq).await {
                Err(e) if e.status() == Some(400) => continue,
                other => return Ok(linked_tracks(&other?)),
            }
        }
        Ok(Vec::new())
    }

    /// Tracks TIDAL considers similar to this one (first page, best match first).
    pub async fn similar_tracks(&self, track_id: &str) -> Result<Vec<Track>> {
        let path = format!("/tracks/{track_id}/relationships/similarTracks");
        for inc in ["similarTracks,similarTracks.artists,similarTracks.albums", "similarTracks"] {
            let query = vec![self.cc(), ("include".into(), inc.into())];
            match self.get(&path, &query).await {
                Err(e) if e.status() == Some(400) => continue,
                // A seed TIDAL no longer has simply gives no suggestions.
                Err(e) if e.status() == Some(404) => return Ok(Vec::new()),
                other => return Ok(linked_tracks(&other?)),
            }
        }
        Ok(Vec::new())
    }

    // --------------------------------------------------------------- genres

    /// The genres TIDAL lets users pick from, named in `locale` (e.g. nb-NO).
    pub async fn genres(&self, locale: &str) -> Result<Vec<Genre>> {
        let query = q(&[("filter[id]", "USER_SELECTABLE"), ("locale", locale)]);
        let doc = self.get_all("/genres", &query, &|| false).await?;
        Ok(doc
            .data_array()
            .iter()
            .filter_map(|r| Some(Genre { id: jsonapi::str_of(r, "id")?, name: jsonapi::attr_str(r, "genreName")? }))
            .collect())
    }

    /// Playlists (mostly TIDAL's own) found by a search, best match first.
    pub async fn search_playlists(&self, query_text: &str) -> Result<Vec<Playlist>> {
        let query = vec![
            ("filter[query]".to_owned(), query_text.to_owned()),
            self.cc(),
            ("deviceType".into(), "DESKTOP".into()),
            ("include".into(), "playlists".into()),
        ];
        let doc = self.get("/searchResults", &query).await?;
        let res = doc.data_array().first().cloned().unwrap_or_else(|| doc.data.clone());
        Ok(jsonapi::rel_ids(&res, "playlists")
            .iter()
            .filter_map(|(t, id)| jsonapi::playlist_from(doc.get(t, id)?))
            .collect())
    }

    /// Genre ids per track. Tracks TIDAL gives no genres for are left out.
    pub async fn track_genres(
        &self,
        ids: &[String],
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<HashMap<String, Vec<String>>> {
        let mut out = HashMap::new();
        for chunk in ids.chunks(FILTER_BATCH) {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let mut query: Query = chunk.iter().map(|id| ("filter[id]".to_owned(), id.clone())).collect();
            query.push(self.cc());
            query.push(("includeLinkage".into(), "genres".into()));
            let doc = match self.get_all("/tracks", &query, cancel).await {
                // Older servers: ask for the full genre resources instead.
                Err(e) if e.status() == Some(400) => {
                    query.pop();
                    query.push(("include".into(), "genres".into()));
                    self.get_all("/tracks", &query, cancel).await?
                }
                other => other?,
            };
            for r in doc.data_array() {
                let Some(id) = jsonapi::str_of(r, "id") else { continue };
                let genres: Vec<String> = jsonapi::rel_ids(r, "genres").into_iter().map(|(_, g)| g).collect();
                if !genres.is_empty() {
                    out.insert(id, genres);
                }
            }
        }
        Ok(out)
    }

    // ----------------------------------------------------------- favourites

    pub async fn set_favourite_tracks(&self, ids: &[String], add: bool) -> Result<()> {
        let path = "/userCollectionTracks/me/relationships/items";
        for chunk in ids.chunks(ADD_BATCH) {
            let data: Vec<Value> = chunk.iter().map(|id| json!({"type": "tracks", "id": id})).collect();
            let method = if add { Method::POST } else { Method::DELETE };
            self.send(method, path, &vec![self.cc()], Some(&json!({"data": data}))).await?;
        }
        Ok(())
    }

    // ------------------------------------------------------------- playback

    /// Asks TIDAL whether this track may be played here. `Ok(None)` = playable;
    /// `Ok(Some(code))` = refused with a 403/404 code such as `GEO_RESTRICTED`.
    pub async fn playback_restriction(&self, track_id: &str) -> Result<Option<String>> {
        match self.track_manifest(track_id).await {
            Ok(_) => Ok(None),
            Err(e) if e.status() == Some(404) => Ok(Some(e.code().unwrap_or("NOT_FOUND").to_owned())),
            Err(e) if e.status() == Some(403) => Ok(Some(e.code().unwrap_or("FORBIDDEN").to_owned())),
            Err(e) => Err(e),
        }
    }

    pub async fn track_manifest(&self, track_id: &str) -> Result<Manifest> {
        let mut query =
            q(&[("adaptive", "false"), ("manifestType", "MPEG_DASH"), ("uriScheme", "DATA"), ("usage", "PLAYBACK")]);
        for f in ["AACLC", "HEAACV1", "FLAC"] {
            query.push(("formats".into(), f.into()));
        }
        let doc = self.get(&format!("/trackManifests/{track_id}"), &query).await?;
        let d = &doc.data;
        Ok(Manifest {
            uri: jsonapi::attr_str(d, "uri").ok_or_else(|| ApiError::Other("manifest has no uri".into()))?,
            full: jsonapi::attr_str(d, "trackPresentation").as_deref() == Some("FULL"),
            drm: jsonapi::attr(d, "drmData").is_some_and(|v| !v.is_null()),
        })
    }

    pub async fn download(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ApiError::Http {
                status: status.as_u16(),
                detail: format!("download {}", strip_query(url)),
                code: None,
            });
        }
        Ok(resp.bytes().await?.to_vec())
    }
}

/// Tracks from a to-many linkage document, in linkage order.
fn linked_tracks(doc: &Doc) -> Vec<Track> {
    doc.data_array()
        .iter()
        .filter_map(|l| {
            let (t, id) = (jsonapi::str_of(l, "type")?, jsonapi::str_of(l, "id")?);
            jsonapi::track_from(doc.get(&t, &id)?, doc)
        })
        .collect()
}

/// Our query params that a `next` link does not already carry.
fn missing_params(next: &str, ours: &Query) -> Query {
    let qs = next.split_once('?').map(|(_, q)| q).unwrap_or("");
    let present: HashSet<String> = url::form_urlencoded::parse(qs.as_bytes()).map(|(k, _)| k.into_owned()).collect();
    ours.iter().filter(|(k, _)| !present.contains(k)).cloned().collect()
}

/// `Retry-After` in seconds or as an HTTP date, capped at `max_secs`.
pub fn retry_after(headers: &reqwest::header::HeaderMap, max_secs: u64) -> Option<Duration> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim();
    let secs = match v.parse::<u64>() {
        Ok(s) => s,
        Err(_) => {
            let at = chrono::DateTime::parse_from_rfc2822(v).ok()?;
            (at.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64
        }
    };
    Some(Duration::from_secs(secs.min(max_secs)))
}

/// Same scheme, host and port.
pub fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

/// A URL without its query string (signed CDN links carry tokens there).
pub fn strip_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500u64 << attempt.min(7))
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_link_params() {
        let ours = q(&[("countryCode", "NO"), ("include", "items")]);
        let m = missing_params("/playlists/1/relationships/items?page[cursor]=x&include=items", &ours);
        assert_eq!(m, q(&[("countryCode", "NO")]));
        assert_eq!(missing_params("/p?countryCode=NO&include=items", &ours), Query::new());
    }

    // ------------------------------------------------- mock-server tests
    use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn client(server: &MockServer, expires_at: i64) -> Client {
        let mut auth = AuthConfig::new("cid", None, "http://localhost:8765/callback");
        auth.token_url = format!("{}/token", server.uri());
        let tokens = TokenSet { access_token: "old".into(), refresh_token: Some("rt".into()), expires_at, scope: None };
        let c = Client::with_base(&format!("{}/v2", server.uri()), auth, tokens, None);
        c.set_country("NO");
        c
    }

    fn far() -> i64 {
        chrono::Utc::now().timestamp() + 3600
    }

    fn page(ids: &[&str], next: Option<&str>) -> serde_json::Value {
        let data: Vec<Value> =
            ids.iter().map(|id| json!({"id": id, "type": "tracks", "meta": {"itemId": format!("occ-{id}")}})).collect();
        let included: Vec<Value> = ids
            .iter()
            .map(
                |id| json!({"id": id, "type": "tracks", "attributes": {"title": format!("T{id}"), "duration": "PT1M"}}),
            )
            .collect();
        let mut links = json!({"self": "/x"});
        if let Some(n) = next {
            links["next"] = n.into();
        }
        json!({"data": data, "included": included, "links": links})
    }

    #[tokio::test]
    async fn follows_cursor_pages_and_keeps_params() {
        let server = MockServer::start().await;
        // Page 2's next link lacks countryCode/include; the client must add them back.
        Mock::given(method("GET"))
            .and(path("/v2/playlists/p/relationships/items"))
            .and(query_param("page[cursor]", "c2"))
            .and(query_param("countryCode", "NO"))
            .and(query_param("include", "items,items.artists,items.tracks:albums"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(&["3"], None)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/playlists/p/relationships/items"))
            .and(header("Authorization", "Bearer old"))
            .and(header("Accept", "application/vnd.api+json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(page(&["1", "2"], Some("/v2/playlists/p/relationships/items?page[cursor]=c2"))),
            )
            .expect(1)
            .mount(&server)
            .await;
        let items = client(&server, far()).playlist_items("p", &|| false).await.unwrap();
        assert_eq!(items.iter().map(|i| i.r.id.as_str()).collect::<Vec<_>>(), vec!["1", "2", "3"]);
        assert!(items.iter().all(|i| i.track.is_some()), "included tracks resolved on every page");
        assert_eq!(items[2].item_id, "occ-3");
    }

    #[tokio::test]
    async fn repeated_cursor_fails_instead_of_returning_part() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/playlists/p/relationships/items"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(page(&["1"], Some("/v2/playlists/p/relationships/items?page[cursor]=same"))),
            )
            .mount(&server)
            .await;
        assert!(client(&server, far()).playlist_items("p", &|| false).await.is_err());
    }

    #[test]
    fn retry_after_and_origins() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("Retry-After", "7".parse().unwrap());
        assert_eq!(retry_after(&h, 120), Some(Duration::from_secs(7)));
        let later = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc2822().replace("+0000", "GMT");
        h.insert("Retry-After", later.parse().unwrap());
        let d = retry_after(&h, 120).unwrap();
        assert!(d >= Duration::from_secs(28) && d <= Duration::from_secs(30), "{d:?}");
        assert!(same_origin("https://openapi.tidal.com/v2/x?y", API_BASE));
        assert!(!same_origin("http://openapi.tidal.com/v2/x", API_BASE));
        assert!(!same_origin("https://openapi.tidal.com.evil.example/v2", API_BASE));
        assert_eq!(strip_query("https://cdn/x.mp4?token=1"), "https://cdn/x.mp4");
    }

    #[tokio::test]
    async fn falls_back_when_include_is_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/playlists/p/relationships/items"))
            .and(query_param("include", "items,items.artists,items.tracks:albums"))
            .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"errors":[{"detail":"bad include"}]}"#))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/playlists/p/relationships/items"))
            .and(query_param("include", "items,items.artists,items.albums"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page(&["9"], None)))
            .mount(&server)
            .await;
        let items = client(&server, far()).playlist_items("p", &|| false).await.unwrap();
        assert_eq!(items.len(), 1);
    }

    struct FailThenOk(std::sync::atomic::AtomicUsize, u16);
    impl Respond for FailThenOk {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                ResponseTemplate::new(self.1).insert_header("Retry-After", "0")
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"data": {"id": "u1", "type": "users",
                    "attributes": {"username": "me", "country": "SE"}}}))
            }
        }
    }

    #[tokio::test]
    async fn retries_after_429() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/users/me"))
            .respond_with(FailThenOk(Default::default(), 429))
            .expect(2)
            .mount(&server)
            .await;
        let c = client(&server, far());
        let u = c.me().await.unwrap();
        assert_eq!((u.username.as_str(), c.country().as_str()), ("me", "SE"));
    }

    #[tokio::test]
    async fn refreshes_expired_token_and_on_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "new", "expires_in": 3600})))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/users/me"))
            .and(header("Authorization", "Bearer new"))
            .respond_with(FailThenOk(Default::default(), 401))
            .expect(2)
            .mount(&server)
            .await;
        // Token already expired → refresh before the first call; the 401 forces a second refresh.
        let c = client(&server, 0);
        c.me().await.unwrap();
        let t = c.tokens.lock().await;
        assert_eq!(t.access_token, "new");
        assert_eq!(t.refresh_token.as_deref(), Some("rt"), "old refresh token kept when none is returned");
    }

    #[tokio::test]
    async fn mutation_bodies_match_the_spec() {
        let server = MockServer::start().await;
        let p = "/v2/playlists/p/relationships/items";
        Mock::given(method("POST"))
            .and(path(p))
            .and(header("Content-Type", "application/vnd.api+json"))
            .and(header_exists("Idempotency-Key"))
            .and(body_json(json!({"data": [{"type": "tracks", "id": "1"}, {"type": "videos", "id": "2"}],
                "meta": {"onDuplicates": "SKIP", "positionBefore": "occ-9"}})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"meta": {"skipped": [{"id": "1"}]}})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path(p))
            .and(body_json(json!({"data": [{"type": "tracks", "id": "1", "meta": {"itemId": "occ-1"}}]})))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path(p))
            .and(body_json(json!({"data": [{"type": "tracks", "id": "3", "meta": {"itemId": "occ-3"}}],
                "meta": {"positionBefore": "occ-1"}})))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        let c = client(&server, far());
        let refs = vec![ItemRef::track("1"), ItemRef { kind: ItemKind::Video, id: "2".into() }];
        let skipped = c.add_items("p", &refs, Some("occ-9"), true, &|_| {}, &|| false).await.unwrap();
        assert_eq!(skipped, 1);
        c.remove_items("p", &[(ItemRef::track("1"), "occ-1".into())], &|_| {}, &|| false).await.unwrap();
        c.move_items("p", &[(ItemRef::track("3"), "occ-3".into())], Some("occ-1"), &|_| {}, &|| false).await.unwrap();
    }

    #[tokio::test]
    async fn batches_respect_spec_limits() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/playlists/p/relationships/items"))
            .respond_with(ResponseTemplate::new(201))
            .expect(3) // 120 items → 50 + 50 + 20
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/v2/playlists/p/relationships/items"))
            .respond_with(ResponseTemplate::new(202))
            .expect(2) // 21 moves → 20 + 1
            .mount(&server)
            .await;
        let c = client(&server, far());
        let refs: Vec<ItemRef> = (0..120).map(|n| ItemRef::track(n.to_string())).collect();
        let seen = std::sync::Mutex::new(Vec::new());
        c.add_items("p", &refs, None, false, &|d| seen.lock().unwrap().push(d), &|| false).await.unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![50, 100, 120]);
        let moves: Vec<(ItemRef, String)> = (0..21).map(|n| (ItemRef::track(n.to_string()), format!("o{n}"))).collect();
        c.move_items("p", &moves, None, &|_| {}, &|| false).await.unwrap();
        // Cancelling stops before the next batch.
        let r = c.add_items("p", &refs, None, false, &|_| {}, &|| true).await;
        assert!(matches!(r, Err(ApiError::Cancelled)));
    }

    #[tokio::test]
    async fn playback_restriction_reads_error_codes() {
        let server = MockServer::start().await;
        let forbidden = |code: &str| {
            ResponseTemplate::new(403).set_body_json(json!({"errors": [{"status": "403", "code": code,
                "detail": "Content is unavailable in your location"}]}))
        };
        Mock::given(method("GET"))
            .and(path("/v2/trackManifests/geo"))
            .respond_with(forbidden("GEO_RESTRICTED"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/trackManifests/app"))
            .respond_with(forbidden("CLIENT_NOT_ENTITLED"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/trackManifests/ok"))
            .and(query_param("usage", "PLAYBACK"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data": {"id": "ok", "type": "trackManifests",
                "attributes": {"uri": "data:application/dash+xml;base64,PE1QRC8+", "trackPresentation": "PREVIEW"}}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/trackManifests/flaky"))
            .respond_with(ResponseTemplate::new(400).set_body_string("{}"))
            .mount(&server)
            .await;
        let c = client(&server, far());
        assert_eq!(c.playback_restriction("geo").await.unwrap().as_deref(), Some("GEO_RESTRICTED"));
        assert_eq!(c.playback_restriction("app").await.unwrap().as_deref(), Some("CLIENT_NOT_ENTITLED"));
        assert_eq!(c.playback_restriction("ok").await.unwrap(), None);
        assert!(c.playback_restriction("flaky").await.is_err(), "other errors are reported, not hidden");
        let e = c.track_manifest("geo").await.unwrap_err();
        assert_eq!(e.code(), Some("GEO_RESTRICTED"));
    }

    #[tokio::test]
    async fn similar_tracks_and_genres() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/tracks/1/relationships/similarTracks"))
            .and(query_param("include", "similarTracks,similarTracks.artists,similarTracks.albums"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"type": "tracks", "id": "7"}, {"type": "tracks", "id": "8"}],
                "included": [
                    {"type": "tracks", "id": "7", "attributes": {"title": "Seven"},
                     "relationships": {"artists": {"data": [{"type": "artists", "id": "a"}]}}},
                    {"type": "tracks", "id": "8", "attributes": {"title": "Eight"}},
                    {"type": "artists", "id": "a", "attributes": {"name": "Ann"}}
                ]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/tracks/gone/relationships/similarTracks"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/genres"))
            .and(query_param("filter[id]", "USER_SELECTABLE"))
            .and(query_param("locale", "nb-NO"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"type": "genres", "id": "g1", "attributes": {"genreName": "Pop"}}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/tracks"))
            .and(query_param("includeLinkage", "genres"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"type": "tracks", "id": "7", "relationships": {"genres": {"data": [{"type": "genres", "id": "g1"}]}}},
                    {"type": "tracks", "id": "8", "relationships": {"genres": {"data": []}}}
                ]
            })))
            .mount(&server)
            .await;
        let c = client(&server, far());
        let sim = c.similar_tracks("1").await.unwrap();
        assert_eq!(sim.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Seven", "Eight"]);
        assert_eq!(sim[0].artists, ["Ann"]);
        assert!(c.similar_tracks("gone").await.unwrap().is_empty(), "a missing seed gives no suggestions");
        let g = c.genres("nb-NO").await.unwrap();
        assert_eq!((g[0].id.as_str(), g[0].name.as_str()), ("g1", "Pop"));
        let tags = c.track_genres(&["7".into(), "8".into()], &|| false).await.unwrap();
        assert_eq!(tags.get("7"), Some(&vec!["g1".to_owned()]));
        assert!(!tags.contains_key("8"), "no genres means no entry");
    }
}
