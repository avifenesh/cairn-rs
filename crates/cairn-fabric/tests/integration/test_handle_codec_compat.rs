//! Handle codec v1 → v2 compatibility shell.
//!
//! FF 0.8 added a v2 wire format for `HandleOpaque` that prepends a
//! `BackendTag` byte (RFC-011 §4.1). The decoder accepts both v1 (pre-
//! 0.8) and v2 (0.8+) shapes. Cairn's migration from FF 0.3 to FF 0.9
//! crosses this boundary: event logs persisted under 0.3 contain v1-
//! shape handles that, on resume, must decode via the v2 compat path.
//!
//! This test covers that path end-to-end:
//!
//!   1. Mint a v1-shape handle for a known `LeaseFencingTriple`.
//!   2. Persist via the standard cairn event-log write path.
//!   3. Read back via the standard decode path.
//!   4. Assert all three fence fields round-trip identically.
//!
//! ## Why this test is `#[ignore]`
//!
//! Step 1 requires a public test fixture for minting v1-shape handle
//! bytes. FF 0.9 does not expose one — `HandleCodec::encode` always
//! writes v2, and `HandleOpaque` is opaque by contract. Reverse-
//! engineering the v1 layout to construct raw bytes here would couple
//! this test to internal codec details that FF 0.9's own compat-decoder
//! is the only supported consumer of.
//!
//! Filed upstream as
//! <https://github.com/avifenesh/FlowFabric/issues/323>
//! (`ff_core::handle_codec::v1_handle_for_tests` behind the
//! `test-fixtures` feature). Once that lands, drop the `#[ignore]`,
//! replace the placeholder with the real fixture call, and flesh out
//! steps 2-4.

/// Compat-path round-trip: v1-shape handle bytes decode to the same
/// `LeaseFencingTriple` the original encode used.
///
/// Ignored until FF#323 publishes the `v1_handle_for_tests` fixture.
/// Once unignored, this test must run against a real Valkey backend
/// (event log round-trip is not a pure unit test) so it lives in the
/// integration suite, not under `#[cfg(test)]` in a crate's src tree.
#[test]
#[ignore = "blocked on FF#323 (v1_handle_for_tests fixture)"]
fn v1_handle_compat_decode_round_trips() {
    // See module docstring. When FF#323 lands:
    //
    //   let bytes = ff_core::handle_codec::v1_handle_for_tests(
    //       ExecutionId::new(...),
    //       LeaseFencingTriple { ... },
    //       // other v1 fields
    //   );
    //   let handle = HandleCodec::decode(&bytes)
    //       .expect("v2 compat path must accept v1 bytes");
    //   assert_eq!(handle.exec_id(), ...);
    //   assert_eq!(handle.lease_fencing_triple(), ...);
    //
    // Until then, failing loudly on invocation is wrong — the test is
    // ignored. This assertion documents the expected shape via a
    // compile-checked pattern.
    panic!("FF#323 fixture required — remove #[ignore] when the fixture lands");
}
