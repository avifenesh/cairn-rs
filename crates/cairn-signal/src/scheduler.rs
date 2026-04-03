use cairn_domain::ids::SourceId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Schedule for polling a signal source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollSchedule {
    pub source_id: SourceId,
    pub project: ProjectKey,
    pub interval_secs: u64,
    pub enabled: bool,
}

/// Seam for signal scheduling. Implementors manage poll timing.
pub trait SignalScheduler {
    type Error;

    fn register(&mut self, schedule: PollSchedule) -> Result<(), Self::Error>;
    fn unregister(&mut self, source_id: &SourceId) -> Result<(), Self::Error>;
    fn list_schedules(&self) -> Vec<PollSchedule>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::SourceId;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn poll_schedule_construction() {
        let schedule = PollSchedule {
            source_id: SourceId::new("rss_feed_1"),
            project: ProjectKey::new("t", "w", "p"),
            interval_secs: 300,
            enabled: true,
        };
        assert!(schedule.enabled);
        assert_eq!(schedule.interval_secs, 300);
    }
}
