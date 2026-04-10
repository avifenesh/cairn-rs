//! Bundled plugin catalog for cairn — RFC 015.
//!
//! Loads curated plugin descriptors from a static TOML catalog embedded
//! in the binary. At startup the catalog loader emits `PluginListed`
//! events for each entry. Plugins are listed-but-not-installed on
//! first boot.
//!
//! The catalog contains **descriptors only**, not binaries. Each entry
//! includes a stable identifier, a download URL (or instructions for
//! the operator to provide a local path), and marketplace metadata.

use serde::Deserialize;

/// The bundled catalog TOML, embedded at compile time.
pub const CATALOG_TOML: &str = include_str!("../catalog.toml");

// ── Wire types for TOML deserialization ──────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CatalogFile {
    #[serde(rename = "plugin")]
    pub plugins: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    pub category: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    pub vendor: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub signal_sources: Vec<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub has_signal_source: bool,
    #[serde(default)]
    pub network_egress: Vec<String>,
    #[serde(default)]
    pub required_credentials: Vec<CatalogCredentialSpec>,
    #[serde(default)]
    pub health_check: Option<CatalogHealthCheck>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogCredentialSpec {
    pub key: String,
    pub display_name: String,
    pub kind: String,
    pub scope: String,
    #[serde(default)]
    pub generated: bool,
    #[serde(default)]
    pub help_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogHealthCheck {
    pub method: String,
    pub timeout_ms: u64,
}

/// Parse the bundled catalog TOML.
pub fn load_bundled_catalog() -> Result<Vec<CatalogEntry>, CatalogError> {
    let catalog: CatalogFile =
        toml::from_str(CATALOG_TOML).map_err(|e| CatalogError::ParseError(e.to_string()))?;
    Ok(catalog.plugins)
}

/// Parse a catalog from an arbitrary TOML string (for testing or custom catalogs).
pub fn load_catalog_from_str(toml_str: &str) -> Result<Vec<CatalogEntry>, CatalogError> {
    let catalog: CatalogFile =
        toml::from_str(toml_str).map_err(|e| CatalogError::ParseError(e.to_string()))?;
    Ok(catalog.plugins)
}

#[derive(Debug)]
pub enum CatalogError {
    ParseError(String),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::ParseError(e) => write!(f, "catalog parse error: {e}"),
        }
    }
}

impl std::error::Error for CatalogError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_catalog_parses() {
        let entries = load_bundled_catalog().unwrap();
        assert!(!entries.is_empty(), "bundled catalog should have at least one entry");
    }

    #[test]
    fn github_entry_present_and_correct() {
        let entries = load_bundled_catalog().unwrap();
        let github = entries.iter().find(|e| e.id == "github").expect("github entry must exist");

        assert_eq!(github.name, "GitHub");
        assert_eq!(github.category, "issue_tracker");
        assert_eq!(github.vendor, "cairn");
        assert!(github.has_signal_source);
        assert!(github.download_url.is_some());
        assert_eq!(github.tools.len(), 19);
        assert_eq!(github.signal_sources.len(), 11);
        assert_eq!(github.required_credentials.len(), 4);
        assert!(github.health_check.is_some());
    }

    #[test]
    fn github_credentials_match_rfc_017() {
        let entries = load_bundled_catalog().unwrap();
        let github = entries.iter().find(|e| e.id == "github").unwrap();

        let cred_keys: Vec<&str> = github.required_credentials.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            cred_keys,
            vec!["github_app_id", "github_app_private_key", "github_webhook_secret", "github_installation_id"]
        );

        // webhook_secret is tenant-scoped and auto-generated
        let webhook_secret = github.required_credentials.iter().find(|c| c.key == "github_webhook_secret").unwrap();
        assert_eq!(webhook_secret.scope, "tenant");
        assert!(webhook_secret.generated);

        // installation_id is project-scoped
        let install_id = github.required_credentials.iter().find(|c| c.key == "github_installation_id").unwrap();
        assert_eq!(install_id.scope, "project");
    }

    #[test]
    fn github_tools_match_rfc_017() {
        let entries = load_bundled_catalog().unwrap();
        let github = entries.iter().find(|e| e.id == "github").unwrap();

        // Verify the 12 read-only + 7 mutating = 19 tools
        assert!(github.tools.contains(&"github.get_issue".to_string()));
        assert!(github.tools.contains(&"github.create_pull_request".to_string()));
        assert!(github.tools.contains(&"github.merge_pull_request".to_string()));
        assert!(github.tools.contains(&"github.get_rate_limit".to_string()));
    }

    #[test]
    fn custom_catalog_parses() {
        let custom = r#"
            [[plugin]]
            id = "slack"
            name = "Slack"
            version = "0.1.0"
            category = "chat_ops"
            vendor = "custom"
            command = ["cairn-plugin-slack"]
        "#;
        let entries = load_catalog_from_str(custom).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "slack");
    }
}
