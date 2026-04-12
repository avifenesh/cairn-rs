//! GitHub App authentication — JWT generation and installation access tokens.
//!
//! Flow:
//! 1. Load PEM private key → `AppCredentials`
//! 2. Generate short-lived JWT (10 min max per GitHub spec)
//! 3. Exchange JWT for an installation access token (1 hour, auto-refreshed)

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::error::GitHubError;

/// GitHub App credentials — app ID + PEM private key.
#[derive(Clone)]
pub struct AppCredentials {
    pub app_id: u64,
    encoding_key: EncodingKey,
}

impl AppCredentials {
    /// Create credentials from an app ID and PEM-encoded RSA private key.
    pub fn new(app_id: u64, pem_key: &[u8]) -> Result<Self, GitHubError> {
        let encoding_key = EncodingKey::from_rsa_pem(pem_key)
            .map_err(|e| GitHubError::InvalidKey(e.to_string()))?;
        Ok(Self {
            app_id,
            encoding_key,
        })
    }

    /// Generate a short-lived JWT for GitHub App API calls.
    ///
    /// Valid for 10 minutes (GitHub maximum). Used to exchange for
    /// installation access tokens.
    pub fn generate_jwt(&self) -> Result<String, GitHubError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let claims = JwtClaims {
            iat: now.saturating_sub(60), // 60s clock skew buffer
            exp: now + 600,              // 10 minutes
            iss: self.app_id.to_string(),
        };

        let header = Header::new(Algorithm::RS256);
        Ok(encode(&header, &claims, &self.encoding_key)?)
    }
}

impl AppCredentials {
    /// List all installations for this GitHub App.
    pub async fn list_installations(
        &self,
        http: &reqwest::Client,
    ) -> Result<Vec<AppInstallation>, GitHubError> {
        let jwt = self.generate_jwt()?;
        let resp = http
            .get("https://api.github.com/app/installations")
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "cairn-github/0.1")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api { status, body });
        }
        Ok(resp.json().await?)
    }
}

/// A GitHub App installation.
#[derive(Clone, Debug, Deserialize)]
pub struct AppInstallation {
    pub id: u64,
    pub account: InstallationAccount,
    #[serde(default)]
    pub repository_selection: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InstallationAccount {
    pub login: String,
    pub id: u64,
}

impl std::fmt::Debug for AppCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppCredentials")
            .field("app_id", &self.app_id)
            .field("encoding_key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Serialize)]
struct JwtClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

/// A cached installation access token with automatic refresh.
#[derive(Clone, Debug)]
pub struct InstallationToken {
    credentials: AppCredentials,
    installation_id: u64,
    token_cache: Arc<RwLock<Option<CachedToken>>>,
    http: reqwest::Client,
}

#[derive(Clone, Debug)]
struct CachedToken {
    token: String,
    expires_at: SystemTime,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
    expires_at: String,
}

impl InstallationToken {
    /// Create a new token manager for an installation.
    pub fn new(credentials: AppCredentials, installation_id: u64, http: reqwest::Client) -> Self {
        Self {
            credentials,
            installation_id,
            token_cache: Arc::new(RwLock::new(None)),
            http,
        }
    }

    /// Get a valid access token, refreshing if expired or missing.
    pub async fn get(&self) -> Result<String, GitHubError> {
        // Check cache first.
        {
            let cache = self.token_cache.read().await;
            if let Some(ref cached) = *cache {
                let now = SystemTime::now();
                // Refresh 5 minutes before expiry.
                if cached.expires_at > now + Duration::from_secs(300) {
                    return Ok(cached.token.clone());
                }
            }
        }

        // Refresh.
        self.refresh().await
    }

    /// Force-refresh the installation access token.
    pub async fn refresh(&self) -> Result<String, GitHubError> {
        let jwt = self.credentials.generate_jwt()?;

        let url = format!(
            "https://api.github.com/app/installations/{}/access_tokens",
            self.installation_id
        );

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "cairn-github/0.1")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status != 201 {
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubError::Api { status, body });
        }

        let token_resp: TokenResponse = resp.json().await?;

        // Parse expires_at (ISO 8601) — fall back to 1 hour from now.
        let expires_at = parse_iso8601(&token_resp.expires_at)
            .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(3600));

        let token = token_resp.token.clone();

        let mut cache = self.token_cache.write().await;
        *cache = Some(CachedToken {
            token: token_resp.token,
            expires_at,
        });

        tracing::info!(
            installation_id = self.installation_id,
            "GitHub installation access token refreshed"
        );

        Ok(token)
    }
}

fn parse_iso8601(s: &str) -> Option<SystemTime> {
    // GitHub returns: "2024-01-01T00:00:00Z"
    // Simple parser — no chrono dependency needed.
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = s.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u64 = date_parts.next()?.parse().ok()?;
    let day: u64 = date_parts.next()?.parse().ok()?;

    let mut time_parts = time.split(':');
    let hour: u64 = time_parts.next()?.parse().ok()?;
    let min: u64 = time_parts.next()?.parse().ok()?;
    let sec: u64 = time_parts.next()?.split('.').next()?.parse().ok()?;

    // Days since epoch (simplified — no leap second handling).
    fn days_from_civil(y: i64, m: u64, d: u64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let m = if m <= 2 { m + 9 } else { m - 3 } as i64;
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400) as u64;
        let doy = (153 * m as u64 + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe as i64 - 719468
    }

    let days = days_from_civil(year, month, day);
    let secs = days as u64 * 86400 + hour * 3600 + min * 60 + sec;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso8601_github_format() {
        let t = parse_iso8601("2026-04-12T15:30:00Z").unwrap();
        let secs = t.duration_since(UNIX_EPOCH).unwrap().as_secs();
        // 2026-04-12T15:30:00Z
        assert!(secs > 1_776_000_000);
        assert!(secs < 1_777_000_000);
    }

    #[test]
    fn parse_iso8601_with_fractional() {
        let t = parse_iso8601("2026-01-01T00:00:00.000Z").unwrap();
        let secs = t.duration_since(UNIX_EPOCH).unwrap().as_secs();
        assert!(secs > 0);
    }

    #[test]
    fn parse_iso8601_invalid_returns_none() {
        assert!(parse_iso8601("not-a-date").is_none());
        assert!(parse_iso8601("").is_none());
    }
}
