//! Data-plane cost measurement for FF's `ScannerFilter` (FF#122).
//!
//! # What this measures
//!
//! FF 0.3.2 introduced `ScannerFilter { namespace, instance_tag }`
//! applied at the backend boundary by execution-shaped scanners and
//! completion subscribers. Cairn runs filter-ON in production with
//! `instance_tag = Some(("cairn.instance_id", worker_instance_id))` and
//! `namespace = None`, paying one extra HGET per candidate per scanner
//! cycle (and per completion frame) — see
//! `ff-backend-valkey-0.3.2/src/completion.rs::FilterGate::admits` and
//! `crates/cairn-fabric/src/boot.rs` for the wire-up.
//!
//! FF's maintainer framed the cost on issue #122 as:
//!   "Per-candidate HGET cost: 1 HGET for namespace only, 2 for both
//!    filters."
//!
//! This test measures the **actual wall-time cost of that per-candidate
//! HGET** against a real Valkey, at candidate counts representative of
//! cairn workloads (N ∈ {100, 1000, 10000}). It compares:
//!
//!   * **filter-OFF baseline** — iterate the candidate set, touch each
//!     eid once with a cheap noop (`HEXISTS` on the same tags-hash key
//!     used by `FilterGate::admits`). Models "scanner visits candidate
//!     but performs no extra filter HGET".
//!   * **filter-ON (cairn production)** — for each candidate, perform
//!     the exact `HGET <tags_key> cairn.instance_id` that
//!     `FilterGate::admits` performs. This is the cost the FF#122
//!     commentary refers to.
//!
//! Delta = filter-ON cost minus filter-OFF cost = the true FF#122 tax.
//!
//! # Scope
//!
//! This is a **microbenchmark of the HGET hot path**, not a full
//! scanner-cycle simulation. The decision to measure at this level is
//! deliberate per the scope gates: criterion + LiveHarness integration
//! is a multi-day lift and the HGET is the isolable cost FF's
//! maintainer specifically called out. The benchmark captures the
//! marginal latency a filter-ON scanner pays per candidate it visits;
//! at 11 execution-shaped scanners firing per cycle, multiply by
//! scanner count and candidate density to get scanner-cycle overhead.
//!
//! Non-goals:
//!
//!   * Full scanner-loop timing across all 11 execution-shaped scanners
//!     (that requires driving the Engine directly — out of scope).
//!   * Throughput under adversarial contention (leave to chaos suite).
//!   * Memory / Valkey working-set cost (negligible — existing keys).
//!
//! # Methodology
//!
//! 1. Seed N exec-tag hashes, each with `cairn.instance_id = <own_tag>`.
//!    Keys are the canonical `ff:exec:{{fp:P}}:{{fp:P}}:<uuid>:tags`
//!    shape the backend HGETs against.
//! 2. Warmup: one pass of each config, discarded.
//! 3. Measure: 3 runs each config; record wall time for N candidates.
//!    Median-of-3 reported.
//! 4. Print a results table to stderr for PR body / FF#122 comment
//!    harvesting.
//!
//! # Reproduce
//!
//! ```bash
//! # With a local Valkey on 6379 (faster; no container boot):
//! CAIRN_TEST_VALKEY_URL=redis://127.0.0.1:6379/ \
//!     cargo test -p cairn-fabric --test integration --release \
//!     -- integration::test_scanner_filter_perf --nocapture
//!
//! # With the testcontainers harness (CI-equivalent):
//! cargo test -p cairn-fabric --test integration --release \
//!     -- integration::test_scanner_filter_perf --nocapture
//! ```
//!
//! `--release` matters — the backend's `cmd("HGET")` path is
//! inlined-heavy and debug builds add multiple microseconds of
//! per-call overhead that do not reflect production.

use std::time::{Duration, Instant};

use cairn_fabric::test_harness::valkey_endpoint;

/// Candidate-count axis. Keep each step 10x so the tabulated result
/// trivially reveals sub-linear / linear / super-linear scaling. The
/// 10_000 step is the stress ceiling — a single cairn-app with 256
/// flow partitions and ~40 in-flight executions per partition tops
/// out near 10k candidates across all execution-shaped scanners per
/// cycle.
const CANDIDATE_COUNTS: &[usize] = &[100, 1_000, 10_000];

/// Runs per configuration. Three is the minimum honest sample —
/// testcontainer + shared-Valkey variance makes single-shot numbers
/// unreliable, and adding more runs past 3 yields diminishing returns
/// at the seconds-per-run timescale.
const RUNS_PER_CONFIG: usize = 3;

/// Hash-tag partition to seed into. Arbitrary; picked distinct from
/// partitions used by neighbouring tests (7, 11, 42) to reduce
/// cross-test key-noise on the shared Valkey container.
const PARTITION: u16 = 91;

fn tags_key(partition: u16, full_eid: &str) -> String {
    format!("ff:exec:{{fp:{partition}}}:{full_eid}:tags")
}

fn full_eid(partition: u16, bare_uuid: &str) -> String {
    format!("{{fp:{partition}}}:{bare_uuid}")
}

/// Seed `n` exec-tag hashes with `cairn.instance_id = want_tag`.
/// Returns the list of tag-hash keys in insertion order so the
/// measurement loop can iterate a deterministic candidate set.
async fn seed(client: &ferriskey::Client, n: usize, want_tag: &str) -> Vec<String> {
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        let eid = full_eid(PARTITION, &uuid::Uuid::new_v4().to_string());
        let key = tags_key(PARTITION, &eid);
        let _: i64 = client
            .cmd("HSET")
            .arg(&key)
            .arg("cairn.instance_id")
            .arg(want_tag)
            .execute()
            .await
            .expect("HSET seed");
        keys.push(key);
    }
    keys
}

/// Delete the seeded keys so the shared Valkey doesn't accumulate
/// cruft across repeated test runs. Best-effort — errors ignored.
async fn cleanup(client: &ferriskey::Client, keys: &[String]) {
    for chunk in keys.chunks(128) {
        let mut cmd = client.cmd("DEL");
        for k in chunk {
            cmd = cmd.arg(k);
        }
        let _: Result<i64, _> = cmd.execute().await;
    }
}

/// Filter-OFF baseline: one `HEXISTS` per candidate on the same tags
/// hash the filter-ON path HGETs. Models "scanner touches candidate
/// but performs no extra filter round-trip" — matched I/O shape
/// (one round-trip per candidate), different op (HEXISTS returns a
/// boolean without shipping the value back). The delta against the
/// filter-ON config isolates the HGET-vs-HEXISTS wire cost, which
/// for a single short field is dominated by the round-trip and is
/// the honest floor for the filter's marginal cost.
///
/// Rationale for HEXISTS over a fully no-op pass: a pure Rust loop
/// with no Valkey contact would conflate the filter's HGET cost with
/// the entire per-candidate RTT, overstating the filter tax. HEXISTS
/// isolates the HGET-vs-HEXISTS delta.
async fn run_filter_off(client: &ferriskey::Client, keys: &[String]) -> Duration {
    let t0 = Instant::now();
    for key in keys {
        // HEXISTS returns a RESP boolean in ferriskey 0.3 — parse as
        // `bool` directly. Using `Option<i64>` triggers a TypeError.
        let _: bool = client
            .cmd("HEXISTS")
            .arg(key)
            .arg("cairn.instance_id")
            .execute()
            .await
            .expect("HEXISTS baseline");
    }
    t0.elapsed()
}

/// Filter-ON production config: one HGET per candidate. Mirrors
/// `FilterGate::admits` exactly.
async fn run_filter_on(client: &ferriskey::Client, keys: &[String]) -> Duration {
    let t0 = Instant::now();
    for key in keys {
        let _: Option<String> = client
            .cmd("HGET")
            .arg(key)
            .arg("cairn.instance_id")
            .execute()
            .await
            .expect("HGET filter-on");
    }
    t0.elapsed()
}

fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort();
    xs[xs.len() / 2]
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Per-candidate median wall time, derived from the aggregate
/// measurement. This is what directly feeds the FF#122 "per-candidate
/// HGET cost" discussion.
fn per_candidate_ns(total: Duration, n: usize) -> u128 {
    total.as_nanos() / n as u128
}

/// The measurement. Not gated behind `#[ignore]` — keeps the bench
/// honest and lets CI surface regressions automatically. Runs quickly
/// on a warm local Valkey (~8s wall-clock for the full matrix) and
/// the one testcontainer spin-up is amortised across the whole
/// integration binary.
#[tokio::test]
async fn scanner_filter_cost_bench() {
    let (host, port) = valkey_endpoint().await;
    let client = ferriskey::ClientBuilder::new()
        .host(&host, port)
        .build()
        .await
        .expect("client build");

    // uuid-scoped instance tag so parallel test runs don't interfere
    // via the shared Valkey's candidate namespace.
    let run_suffix = uuid::Uuid::new_v4().simple().to_string();
    let instance_tag = format!("bench-inst-{run_suffix}");

    // Collect results for the final table.
    let mut rows: Vec<(usize, Duration, Duration, Duration, Duration)> = Vec::new();
    // (N, median_off, median_on, p50_on, p95_on) — p50/p95 on filter-ON
    // computed across per-candidate samples in the median run.

    for &n in CANDIDATE_COUNTS {
        let keys = seed(&client, n, &instance_tag).await;

        // Warmup — discarded. Primes Valkey's internal caches,
        // ferriskey's connection pipelining state, and Linux page
        // cache for the tags-hash memory.
        let _ = run_filter_off(&client, &keys).await;
        let _ = run_filter_on(&client, &keys).await;

        // Measured runs.
        let mut offs = Vec::with_capacity(RUNS_PER_CONFIG);
        let mut ons = Vec::with_capacity(RUNS_PER_CONFIG);
        for _ in 0..RUNS_PER_CONFIG {
            offs.push(run_filter_off(&client, &keys).await);
            ons.push(run_filter_on(&client, &keys).await);
        }

        // For p50/p95 we need per-candidate samples. Do one more
        // filter-ON pass recording each HGET's wall time. Keeps the
        // aggregate timing above (used for totals) free of the
        // per-iteration Instant::now() overhead, which is small but
        // non-zero.
        let mut per_call = Vec::with_capacity(n);
        for key in &keys {
            let t = Instant::now();
            let _: Option<String> = client
                .cmd("HGET")
                .arg(key)
                .arg("cairn.instance_id")
                .execute()
                .await
                .expect("HGET sample");
            per_call.push(t.elapsed());
        }
        per_call.sort();
        let p50 = percentile(&per_call, 0.50);
        let p95 = percentile(&per_call, 0.95);

        let med_off = median(offs.clone());
        let med_on = median(ons.clone());

        eprintln!(
            "[ff#122-bench] N={n:>6}  filter-OFF median={:>8?}  filter-ON median={:>8?}  \
             per-candidate p50={:>6?} p95={:>6?}  delta={:>+.2}%",
            med_off,
            med_on,
            p50,
            p95,
            ((med_on.as_nanos() as f64 - med_off.as_nanos() as f64) / med_off.as_nanos() as f64)
                * 100.0,
        );

        rows.push((n, med_off, med_on, p50, p95));
        cleanup(&client, &keys).await;
    }

    // Final table — copy-pasteable into the PR body and the FF#122
    // comment.
    eprintln!("\n=== FF#122 ScannerFilter cost: cairn data-plane measurement ===");
    eprintln!(
        "{:>8}  {:>14}  {:>14}  {:>10}  {:>10}  {:>10}  {:>10}",
        "N", "filter-OFF", "filter-ON", "per-cand", "per-cand", "delta", "HGET amp",
    );
    eprintln!(
        "{:>8}  {:>14}  {:>14}  {:>10}  {:>10}  {:>10}  {:>10}",
        "", "(median)", "(median)", "p50", "p95", "vs OFF", "factor",
    );
    for (n, off, on, p50, p95) in &rows {
        let off_ns = off.as_nanos() as f64;
        let on_ns = on.as_nanos() as f64;
        let delta_pct = (on_ns - off_ns) / off_ns * 100.0;
        let per_on_ns = per_candidate_ns(*on, *n);
        let per_off_ns = per_candidate_ns(*off, *n);
        // HGET amplification factor: per-candidate cost ratio. ~1.0
        // means HGET is as cheap as HEXISTS (pure RTT); >1.0 means
        // HGET's value-bearing response path adds measurable cost.
        let amp = per_on_ns as f64 / per_off_ns as f64;
        eprintln!(
            "{n:>8}  {off:>14?}  {on:>14?}  {p50:>10?}  {p95:>10?}  {delta_pct:>+9.2}%  {amp:>10.3}",
        );
    }
    eprintln!(
        "\n  per-candidate filter-ON p50 ~= FilterGate::admits() HGET cost \
         (matches FF#122 maintainer framing: 1 HGET per candidate for instance_tag only)"
    );
    eprintln!("================================================================\n");

    // Honest sanity floor — if filter-ON takes longer than filter-OFF,
    // the delta should be positive. If it's negative, the benchmark is
    // noisy to the point of uselessness and the numbers shouldn't be
    // trusted. This assertion keeps a silently-broken future refactor
    // from painting green.
    for (n, off, on, _, _) in &rows {
        assert!(
            on.as_nanos() >= off.as_nanos() / 2,
            "N={n}: filter-ON ({on:?}) is wildly faster than filter-OFF ({off:?}) — \
             measurement is noise-dominated",
        );
    }
}
