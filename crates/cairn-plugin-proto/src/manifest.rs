use serde::{Deserialize, Serialize};

/// Declarative plugin manifest loaded before process spawn per RFC 007.
///
/// This is the wire-format manifest (JSON), distinct from the host-side
/// `PluginManifest` in cairn-tools which adds runtime metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifestWire {
    pub id: String,
    pub name: String,
    pub version: String,
    pub command: Vec<String>,
    pub capabilities: Vec<CapabilityWire>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub limits: Option<LimitsWire>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Capability declaration in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityWire {
    #[serde(rename = "type")]
    pub capability_type: String,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub signals: Option<Vec<String>>,
    #[serde(default)]
    pub channels: Option<Vec<String>>,
}

/// Concurrency and timeout limits in the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsWire {
    #[serde(rename = "maxConcurrency")]
    pub max_concurrency: Option<u32>,
    #[serde(rename = "defaultTimeoutMs")]
    pub default_timeout_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip() {
        let json = r#"{
            "id": "com.example.git-tools",
            "name": "Git Tools",
            "version": "0.1.0",
            "command": ["plugin-binary", "--serve"],
            "capabilities": [
                { "type": "tool_provider", "tools": ["git.status", "git.diff"] }
            ],
            "permissions": ["fs.read", "process.exec"],
            "limits": { "maxConcurrency": 4, "defaultTimeoutMs": 30000 }
        }"#;

        let manifest: PluginManifestWire = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.id, "com.example.git-tools");
        assert_eq!(manifest.capabilities.len(), 1);
        assert_eq!(manifest.permissions, vec!["fs.read", "process.exec"]);
        assert_eq!(manifest.limits.as_ref().unwrap().max_concurrency, Some(4));
    }
}
