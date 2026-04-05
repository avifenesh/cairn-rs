//! RFC 005 SLA breach detection end-to-end integration test.
//!
//! Validates the run SLA lifecycle:
//!   (1) set an SLA target on a run
//!   (2) breach the SLA by waiting past the target duration
//!   (3) verify RunSlaBreached event emitted with correct fields
//!   (4) verify the run appears in list_breached_by_tenant
//!   (5) run within SLA does not trigger a breach
//!   (6) check_and_breach is idempotent (second call returns false)
//!   (7) check_sla reports percent_used and on_track correctly

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey,
    RunCreated, RunId, RuntimeEvent, SessionId, TenantId,
};
use cairn_runtime::RunSlaService;
use cairn_runtime::services::RunSlaServiceImpl;
use cairn_store::{EventLog, InMemoryStore};
use tokio::time::{sleep, Duration};

fn project() -> ProjectKey { ProjectKey::new("t_sla", "ws_sla", "proj_sla") }
fn tenant()  -> TenantId   { TenantId::new("t_sla") }

fn setup() -> (Arc<InMemoryStore>, RunSlaServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let svc   = RunSlaServiceImpl::new(store.clone());
    (store, svc)
}

async fn seed_run(store: &Arc<InMemoryStore>, run_id: &str) {
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_{run_id}")),
            EventSource::Runtime,
            RuntimeEvent::RunCreated(RunCreated {
                project:          project(),
                session_id:       SessionId::new("sess_sla"),
                run_id:           RunId::new(run_id),
                parent_run_id:    None,
                prompt_release_id: None,
                agent_role_id:    None,
            }),
        )])
        .await
        .unwrap();
}

// ── (1) Set SLA target — config stored ───────────────────────────────────

#[tokio::test]
async fn set_sla_stores_config_with_correct_fields() {
    let (_, svc) = setup();
    let run_id = RunId::new("run_sla_set");

    let config = svc
        .set_sla(run_id.clone(), tenant(), 60_000, 80)
        .await
        .unwrap();

    assert_eq!(config.run_id, run_id);
    assert_eq!(config.tenant_id, tenant());
    assert_eq!(config.target_completion_ms, 60_000);

    // get_sla retrieves the same config.
    let fetched = svc.get_sla(&run_id).await.unwrap().unwrap();
    assert_eq!(fetched.target_completion_ms, 60_000);
}

// ── (2)+(3) Breach the SLA, verify RunSlaBreached event ──────────────────

#[tokio::test]
async fn sla_breach_emits_run_sla_breached_event_with_correct_fields() {
    let (store, svc) = setup();
    let run_id = RunId::new("run_sla_breach");
    seed_run(&store, "run_sla_breach").await;

    // Set a very short SLA target (50ms).
    svc.set_sla(run_id.clone(), tenant(), 50, 80).await.unwrap();

    // Wait longer than the target.
    sleep(Duration::from_millis(100)).await;

    // Trigger breach detection.
    let breached = svc.check_and_breach(&run_id).await.unwrap();
    assert!(breached, "check_and_breach must return true when SLA is exceeded");

    // Verify RunSlaBreached event in the event log.
    let events = store.read_stream(None, 50).await.unwrap();
    let breach_event = events.iter().find_map(|e| {
        if let RuntimeEvent::RunSlaBreached(ev) = &e.envelope.payload {
            if ev.run_id == run_id { Some(ev.clone()) } else { None }
        } else {
            None
        }
    });

    let ev = breach_event.expect("RunSlaBreached event must be in the event log");
    assert_eq!(ev.run_id, run_id);
    assert_eq!(ev.tenant_id, tenant());
    assert_eq!(ev.target_ms, 50);
    assert!(ev.elapsed_ms >= 50, "elapsed_ms must be >= target_ms at breach");
    assert!(ev.breached_at_ms > 0, "breached_at_ms must be set");
}

// ── (4) Breached run appears in list_breached_by_tenant ──────────────────

#[tokio::test]
async fn breached_run_appears_in_list_breached_by_tenant() {
    let (store, svc) = setup();
    let run_id = RunId::new("run_list_breach");
    seed_run(&store, "run_list_breach").await;

    svc.set_sla(run_id.clone(), tenant(), 50, 80).await.unwrap();
    sleep(Duration::from_millis(100)).await;
    svc.check_and_breach(&run_id).await.unwrap();

    let breaches = svc.list_breached_by_tenant(&tenant()).await.unwrap();
    assert_eq!(breaches.len(), 1, "exactly one breach must be listed");

    let breach = &breaches[0];
    assert_eq!(breach.run_id, run_id);
    assert_eq!(breach.tenant_id, tenant());
    assert_eq!(breach.target_ms, 50);
    assert!(breach.elapsed_ms >= 50);
    assert!(breach.breached_at_ms > 0);
}

// ── (5) Run within SLA does not trigger breach ────────────────────────────

#[tokio::test]
async fn run_within_sla_does_not_trigger_breach() {
    let (store, svc) = setup();
    let run_id = RunId::new("run_on_track");
    seed_run(&store, "run_on_track").await;

    // Set a generous 60-second SLA — will not expire in this test.
    svc.set_sla(run_id.clone(), tenant(), 60_000, 80).await.unwrap();

    let breached = svc.check_and_breach(&run_id).await.unwrap();
    assert!(!breached, "run within SLA must not trigger a breach");

    // No RunSlaBreached event.
    let events = store.read_stream(None, 20).await.unwrap();
    let has_breach = events.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RunSlaBreached(ev) if ev.run_id == run_id)
    });
    assert!(!has_breach, "no RunSlaBreached event must be emitted for on-track run");

    // list_breached returns empty.
    let breaches = svc.list_breached_by_tenant(&tenant()).await.unwrap();
    assert!(breaches.is_empty(), "on-track run must not appear in breach list");
}

// ── (6) check_and_breach is idempotent ───────────────────────────────────

#[tokio::test]
async fn check_and_breach_is_idempotent() {
    let (store, svc) = setup();
    let run_id = RunId::new("run_idem");
    seed_run(&store, "run_idem").await;

    svc.set_sla(run_id.clone(), tenant(), 50, 80).await.unwrap();
    sleep(Duration::from_millis(100)).await;

    let first  = svc.check_and_breach(&run_id).await.unwrap();
    let second = svc.check_and_breach(&run_id).await.unwrap();
    let third  = svc.check_and_breach(&run_id).await.unwrap();

    assert!(first,  "first call must return true (breach emitted)");
    assert!(!second, "second call must return false (idempotent)");
    assert!(!third,  "third call must return false (idempotent)");

    // Only one RunSlaBreached event in the log.
    let events = store.read_stream(None, 50).await.unwrap();
    let breach_count = events
        .iter()
        .filter(|e| matches!(&e.envelope.payload, RuntimeEvent::RunSlaBreached(ev) if ev.run_id == run_id))
        .count();
    assert_eq!(breach_count, 1, "exactly one RunSlaBreached event must exist");
}

// ── (7) check_sla reports percent_used and on_track ──────────────────────

#[tokio::test]
async fn check_sla_reports_correct_status_fields() {
    let (store, svc) = setup();
    let run_id = RunId::new("run_status");
    seed_run(&store, "run_status").await;

    // Set 60-second SLA — still on-track immediately after creation.
    svc.set_sla(run_id.clone(), tenant(), 60_000, 80).await.unwrap();

    let status = svc.check_sla(&run_id).await.unwrap();
    assert!(status.on_track, "run must be on_track immediately after creation");
    assert_eq!(status.target_ms, 60_000);
    assert!(status.elapsed_ms < 60_000, "elapsed_ms must be less than target");
    assert!(status.percent_used < 100, "percent_used must be < 100 when on track");
}

#[tokio::test]
async fn check_sla_reports_over_100_percent_when_breached() {
    let (store, svc) = setup();
    let run_id = RunId::new("run_over_100");
    seed_run(&store, "run_over_100").await;

    svc.set_sla(run_id.clone(), tenant(), 50, 80).await.unwrap();
    sleep(Duration::from_millis(100)).await;

    let status = svc.check_sla(&run_id).await.unwrap();
    assert!(!status.on_track, "run must not be on_track after SLA expired");
    assert!(status.percent_used > 100, "percent_used must exceed 100 after SLA target elapsed");
    assert!(status.elapsed_ms >= status.target_ms);
}

// ── Multiple tenants isolated ─────────────────────────────────────────────

#[tokio::test]
async fn breaches_are_scoped_to_tenant() {
    let store  = Arc::new(InMemoryStore::new());
    let svc    = RunSlaServiceImpl::new(store.clone());
    let tenant_a = TenantId::new("sla_tenant_a");
    let tenant_b = TenantId::new("sla_tenant_b");
    let run_a  = RunId::new("run_breach_a");
    let run_b  = RunId::new("run_breach_b");

    for run_id in [run_a.as_str(), run_b.as_str()] {
        store.append(&[EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_{run_id}")),
            EventSource::Runtime,
            RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_iso"),
                run_id: RunId::new(run_id),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
        )]).await.unwrap();
    }

    svc.set_sla(run_a.clone(), tenant_a.clone(), 50, 80).await.unwrap();
    svc.set_sla(run_b.clone(), tenant_b.clone(), 50, 80).await.unwrap();
    sleep(Duration::from_millis(100)).await;
    svc.check_and_breach(&run_a).await.unwrap();
    svc.check_and_breach(&run_b).await.unwrap();

    let a_breaches = svc.list_breached_by_tenant(&tenant_a).await.unwrap();
    let b_breaches = svc.list_breached_by_tenant(&tenant_b).await.unwrap();

    assert_eq!(a_breaches.len(), 1, "tenant_a must see only its own breach");
    assert_eq!(b_breaches.len(), 1, "tenant_b must see only its own breach");
    assert_eq!(a_breaches[0].run_id, run_a);
    assert_eq!(b_breaches[0].run_id, run_b);
}
