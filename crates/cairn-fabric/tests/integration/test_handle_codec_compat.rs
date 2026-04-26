//! Handle codec v1 → v2 compatibility round-trip (FF#323).
//!
//! FF 0.8 added a v2 wire format for `HandleOpaque` that prepends a
//! `BackendTag` byte (RFC-011 §4.1). The decoder accepts both v1 (pre-
//! 0.8) and v2 (0.8+) shapes. Cairn's migration from FF 0.3 to FF 0.10
//! crosses this boundary: event logs persisted under 0.3 contain v1-
//! shape handles that, on resume, must decode cleanly via the v2 compat
//! path.
//!
//! FF 0.10 ships `ff_core::handle_codec::v1_handle_for_tests` behind
//! the `test-fixtures` feature (FF#323). Cairn's dev-deps pull that
//! feature on `ff-core`, so this test synthesises a real v1-shape byte
//! buffer and drives it through the production decode path.

use ff_core::backend::{BackendTag, HandleOpaque};
use ff_core::handle_codec::{self, HandlePayload};
use ff_core::types::{
    AttemptId, AttemptIndex, ExecutionId, LaneId, LeaseEpoch, LeaseId, WorkerInstanceId,
};

/// Compat-path round-trip: v1-shape bytes decode to the same
/// [`HandlePayload`] fields the v1 encoder recorded, and the decoder
/// infers [`BackendTag::Valkey`] per the RFC-011 compat rule.
#[test]
fn v1_handle_compat_decode_round_trips() {
    // Deterministic fixture. Use a parseable ExecutionId so the
    // decoder's ExecutionId::parse step accepts it — the v1 fixture
    // serialises the ExecutionId's string form.
    let execution_id = ExecutionId::parse("{fp:0}:11111111-1111-4111-8111-111111111111")
        .expect("execution id fixture parses");
    let attempt_id = AttemptId::parse("22222222-2222-4222-8222-222222222222")
        .expect("attempt id fixture parses");
    let lease_id =
        LeaseId::parse("33333333-3333-4333-8333-333333333333").expect("lease id fixture parses");

    let payload = HandlePayload::new(
        execution_id.clone(),
        AttemptIndex::new(7),
        attempt_id.clone(),
        lease_id.clone(),
        LeaseEpoch::new(3),
        30_000,
        LaneId::new("cairn"),
        WorkerInstanceId::new("instance-a"),
    );

    // Synthesise the v1 byte buffer via FF's test-fixture. Wrap into a
    // HandleOpaque the production decoder consumes.
    let v1_bytes = handle_codec::v1_handle_for_tests(&payload);
    let opaque = HandleOpaque::new(v1_bytes.into_boxed_slice());

    // Decode via the v2-era decoder's compat path.
    let decoded = handle_codec::decode(&opaque)
        .expect("v1 compat decode path must accept v1_handle_for_tests output");

    // v1 inferences: BackendTag::Valkey (RFC-011 §4.1 — pre-Wave-1c
    // buffers are Valkey-only).
    assert_eq!(decoded.tag, BackendTag::Valkey, "v1 compat tag is Valkey");

    // Every payload field must round-trip byte-identical.
    assert_eq!(decoded.payload.execution_id, execution_id);
    assert_eq!(decoded.payload.attempt_index, AttemptIndex::new(7));
    assert_eq!(decoded.payload.attempt_id, attempt_id);
    assert_eq!(decoded.payload.lease_id, lease_id);
    assert_eq!(decoded.payload.lease_epoch, LeaseEpoch::new(3));
    assert_eq!(decoded.payload.lease_ttl_ms, 30_000);
    assert_eq!(decoded.payload.lane_id.as_str(), "cairn");
    assert_eq!(decoded.payload.worker_instance_id.as_str(), "instance-a");
}
