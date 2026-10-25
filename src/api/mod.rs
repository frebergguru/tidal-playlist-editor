pub mod auth;
pub mod client;
pub mod jsonapi;
pub mod models;

pub use client::Client;
pub use models::*;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("HTTP {status}: {detail}")]
    Http { status: u16, detail: String, code: Option<String> },
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("login: {0}")]
    Auth(String),
    #[error("not logged in")]
    NotLoggedIn,
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl ApiError {
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The JSON:API error `code`, e.g. `GEO_RESTRICTED`.
    pub fn code(&self) -> Option<&str> {
        match self {
            ApiError::Http { code, .. } => code.as_deref(),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
