use cairn_domain::ids::SourceId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Signal source kinds supported in v1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Rss,
    Api,
    Plugin,
}

/// A registered signal source that can be polled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalSource {
    pub source_id: SourceId,
    pub project: ProjectKey,
    pub kind: SourceKind,
    pub name: String,
}

/// Result of polling a signal source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollResult {
    pub source_id: SourceId,
    pub new_items: u32,
    pub cursor: Option<String>,
}

/// Seam for source polling. Implementors fetch new signals from a source.
pub trait SourcePoller {
    type Error;

    fn poll(&self, source: &SignalSource, cursor: Option<&str>) -> Result<PollResult, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::SourceId;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn signal_source_construction() {
        let source = SignalSource {
            source_id: SourceId::new("src_1"),
            project: ProjectKey::new("t", "w", "p"),
            kind: SourceKind::Rss,
            name: "Tech News".to_owned(),
        };
        assert_eq!(source.kind, SourceKind::Rss);
    }

    #[test]
    fn poll_result_tracks_new_items() {
        let result = PollResult {
            source_id: SourceId::new("src_1"),
            new_items: 5,
            cursor: Some("cursor_abc".to_owned()),
        };
        assert_eq!(result.new_items, 5);
        assert!(result.cursor.is_some());
    }
}
