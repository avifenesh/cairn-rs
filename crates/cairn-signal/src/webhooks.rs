use cairn_domain::ids::SourceId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Inbound webhook registration for push-based signal sources.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookRegistration {
    pub source_id: SourceId,
    pub project: ProjectKey,
    pub path: String,
    pub secret_ref: Option<String>,
}

/// Inbound webhook payload received from an external system.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub source_id: SourceId,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

/// Seam for webhook ingestion. Implementors validate and ingest inbound webhooks.
pub trait WebhookIngester {
    type Error;

    fn ingest(&self, payload: WebhookPayload) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::SourceId;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn webhook_registration_construction() {
        let reg = WebhookRegistration {
            source_id: SourceId::new("github_hook"),
            project: ProjectKey::new("t", "w", "p"),
            path: "/webhooks/github".to_owned(),
            secret_ref: Some("secret_github".to_owned()),
        };
        assert_eq!(reg.path, "/webhooks/github");
    }

    #[test]
    fn webhook_payload_construction() {
        let payload = WebhookPayload {
            source_id: SourceId::new("github_hook"),
            headers: vec![("x-hub-signature".to_owned(), "sha256=abc".to_owned())],
            body: serde_json::json!({"action": "opened"}),
        };
        assert_eq!(payload.headers.len(), 1);
    }
}
