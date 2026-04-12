//! Error types for the GitHub integration.

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("invalid webhook signature")]
    InvalidSignature,

    #[error("API request failed: {status} {body}")]
    Api { status: u16, body: String },

    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("JWT encoding error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("invalid private key: {0}")]
    InvalidKey(String),

    #[error("installation token expired or missing — call refresh_token()")]
    TokenExpired,
}
