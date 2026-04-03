//! Signal ingestion, scheduling, and digest generation boundaries.

pub mod digests;
pub mod pollers;
pub mod scheduler;
pub mod webhooks;

pub use digests::{Digest, DigestGenerator};
pub use pollers::{PollResult, SignalSource, SourceKind, SourcePoller};
pub use scheduler::{PollSchedule, SignalScheduler};
pub use webhooks::{WebhookIngester, WebhookPayload, WebhookRegistration};

#[cfg(test)]
mod tests {
    use cairn_domain::ids::SourceId;
    use cairn_domain::tenancy::ProjectKey;

    use crate::pollers::{SignalSource, SourceKind};
    use crate::scheduler::PollSchedule;

    #[test]
    fn source_and_schedule_share_source_id() {
        let source_id = SourceId::new("src_1");
        let source = SignalSource {
            source_id: source_id.clone(),
            project: ProjectKey::new("t", "w", "p"),
            kind: SourceKind::Rss,
            name: "Feed".to_owned(),
        };
        let schedule = PollSchedule {
            source_id: source_id.clone(),
            project: ProjectKey::new("t", "w", "p"),
            interval_secs: 60,
            enabled: true,
        };
        assert_eq!(source.source_id, schedule.source_id);
    }
}
