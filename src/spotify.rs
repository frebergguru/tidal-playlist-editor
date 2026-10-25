//! Spotify Web API client (only what cloning playlists needs).
//!
//! Endpoints and fields follow refs/spotify-openapi.yaml. Uses the non-deprecated
//! `/me/playlists`, `/playlists/{id}/items` (`item`, not `track`) and `/me/tracks`.
//! Nothing read from Spotify is cached on disk (Spotify Developer Terms).

use std::path::PathBuf;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::api::auth::{self, AuthConfig, TokenSet};
use crate::api::{ApiError, ItemKind, Result, Track};

pub const API_BASE: &str = "https://api.spotify.com/v1";
/// `POST /playlists/{id}/items`: "A maximum of 100 items can be added in one request."
pub const ADD_BATCH: usize = 100;
/// Paged reads (`limit` maximum 50) and search (`limit` maximum 10).
const PAGE: usize = 50;
const SEARCH_LIMIT: usize = 10;
const MAX_ATTEMPTS: u32 = 6;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SpUser {
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpPlaylist {
    /// Empty id = the user's Liked Songs.
    pub id: String,
    pub name: String,
    pub description: String,
    pub public: bool,
    pub owner_id: String,
    pub total: Option<u32>,
}

impl SpPlaylist {
    pub fn is_liked_songs(&self) -> bool {
        self.id.is_empty()
    }
}

/// Result of reading a Spotify playlist: matchable tracks plus what had to be skipped.
#[derive(Clone, Debug, Default)]
pub struct SpItems {
    pub tracks: Vec<Track>,
    /// Local files and podcast episodes cannot be moved to TIDAL.
    pub skipped: usize,
}

pub struct SpotifyClient {
    http: reqwest::Client,
    base: String,
    auth: AuthConfig,
    tokens: Mutex<TokenSet>,
    token_path: Option<PathBuf>,
}

/// Spotify error bodies: `{"error": {"status": 400, "message": "..."}}` or OAuth style.
fn error_message(body: &str) -> String {
    let v: Option<Value> = serde_json::from_str(body).ok();
    v.as_ref()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| v.get("error_description").and_then(Value::as_str))
                .or_else(|| v.get("error").and_then(Value::as_str))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| body.chars().take(300).collect())
}

/// Prefix of [`ApiError::Auth`] messages that mean «connect Spotify again».
pub const LOGIN_ERROR: &str = "Spotify login";

fn login_error(why: &str) -> ApiError {
    ApiError::Auth(format!("{LOGIN_ERROR}: {why}"))
}

pub fn is_login_error(e: &ApiError) -> bool {
    matches!(e, ApiError::Auth(m) if m.starts_with(LOGIN_ERROR))
}

pub fn track_uri(id: &str) -> String {
    format!("spotify:track:{id}")
}

pub fn track_url(id: &str) -> String {
    format!("https://open.spotify.com/track/{id}")
}

/// Spotify TrackObject → our [`Track`] (id = Spotify id). Returns `None` for
/// episodes, local files and removed items.
pub fn track_from(v: &Value) -> Option<Track> {
    if v.get("type").and_then(Value::as_str).is_some_and(|t| t != "track") {
        return None;
    }
    if v.get("is_local").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let id = v.get("id").and_then(Value::as_str)?.to_owned();
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let playable = v.get("is_playable").and_then(Value::as_bool);
    Some(Track {
        id,
        kind: ItemKind::Track,
        title: s("name").unwrap_or_default(),
        version: None,
        isrc: v.pointer("/external_ids/isrc").and_then(Value::as_str).map(str::to_owned),
        duration_secs: v.get("duration_ms").and_then(Value::as_u64).map_or(0, |ms| (ms / 1000) as u32),
        explicit: v.get("explicit").and_then(Value::as_bool).unwrap_or(false),
        artists: v
            .get("artists")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).map(str::to_owned).collect())
            .unwrap_or_default(),
        artist_ids: Vec::new(),
        album: v.pointer("/album/name").and_then(Value::as_str).map(str::to_owned),
        album_id: None,
        // `is_playable` is only present with a market; false = not playable there.
        availability: (playable == Some(false)).then(Vec::new),
    })
}

/// Text for Spotify's field-filtered search: `track:… artist:…`.
pub fn search_query(title: &str, artist: Option<&str>) -> String {
    let clean = |s: &str| s.replace(['"', ':'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    match artist.filter(|a| !a.trim().is_empty()) {
        Some(a) => format!("track:{} artist:{}", clean(title), clean(a)),
        None => format!("track:{}", clean(title)),
    }
}

impl SpotifyClient {
    pub fn new(auth: AuthConfig, tokens: TokenSet, token_path: Option<PathBuf>) -> Self {
        Self::with_base(API_BASE, auth, tokens, token_path)
    }

    pub fn with_base(base: &str, auth: AuthConfig, tokens: TokenSet, token_path: Option<PathBuf>) -> Self {
        Self {
            http: crate::api::Client::http(),
            base: base.trim_end_matches('/').to_owned(),
            auth,
            tokens: Mutex::new(tokens),
            token_path,
        }
    }

    /// `rejected` is a token that just got a 401; it is refreshed unless
    /// another request has already replaced it.
    async fn access_token(&self, rejected: Option<&str>) -> Result<String> {
        let mut t = self.tokens.lock().await;
        let force = rejected.is_some_and(|r| r == t.access_token);
        if force || t.expires_soon() {
            let Some(rt) = t.refresh_token.clone() else { return Err(login_error("no refresh token")) };
            // A failed refresh (e.g. revoked/expired refresh token) means logging in again.
            let fresh = auth::refresh(&self.http, &self.auth, &rt).await.map_err(|e| match e {
                ApiError::Auth(m) => login_error(&m),
                other => other,
            })?;
            if let Some(p) = &self.token_path
                && let Err(e) = auth::save_tokens(p, &fresh)
            {
                tracing::warn!("could not save Spotify tokens: {e}");
            }
            *t = fresh;
        }
        Ok(t.access_token.clone())
    }

    /// One request with exponential backoff on 429/5xx (honouring Retry-After)
    /// and a single token refresh on 401. Spotify has no idempotency keys, so a
    /// write is retried only when it surely was not applied (connect error, 429).
    async fn send(&self, method: Method, url: &str, query: &[(&str, String)], body: Option<&Value>) -> Result<Value> {
        let url = if url.starts_with("http") {
            // `next` links must stay on the API host so the token goes nowhere else.
            if !crate::api::client::same_origin(url, &self.base) {
                let shown = crate::api::client::strip_query(url);
                return Err(ApiError::Other(format!("Spotify: refusing link to another host: {shown}")));
            }
            url.to_owned()
        } else {
            format!("{}{url}", self.base)
        };
        let idempotent = method == Method::GET;
        let mut refreshed = false;
        for attempt in 1..=MAX_ATTEMPTS {
            let token = self.access_token(None).await?;
            let mut req = self.http.request(method.clone(), &url).bearer_auth(&token).query(query);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if attempt < MAX_ATTEMPTS && (e.is_connect() || (idempotent && e.is_timeout())) => {
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
            let retryable = status == StatusCode::TOO_MANY_REQUESTS || (idempotent && status.is_server_error());
            if retryable && attempt < MAX_ATTEMPTS {
                let wait = crate::api::client::retry_after(resp.headers(), 300)
                    .map(|d| d.max(Duration::from_secs(1)))
                    .unwrap_or_else(|| backoff(attempt));
                tracing::debug!("Spotify {status}, waiting {wait:?}");
                tokio::time::sleep(wait).await;
                continue;
            }
            let text = resp.text().await?;
            if !status.is_success() {
                let msg = error_message(&text);
                return Err(ApiError::Http { status: status.as_u16(), detail: format!("Spotify: {msg}"), code: None });
            }
            return if text.trim().is_empty() {
                Ok(Value::Null)
            } else {
                serde_json::from_str(&text).map_err(|e| ApiError::Other(format!("Spotify: bad JSON: {e}")))
            };
        }
        Err(ApiError::Other("Spotify: too many retries".into()))
    }

    /// Follows offset paging through `next` (absolute URLs).
    async fn get_all(
        &self,
        path: &str,
        query: &[(&str, String)],
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut v = self.send(Method::GET, path, query, None).await?;
        loop {
            out.extend(v.get("items").and_then(Value::as_array).cloned().unwrap_or_default());
            let Some(next) = v.get("next").and_then(Value::as_str).map(str::to_owned) else { break };
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            v = self.send(Method::GET, &next, &[], None).await?;
        }
        Ok(out)
    }

    pub async fn me(&self) -> Result<SpUser> {
        let v = self.send(Method::GET, "/me", &[], None).await?;
        let id = v.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
        let display_name =
            v.get("display_name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(&id).to_owned();
        Ok(SpUser { id, display_name })
    }

    /// The user's playlists (owned and followed), with Liked Songs first.
    pub async fn playlists(&self) -> Result<Vec<SpPlaylist>> {
        let items = self.get_all("/me/playlists", &[("limit", PAGE.to_string())], &|| false).await?;
        let mut out = vec![SpPlaylist { name: "Liked Songs".into(), ..Default::default() }];
        out.extend(items.iter().filter(|p| !p.is_null()).map(|p| SpPlaylist {
            id: p.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
            name: p.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
            description: p.get("description").and_then(Value::as_str).unwrap_or_default().to_owned(),
            public: p.get("public").and_then(Value::as_bool).unwrap_or(false),
            owner_id: p.pointer("/owner/id").and_then(Value::as_str).unwrap_or_default().to_owned(),
            total: p.pointer("/items/total").and_then(Value::as_u64).map(|n| n as u32),
        }));
        Ok(out)
    }

    pub async fn playlist_tracks(&self, p: &SpPlaylist, cancel: &(dyn Fn() -> bool + Sync)) -> Result<SpItems> {
        let market = ("market", "from_token".to_owned());
        let (rows, field) = if p.is_liked_songs() {
            (self.get_all("/me/tracks", &[("limit", PAGE.to_string()), market], cancel).await?, "track")
        } else {
            let path = format!("/playlists/{}/items", p.id);
            let q = [("limit", PAGE.to_string()), market, ("additional_types", "track".to_owned())];
            (self.get_all(&path, &q, cancel).await?, "item")
        };
        let mut out = SpItems::default();
        for r in rows {
            match r.get(field).and_then(track_from) {
                Some(t) => out.tracks.push(t),
                None => out.skipped += 1,
            }
        }
        Ok(out)
    }

    pub async fn search_tracks(&self, q: &str) -> Result<Vec<Track>> {
        let query = [
            ("q", q.to_owned()),
            ("type", "track".to_owned()),
            ("limit", SEARCH_LIMIT.to_string()),
            ("market", "from_token".to_owned()),
        ];
        let v = self.send(Method::GET, "/search", &query, None).await?;
        Ok(v.pointer("/tracks/items")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(track_from).collect())
            .unwrap_or_default())
    }

    pub async fn create_playlist(&self, name: &str, description: &str, public: bool) -> Result<SpPlaylist> {
        let mut body = json!({"name": name, "public": public});
        if !description.is_empty() {
            body["description"] = description.into();
        }
        let v = self.send(Method::POST, "/me/playlists", &[], Some(&body)).await?;
        Ok(SpPlaylist {
            id: v.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            public,
            ..Default::default()
        })
    }

    /// Adds track ids in order, 100 per request.
    pub async fn add_tracks(
        &self,
        playlist_id: &str,
        ids: &[String],
        progress: &(dyn Fn(usize) + Sync),
        cancel: &(dyn Fn() -> bool + Sync),
    ) -> Result<()> {
        let path = format!("/playlists/{playlist_id}/items");
        let mut done = 0;
        for chunk in ids.chunks(ADD_BATCH) {
            if cancel() {
                return Err(ApiError::Cancelled);
            }
            let uris: Vec<String> = chunk.iter().map(|id| track_uri(id)).collect();
            self.send(Method::POST, &path, &[], Some(&json!({"uris": uris}))).await?;
            done += chunk.len();
            progress(done);
        }
        Ok(())
    }
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(1000u64 << attempt.min(6))
}

impl crate::ops::matcher::Catalog for SpotifyClient {
    fn native_ids(&self) -> bool {
        false
    }

    async fn lookup_ids(&self, _ids: &[String], _cancel: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Track>> {
        // Not used: rows coming from TIDAL carry TIDAL ids (see `native_ids`).
        Ok(Vec::new())
    }

    async fn by_isrc(&self, isrc: &str) -> Result<Vec<Track>> {
        self.search_tracks(&format!("isrc:{isrc}")).await
    }

    async fn search(&self, row: &crate::ops::files::ImportRow) -> Result<Vec<Track>> {
        let title = crate::ops::matcher::normalise(&row.title);
        let found = self.search_tracks(&search_query(&title, row.artists.first().map(String::as_str))).await?;
        if !found.is_empty() {
            return Ok(found);
        }
        // Field filters can be too strict (e.g. "feat." spellings); try free text.
        self.search_tracks(&crate::ops::matcher::search_text(row)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> SpotifyClient {
        let mut auth = AuthConfig::spotify("cid", "http://127.0.0.1:8766/callback");
        auth.token_url = format!("{}/api/token", server.uri());
        let tokens = TokenSet {
            access_token: "tok".into(),
            refresh_token: Some("rt".into()),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            scope: None,
        };
        SpotifyClient::with_base(&format!("{}/v1", server.uri()), auth, tokens, None)
    }

    fn sp_track(id: &str, isrc: &str) -> Value {
        json!({"type": "track", "id": id, "name": format!("Song {id}"), "duration_ms": 201_000, "explicit": false,
               "is_local": false, "external_ids": {"isrc": isrc}, "artists": [{"name": "Band"}], "album": {"name": "LP"}})
    }

    #[test]
    fn maps_tracks_and_skips_episodes_and_local_files() {
        let t = track_from(&sp_track("a", "US1")).unwrap();
        assert_eq!(
            (t.title.as_str(), t.isrc.as_deref(), t.duration_secs, t.album.as_deref()),
            ("Song a", Some("US1"), 201, Some("LP"))
        );
        assert!(t.streamable());
        let mut blocked = sp_track("b", "US2");
        blocked["is_playable"] = false.into();
        assert!(!track_from(&blocked).unwrap().streamable());
        assert!(track_from(&json!({"type": "episode", "id": "e"})).is_none());
        assert!(track_from(&json!({"type": "track", "id": "l", "is_local": true})).is_none());
        assert_eq!(search_query("Don't: Stop", Some("Queen")), "track:Don't Stop artist:Queen");
        assert_eq!(
            error_message(r#"{"error":{"status":403,"message":"Insufficient client scope"}}"#),
            "Insufficient client scope"
        );
        assert_eq!(
            error_message(r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#),
            "Refresh token revoked"
        );
    }

    #[tokio::test]
    async fn reads_playlist_items_with_paging() {
        let server = MockServer::start().await;
        let next = format!("{}/v1/playlists/p1/items?offset=50&limit=50", server.uri());
        Mock::given(method("GET"))
            .and(path("/v1/playlists/p1/items"))
            .and(query_param("offset", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [{"item": {"type": "episode", "id": "ep"}}, {"is_local": true, "item": {"type": "track", "is_local": true, "id": "x"}}],
                "next": null})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/playlists/p1/items"))
            .and(query_param("market", "from_token"))
            .and(query_param("limit", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [{"item": sp_track("a", "US1")}, {"item": sp_track("b", "US2")}], "next": next})))
            .mount(&server)
            .await;
        let p = SpPlaylist { id: "p1".into(), ..Default::default() };
        let r = client(&server).playlist_tracks(&p, &|| false).await.unwrap();
        assert_eq!(r.tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(r.skipped, 2);
    }

    #[tokio::test]
    async fn creates_playlist_and_adds_in_batches_of_100() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/me/playlists"))
            .and(body_json(json!({"name": "Mix", "public": false, "description": "From TIDAL"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "new1"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/playlists/new1/items"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"snapshot_id": "s"})))
            .expect(3)
            .mount(&server)
            .await;
        let c = client(&server);
        let p = c.create_playlist("Mix", "From TIDAL", false).await.unwrap();
        assert_eq!(p.id, "new1");
        let ids: Vec<String> = (0..250).map(|n| format!("t{n}")).collect();
        c.add_tracks("new1", &ids, &|_| {}, &|| false).await.unwrap();
        let reqs = server.received_requests().await.unwrap();
        let first: Value = serde_json::from_slice(&reqs[1].body).unwrap();
        assert_eq!(first["uris"].as_array().unwrap().len(), 100);
        assert_eq!(first["uris"][0], "spotify:track:t0");
    }

    #[tokio::test]
    async fn backs_off_on_429_and_reports_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/search"))
            .and(query_param("q", "isrc:US1"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/search"))
            .and(query_param("q", "isrc:US1"))
            .and(query_param("type", "track"))
            .and(query_param("limit", "10"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"tracks": {"items": [sp_track("a", "US1")]}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/me"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(json!({"error": {"status": 403, "message": "Insufficient client scope"}})),
            )
            .mount(&server)
            .await;
        let c = client(&server);
        let started = std::time::Instant::now();
        let found = crate::ops::matcher::Catalog::by_isrc(&c, "US1").await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(900), "waited for Retry-After");
        assert_eq!(found[0].id, "a");
        let e = c.me().await.unwrap_err();
        assert_eq!(e.status(), Some(403));
        assert!(e.to_string().contains("Insufficient client scope"));
    }

    #[tokio::test]
    async fn writes_are_not_retried_after_a_server_error() {
        // A 502 after a POST may mean it was applied; retrying would duplicate.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/me/playlists"))
            .respond_with(ResponseTemplate::new(502))
            .expect(1)
            .mount(&server)
            .await;
        let e = client(&server).create_playlist("Mix", "", false).await.unwrap_err();
        assert_eq!(e.status(), Some(502));
    }

    #[tokio::test]
    async fn next_links_to_other_hosts_are_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/me/playlists"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"items": [], "next": "https://evil.example/v1/me/playlists?offset=50&token=x"}),
                ),
            )
            .mount(&server)
            .await;
        let e = client(&server).playlists().await.unwrap_err();
        assert!(e.to_string().contains("another host"), "{e}");
        assert!(!e.to_string().contains("token=x"), "query is not shown");
    }
}
