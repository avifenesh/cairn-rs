use cairn_domain::ids::SourceId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// A digest aggregates recent signals for operator review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    pub project: ProjectKey,
    pub source_ids: Vec<SourceId>,
    pub item_count: u32,
    pub summary: Option<String>,
}

/// Seam for digest generation. Implementors aggregate signals into digests.
pub trait DigestGenerator {
    type Error;

    fn generate(&self, project: &ProjectKey) -> Result<Digest, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::SourceId;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn digest_construction() {
        let digest = Digest {
            project: ProjectKey::new("t", "w", "p"),
            source_ids: vec![SourceId::new("src_1"), SourceId::new("src_2")],
            item_count: 12,
            summary: Some("12 new items from 2 sources".to_owned()),
        };
        assert_eq!(digest.source_ids.len(), 2);
        assert_eq!(digest.item_count, 12);
    }
}
