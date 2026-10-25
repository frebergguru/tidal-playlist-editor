//! OAuth 2.1 Authorization Code + PKCE against TIDAL, with a loopback redirect.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ApiError;

pub const AUTHORIZE_URL: &str = "https://login.tidal.com/authorize";
pub const TOKEN_URL: &str = "https://auth.tidal.com/v1/oauth2/token";

pub const SPOTIFY_AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
pub const SPOTIFY_TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
/// Minimum Spotify scopes: read own/collaborative playlists and Liked Songs,
/// create playlists and add items to them.
pub const SPOTIFY_SCOPES: &str = "playlist-read-private playlist-read-collaborative playlist-modify-private playlist-modify-public user-library-read";

/// Third-party scopes this app uses (see `components.securitySchemes` in the spec).
pub const SCOPES: &str =
    "user.read collection.read collection.write playlists.read playlists.write search.read playback";

#[derive(Clone, Debug)]
pub struct AuthConfig {
    pub client_id: String,
    /// Optional; PKCE public clients do not need one.
    pub client_secret: Option<String>,
    pub redirect_uri: String,
    pub authorize_url: String,
    pub token_url: String,
    pub scopes: String,
}

impl AuthConfig {
    pub fn new(client_id: &str, client_secret: Option<String>, redirect_uri: &str) -> Self {
        Self {
            client_id: client_id.trim().to_owned(),
            client_secret: client_secret.filter(|s| !s.trim().is_empty()),
            redirect_uri: redirect_uri.trim().to_owned(),
            authorize_url: AUTHORIZE_URL.into(),
            token_url: TOKEN_URL.into(),
            scopes: SCOPES.into(),
        }
    }

    /// Spotify Web API: Authorization Code + PKCE, no client secret.
    pub fn spotify(client_id: &str, redirect_uri: &str) -> Self {
        Self {
            client_id: client_id.trim().to_owned(),
            client_secret: None,
            redirect_uri: redirect_uri.trim().to_owned(),
            authorize_url: SPOTIFY_AUTHORIZE_URL.into(),
            token_url: SPOTIFY_TOKEN_URL.into(),
            scopes: SPOTIFY_SCOPES.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub scope: Option<String>,
}

impl TokenSet {
    pub fn expires_soon(&self) -> bool {
        chrono::Utc::now().timestamp() + 60 >= self.expires_at
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    pub fn new() -> Self {
        // Two v4 UUIDs give 244 random bits as 64 unreserved characters.
        let verifier = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge, state: uuid::Uuid::new_v4().simple().to_string() }
    }
}

pub fn authorize_url(cfg: &AuthConfig, pkce: &Pkce) -> String {
    let mut u = url::Url::parse(&cfg.authorize_url).expect("valid authorize url");
    u.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &cfg.client_id)
        .append_pair("redirect_uri", &cfg.redirect_uri)
        .append_pair("scope", &cfg.scopes)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("state", &pkce.state);
    u.into()
}

const DONE_PAGE: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>TIDAL Playlist Editor</title>\
<body style=\"font-family:sans-serif;font-size:1.25rem;background:#111;color:#eee;padding:2rem\">\
<h1>Login complete / Innlogging fullført</h1>\
<p>You can close this tab and return to TIDAL Playlist Editor.</p>\
<p lang=\"nb\">Du kan lukke denne fanen og gå tilbake til TIDAL Spillelisteredigering.</p></body></html>";

const FAILED_PAGE: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>TIDAL Playlist Editor</title>\
<body style=\"font-family:sans-serif;font-size:1.25rem;background:#111;color:#eee;padding:2rem\">\
<h1>Login failed / Innloggingen mislyktes</h1>\
<p>Return to TIDAL Playlist Editor to see why and try again.</p>\
<p lang=\"nb\">Gå tilbake til TIDAL Spillelisteredigering for å se hvorfor, og prøv igjen.</p></body></html>";

const STALE_PAGE: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>TIDAL Playlist Editor</title>\
<body style=\"font-family:sans-serif;font-size:1.25rem;background:#111;color:#eee;padding:2rem\">\
<h1>Old login link / Gammel innloggingslenke</h1>\
<p>This page belongs to an earlier login. Use the newest login tab.</p>\
<p lang=\"nb\">Denne siden hører til en tidligere innlogging. Bruk den nyeste innloggingsfanen.</p></body></html>";

/// Waits for the browser to hit the redirect URI and returns the authorization code.
/// Blocking; runs until a code arrives or `cancel` is set (no time limit).
pub fn wait_for_code(cfg: &AuthConfig, state: &str, cancel: Arc<AtomicBool>) -> Result<String, ApiError> {
    let redirect = url::Url::parse(&cfg.redirect_uri).map_err(|e| ApiError::Auth(format!("redirect URI: {e}")))?;
    let port = redirect.port_or_known_default().unwrap_or(80);
    // Browsers may resolve "localhost" to ::1 or 127.0.0.1, so listen on both.
    let mut servers = vec![
        tiny_http::Server::http(("127.0.0.1", port))
            .map_err(|e| ApiError::Auth(format!("cannot listen on 127.0.0.1:{port}: {e}")))?,
    ];
    if let Ok(v6) = tiny_http::Server::http(("::1", port)) {
        servers.push(v6);
    }
    let mut turn = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(ApiError::Cancelled);
        }
        turn = (turn + 1) % servers.len();
        let Ok(Some(req)) = servers[turn].recv_timeout(Duration::from_millis(125)) else { continue };
        let full = format!("http://127.0.0.1{}", req.url());
        let parsed = url::Url::parse(&full).ok();
        let Some(parsed) = parsed.filter(|u| u.path() == redirect.path()) else {
            let _ = req.respond(tiny_http::Response::empty(404));
            continue;
        };
        let q: std::collections::HashMap<String, String> = parsed.query_pairs().into_owned().collect();
        let html = tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8").expect("static header");
        // A redirect without our state did not come from this login (stale tab,
        // another app or a forged request): refuse it and keep waiting.
        if q.get("state").map(String::as_str) != Some(state) {
            tracing::warn!("ignoring login redirect with the wrong state");
            let _ = req.respond(tiny_http::Response::from_string(STALE_PAGE).with_header(html).with_status_code(400));
            continue;
        }
        if let Some(err) = q.get("error") {
            let _ = req.respond(tiny_http::Response::from_string(FAILED_PAGE).with_header(html));
            let desc = q.get("error_description").cloned().unwrap_or_default();
            return Err(ApiError::Auth(format!("{err} {desc}").trim().to_owned()));
        }
        let Some(code) = q.get("code").cloned() else {
            let _ = req.respond(tiny_http::Response::from_string(FAILED_PAGE).with_header(html));
            return Err(ApiError::Auth("no code in login redirect".into()));
        };
        let _ = req.respond(tiny_http::Response::from_string(DONE_PAGE).with_header(html));
        return Ok(code);
    }
}

async fn token_request(
    http: &reqwest::Client,
    cfg: &AuthConfig,
    form: &[(&str, &str)],
) -> Result<TokenResponse, ApiError> {
    let mut req = http.post(&cfg.token_url).form(form);
    if let Some(secret) = &cfg.client_secret {
        req = req.basic_auth(&cfg.client_id, Some(secret));
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(ApiError::Auth(format!("{status}: {}", super::jsonapi::error_detail(&body))));
    }
    serde_json::from_str(&body).map_err(|e| ApiError::Auth(format!("token response: {e}")))
}

fn to_set(r: TokenResponse, old_refresh: Option<&str>) -> TokenSet {
    TokenSet {
        access_token: r.access_token,
        refresh_token: r.refresh_token.or(old_refresh.map(str::to_owned)),
        expires_at: chrono::Utc::now().timestamp() + r.expires_in.unwrap_or(3600),
        scope: r.scope,
    }
}

pub async fn exchange_code(
    http: &reqwest::Client,
    cfg: &AuthConfig,
    code: &str,
    verifier: &str,
) -> Result<TokenSet, ApiError> {
    let r = token_request(
        http,
        cfg,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", &cfg.client_id),
            ("code", code),
            ("redirect_uri", &cfg.redirect_uri),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    Ok(to_set(r, None))
}

pub async fn refresh(http: &reqwest::Client, cfg: &AuthConfig, refresh_token: &str) -> Result<TokenSet, ApiError> {
    let r = token_request(
        http,
        cfg,
        &[("grant_type", "refresh_token"), ("client_id", &cfg.client_id), ("refresh_token", refresh_token)],
    )
    .await?;
    Ok(to_set(r, Some(refresh_token)))
}

pub fn load_tokens(path: &Path) -> Option<TokenSet> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Stores tokens readable only by the current user.
pub fn save_tokens(path: &Path, t: &TokenSet) -> std::io::Result<()> {
    crate::store::write_private(path, &serde_json::to_vec_pretty(t).expect("serialisable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = Pkce::new();
        assert_eq!(p.verifier.len(), 64);
        let expect = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes()));
        assert_eq!(p.challenge, expect);
        let cfg = AuthConfig::new("cid", None, "http://localhost:8765/callback");
        let u = authorize_url(&cfg, &p);
        assert!(u.starts_with(AUTHORIZE_URL));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(u.contains("redirect_uri=http%3A%2F%2Flocalhost%3A8765%2Fcallback"));
    }

    #[test]
    fn token_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.json");
        let t = TokenSet { access_token: "a".into(), refresh_token: Some("r".into()), expires_at: 5, scope: None };
        save_tokens(&p, &t).unwrap();
        let back = load_tokens(&p).unwrap();
        assert_eq!(back.refresh_token.as_deref(), Some("r"));
        assert!(back.expires_soon());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    fn hit(port: u16, path: &str) -> String {
        use std::io::{Read, Write};
        // The listener may need a moment to bind.
        for _ in 0..50 {
            if let Ok(mut c) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                write!(c, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
                let mut out = String::new();
                let _ = c.read_to_string(&mut out);
                return out;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("listener never came up");
    }

    #[test]
    fn loopback_receives_code_and_checks_state() {
        let cfg = AuthConfig::new("cid", None, "http://localhost:18765/callback");
        let cancel = Arc::new(AtomicBool::new(false));
        let (c2, k2) = (cfg.clone(), cancel.clone());
        let t = std::thread::spawn(move || wait_for_code(&c2, "S1", k2));
        assert!(hit(18765, "/favicon.ico").starts_with("HTTP/1.1 404"));
        let page = hit(18765, "/callback?code=abc&state=S1");
        assert!(page.contains("Login complete"));
        assert_eq!(t.join().unwrap().unwrap(), "abc");

        // Wrong state is refused (CSRF protection) and the real redirect still works.
        let (c3, k3) = (cfg.clone(), cancel.clone());
        let t = std::thread::spawn(move || wait_for_code(&c3, "S2", k3));
        assert!(hit(18765, "/callback?code=evil&state=evil").starts_with("HTTP/1.1 400"));
        assert!(hit(18765, "/callback?error=access_denied&state=S2").contains("Login failed"));
        assert!(matches!(t.join().unwrap(), Err(ApiError::Auth(_))));

        // Cancel ends the wait without a request.
        let k4 = Arc::new(AtomicBool::new(false));
        let (c4, k4b) = (cfg, k4.clone());
        let t = std::thread::spawn(move || wait_for_code(&c4, "S3", k4b));
        k4.store(true, Ordering::Relaxed);
        assert!(matches!(t.join().unwrap(), Err(ApiError::Cancelled)));
    }
}
