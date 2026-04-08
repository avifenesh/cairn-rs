//! SOUL.md domain types — agent identity and behavioral constitution.
//!
//! A `SoulDocument` is the persistent, versioned representation of an agent's
//! SOUL.md file. Fields are classified as either personality/identity (require
//! operator approval to modify) or operational (can be patched freely).
//!
//! Locked fields are immutable regardless of patch content.

use serde::{Deserialize, Serialize};

/// Canonical set of SOUL.md section names that are considered
/// personality/identity fields. Patches that add or alter text under these
/// headings require operator approval before being applied.
pub const PERSONALITY_FIELDS: &[&str] = &[
    "who i am",
    "voice",
    "values",
    "identity",
    "personality",
    "tone",
    "learning you",
    "learning",
    "character",
];

/// Canonical set of SOUL.md section names that are considered operational.
/// Patches to these sections are allowed without approval.
pub const OPERATIONAL_FIELDS: &[&str] = &[
    "auto-execute",
    "autonomy",
    "require approval",
    "never",
    "proactive behaviors",
    "commands",
    "tool config",
    "tool configs",
    "tools",
    "learned patterns",
];

/// The persistent, versioned SOUL.md document for an agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulDocument {
    /// Full text content of the SOUL.md file.
    pub content: String,
    /// Monotonically increasing version number, incremented on each applied patch.
    pub version: u32,
    /// Section names (lowercase) that are permanently locked and may not be
    /// patched. Patches that touch a locked field are denied.
    pub locked_fields: Vec<String>,
}

impl SoulDocument {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            version: 1,
            locked_fields: Vec::new(),
        }
    }

    pub fn with_locked_fields(
        mut self,
        fields: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.locked_fields = fields
            .into_iter()
            .map(|f| f.into().to_lowercase())
            .collect();
        self
    }
}

/// Result of validating a proposed SOUL.md patch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulPatchResult {
    /// Whether the patch is allowed to proceed (possibly after approval).
    pub allowed: bool,
    /// Human-readable reason for the decision.
    pub reason: String,
    /// If true, an operator must approve before the patch is applied.
    pub requires_approval: bool,
}

impl SoulPatchResult {
    pub fn allowed(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
            requires_approval: false,
        }
    }

    pub fn requires_approval(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
            requires_approval: true,
        }
    }

    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
            requires_approval: false,
        }
    }
}

/// Mode for a soul patch operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoulPatchMode {
    /// Append text to the end of the document.
    Append,
    /// Replace the entire document content.
    Replace,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soul_document_default_version_is_1() {
        let doc = SoulDocument::new("# Cairn\n\nHello world.");
        assert_eq!(doc.version, 1);
        assert!(doc.locked_fields.is_empty());
    }

    #[test]
    fn soul_document_locked_fields_lowercased() {
        let doc = SoulDocument::new("").with_locked_fields(["Values", "VOICE"]);
        assert!(doc.locked_fields.contains(&"values".to_owned()));
        assert!(doc.locked_fields.contains(&"voice".to_owned()));
    }

    #[test]
    fn soul_patch_result_constructors() {
        let a = SoulPatchResult::allowed("ok");
        assert!(a.allowed);
        assert!(!a.requires_approval);

        let r = SoulPatchResult::requires_approval("needs review");
        assert!(r.allowed);
        assert!(r.requires_approval);

        let d = SoulPatchResult::denied("locked");
        assert!(!d.allowed);
        assert!(!d.requires_approval);
    }
}
