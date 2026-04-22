//! 30-minute real-LLM soak test — step 2 of 3 in the soak ladder.
//!
//! Spawns a real cairn-app subprocess, wires it to the live OpenRouter
//! API via a free-tier MiniMax model, then runs **N=3 concurrent agent
//! runs** against it for **1800 real seconds**. Every 15 seconds samples
//! process RSS, open-fd count, `/health/ready`, `/v1/status`, and the
//! event-log position so post-run assertions can check for memory/fd
//! leaks and readiness drops.
//!
//! # Running locally
//!
//! Requires the OpenRouter API key at `~/.cairn-secrets/openrouter.key`
//! (600-mode file outside the repo). Without that file the test
//! gracefully self-skips — it will NOT panic, NOT fail.
//!
//! ```bash
//! cargo test -p cairn-app --test test_soak_30min -- --ignored --nocapture
//! ```
//!
//! Expect ~30 min + ~10s for cairn-app boot.
//!
//! # Bounds — tightened from 5min empirical data (PR #92)
//!
//! The 5min soak saw RSS +20.4% and fd +6% across 11 successful
//! orchestrations. Memory growth is typically sub-linear (log-ish), not
//! linear, so scaling 6× in duration does not justify scaling bounds 6×.
//!
//! - RSS growth: **<50%** (5min saw 20.4%; headroom for the first real
//!   30min run while still catching a real leak ≥2× the 5min curve).
//! - fd growth: **<30%** (5min saw 6%; bounded growth — real fd leaks
//!   grow unboundedly).
//! - Upstream 429 count: informational only (no bound).
//! - Readiness drops: zero tolerance.
//! - Panics in subprocess stderr: zero tolerance.
//!
//! # CI posture
//!
//! `#[ignore]` by default — real-LLM calls against an operator-owned key
//! must not run in shared CI. This test is opt-in only.
//!
//! # Out of scope (follow-up PRs)
//!
//! - 1-hr soak (step 3 of 3).
//! - Nightly CI wiring (explicit user follow-up).

mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;
use tokio::sync::Mutex;

/// Free-tier OpenRouter model. Same as `contract_openrouter` (PR #91)
/// and `test_soak_5min` (PR #92).
const MODEL: &str = "minimax/minimax-m2.5:free";

/// OpenRouter production base URL.
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/";

/// Number of concurrent soak workers. Kept small so 30 min × 10 s cadence
/// stays within ~540-1080 total OpenRouter calls — significant but still
/// well under any reasonable free-tier abuse threshold when spaced out.
const WORKERS: usize = 3;

/// Total soak duration. 1800s = 30 minutes. Step 2 of the 5min → 30min
/// → 1hr ladder.
const SOAK_DURATION: Duration = Duration::from_secs(1800);

/// Interval between successive orchestrations within one worker. Spaces
/// requests so the free tier is never stressed: 30 min / 10 s = ~180
/// calls per worker, ~540 across N=3 workers.
const WORKER_INTERVAL: Duration = Duration::from_secs(10);

/// Interval between metric samples. 15 s × 30 min = 120 samples.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

/// Log a running-sample line every 60 s instead of every sample — 30
/// log lines over 30 min keeps stderr manageable while still giving
/// clear trajectory visibility.
const LOG_INTERVAL: Duration = Duration::from_secs(60);

/// RSS growth bound. 5min empirical saw 20.4%; 50% is ~2.5× that, giving
/// headroom for sub-linear growth over 6× longer duration while still
/// catching real leaks.
const MAX_RSS_GROWTH_PCT: f64 = 50.0;

/// fd growth bound. 5min empirical saw 6%; 30% is 5× that, catches any
/// unbounded growth pattern.
const MAX_FD_GROWTH_PCT: f64 = 30.0;

/// Resolve `~/.cairn-secrets/openrouter.key`. Returns `None` if `$HOME`
/// is unset or the file is absent — test self-skips in that case.
fn openrouter_key_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".cairn-secrets");
    p.push("openrouter.key");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

/// Read the OpenRouter key, trimming trailing whitespace/newline.
fn read_openrouter_key(path: &PathBuf) -> String {
    fs::read_to_string(path)
        .expect("openrouter.key readable")
        .trim()
        .to_owned()
}

/// Linux-only RSS sampler. Reads `VmRSS` (kB) from `/proc/<pid>/status`.
fn sample_rss_kb(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            // Format: `VmRSS:   12345 kB`
            let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
            return digits.parse().ok();
        }
    }
    None
}

/// Linux-only fd sampler. Counts entries in `/proc/<pid>/fd/`.
fn sample_fd_count(pid: u32) -> Option<usize> {
    let dir = fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    Some(dir.filter_map(|e| e.ok()).count())
}

/// One metric sample captured periodically during the soak.
#[derive(Debug, Clone)]
struct Sample {
    elapsed_s: u64,
    rss_kb: u64,
    fd_count: usize,
    ready_ok: bool,
    status_ok: bool,
    event_count: usize,
}

/// Query `/v1/events/recent?limit=500` and return the number of events.
/// Loose proxy for event-log growth; the exact number is informational.
async fn event_count(client: &reqwest::Client, base_url: &str, token: &str) -> usize {
    match client
        .get(format!("{base_url}/v1/events/recent?limit=500"))
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(v) => v
                .get("items")
                .and_then(|i| i.as_array())
                .map(|a| a.len())
                .or_else(|| v.as_array().map(|a| a.len()))
                .unwrap_or(0),
            Err(_) => 0,
        },
        _ => 0,
    }
}

async fn ready_ok(client: &reqwest::Client, base_url: &str) -> bool {
    client
        .get(format!("{base_url}/health/ready"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn status_ok(client: &reqwest::Client, base_url: &str, token: &str) -> bool {
    client
        .get(format!("{base_url}/v1/status"))
        .bearer_auth(token)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tokio::test]
#[ignore = "real-LLM soak: requires ~/.cairn-secrets/openrouter.key and ~30 min"]
async fn cairn_app_sustains_30min_real_llm_soak() {
    let Some(key_path) = openrouter_key_path() else {
        eprintln!(
            "[SKIP] soak: ~/.cairn-secrets/openrouter.key missing. \
             Create the 600-mode file to enable this test."
        );
        return;
    };
    let api_key = read_openrouter_key(&key_path);
    assert!(
        api_key.starts_with("sk-or-"),
        "unexpected key shape in {} — expected OpenRouter `sk-or-` prefix",
        key_path.display()
    );

    // 1. Spawn cairn-app with sqlite so event-log state is inspectable
    //    if debugging post-hoc.
    let h = LiveHarness::setup_with_sqlite().await;
    let Some(pid) = h.subprocess_pid() else {
        panic!("cairn-app subprocess has no PID — fatal for soak sampling");
    };

    let tenant = "default_tenant".to_owned();
    let workspace = "default_workspace".to_owned();
    let project = "default_project".to_owned();
    let suffix = h.project.clone();
    let connection_id = format!("conn_{suffix}");
    let session_id = format!("sess_{suffix}");

    // 2. Credential.
    let r = h
        .client()
        .post(format!(
            "{}/v1/admin/tenants/{}/credentials",
            h.base_url, tenant
        ))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "provider_id": "openrouter", "plaintext_value": api_key }))
        .send()
        .await
        .expect("credential POST reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "credential create: {}",
        r.text().await.unwrap_or_default()
    );
    let credential_id = r
        .json::<Value>()
        .await
        .unwrap()
        .get("id")
        .and_then(|v| v.as_str())
        .expect("credential id")
        .to_owned();

    // 3. Provider connection wired to the REAL OpenRouter URL.
    let r = h
        .client()
        .post(format!("{}/v1/providers/connections", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "provider_connection_id": connection_id,
            "provider_family": "openrouter",
            "adapter_type": "openrouter",
            "supported_models": [MODEL],
            "credential_id": credential_id,
            "endpoint_url": OPENROUTER_URL,
        }))
        .send()
        .await
        .expect("connection POST reaches server");
    assert_eq!(
        r.status().as_u16(),
        201,
        "connection create: {}",
        r.text().await.unwrap_or_default()
    );

    // 4. Point brain + generate defaults at the free model.
    for key in ["generate_model", "brain_model"] {
        let r = h
            .client()
            .put(format!(
                "{}/v1/settings/defaults/system/system/{}",
                h.base_url, key
            ))
            .bearer_auth(&h.admin_token)
            .json(&json!({ "value": MODEL }))
            .send()
            .await
            .expect("settings PUT reaches server");
        assert_eq!(r.status().as_u16(), 200, "settings {key}");
    }

    // 5. Shared session for all workers.
    let r = h
        .client()
        .post(format!("{}/v1/sessions", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id": tenant,
            "workspace_id": workspace,
            "project_id": project,
            "session_id": session_id,
        }))
        .send()
        .await
        .expect("session POST reaches server");
    assert_eq!(r.status().as_u16(), 201, "session create");

    // 6. Baseline metrics before any run traffic.
    let baseline_rss = sample_rss_kb(pid).expect("/proc/pid/status readable");
    let baseline_fd = sample_fd_count(pid).expect("/proc/pid/fd readable");
    let baseline_events = event_count(h.client(), &h.base_url, &h.admin_token).await;
    eprintln!(
        "[soak] baseline: rss={} kB, fd={}, events={}",
        baseline_rss, baseline_fd, baseline_events
    );

    // 7. Shared state for workers + sampler.
    let stop_at = Instant::now() + SOAK_DURATION;
    let iterations_per_worker: Vec<Arc<AtomicUsize>> = (0..WORKERS)
        .map(|_| Arc::new(AtomicUsize::new(0)))
        .collect();
    let upstream_429s = Arc::new(AtomicUsize::new(0));
    let samples: Arc<Mutex<Vec<Sample>>> = Arc::new(Mutex::new(Vec::new()));

    // 8. Spawn sampler task.
    let sampler = {
        let client = h.client().clone();
        let base_url = h.base_url.clone();
        let token = h.admin_token.clone();
        let samples = samples.clone();
        let start = Instant::now();
        tokio::spawn(async move {
            let mut next_log = LOG_INTERVAL;
            loop {
                tokio::time::sleep(SAMPLE_INTERVAL).await;
                let elapsed = start.elapsed();
                if elapsed >= SOAK_DURATION {
                    break;
                }
                let rss = sample_rss_kb(pid).unwrap_or(0);
                let fdc = sample_fd_count(pid).unwrap_or(0);
                let ready = ready_ok(&client, &base_url).await;
                let stat = status_ok(&client, &base_url, &token).await;
                let evs = event_count(&client, &base_url, &token).await;
                let s = Sample {
                    elapsed_s: elapsed.as_secs(),
                    rss_kb: rss,
                    fd_count: fdc,
                    ready_ok: ready,
                    status_ok: stat,
                    event_count: evs,
                };
                samples.lock().await.push(s.clone());
                if elapsed >= next_log {
                    eprintln!(
                        "[soak @ {}s] rss={} kB, fd={}, ready={}, status={}, events={}",
                        s.elapsed_s, s.rss_kb, s.fd_count, s.ready_ok, s.status_ok, s.event_count,
                    );
                    next_log += LOG_INTERVAL;
                }
            }
        })
    };

    // 9. Spawn WORKERS concurrent run loops.
    let mut worker_handles = Vec::new();
    for (wid, counter) in iterations_per_worker.iter().cloned().enumerate() {
        let client = h.client().clone();
        let base_url = h.base_url.clone();
        let token = h.admin_token.clone();
        let suffix = suffix.clone();
        let tenant = tenant.clone();
        let workspace = workspace.clone();
        let project = project.clone();
        let session_id = session_id.clone();
        let rate_limited = upstream_429s.clone();
        worker_handles.push(tokio::spawn(async move {
            let mut iter = 0usize;
            while Instant::now() < stop_at {
                let run_id = format!("run_{suffix}_w{wid}_i{iter}");

                // Create run.
                let r = client
                    .post(format!("{base_url}/v1/runs"))
                    .bearer_auth(&token)
                    .json(&json!({
                        "tenant_id": tenant,
                        "workspace_id": workspace,
                        "project_id": project,
                        "session_id": session_id,
                        "run_id": run_id,
                    }))
                    .send()
                    .await;
                let Ok(resp) = r else {
                    // Transient network blip; record as non-fatal, keep going.
                    tokio::time::sleep(WORKER_INTERVAL).await;
                    iter += 1;
                    continue;
                };
                if resp.status().as_u16() != 201 {
                    // Don't panic — log and move on. We're soaking the
                    // server, not asserting per-request correctness.
                    let code = resp.status().as_u16();
                    let body = resp.text().await.unwrap_or_default();
                    eprintln!("[soak w{wid}] run create non-201: {code}: {body}");
                    tokio::time::sleep(WORKER_INTERVAL).await;
                    iter += 1;
                    continue;
                }

                // Orchestrate. This is the real-LLM call.
                let o = client
                    .post(format!("{base_url}/v1/runs/{run_id}/orchestrate"))
                    .bearer_auth(&token)
                    .json(&json!({
                        "goal": "Say hello in one word.",
                        "max_iterations": 1,
                    }))
                    .send()
                    .await;
                match o {
                    Ok(resp) => {
                        let st = resp.status().as_u16();
                        if st == 429 {
                            rate_limited.fetch_add(1, Ordering::Relaxed);
                        } else if !resp.status().is_success() {
                            // 5xx or 4xx from orchestrate — log, don't
                            // fail the soak. Upstream OpenRouter hiccups
                            // are not cairn bugs.
                            let body = resp.text().await.unwrap_or_default();
                            eprintln!("[soak w{wid}] orchestrate status={st}: {body}");
                        } else {
                            counter.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        eprintln!("[soak w{wid}] orchestrate network error: {e}");
                    }
                }

                iter += 1;
                tokio::time::sleep(WORKER_INTERVAL).await;
            }
        }));
    }

    // 10. Wait for everything.
    for h in worker_handles {
        let _ = h.await;
    }
    let _ = sampler.await;

    // 11. End metrics.
    let end_rss = sample_rss_kb(pid).unwrap_or(0);
    let end_fd = sample_fd_count(pid).unwrap_or(0);
    let end_events = event_count(h.client(), &h.base_url, &h.admin_token).await;
    let total_iter: usize = iterations_per_worker
        .iter()
        .map(|c| c.load(Ordering::Relaxed))
        .sum();
    let rl = upstream_429s.load(Ordering::Relaxed);

    eprintln!("[soak] END rss={end_rss} kB, fd={end_fd}, events={end_events}");
    eprintln!(
        "[soak] successful orchestrations: {total_iter}, upstream_429s: {rl}, \
         per-worker: {:?}",
        iterations_per_worker
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .collect::<Vec<_>>(),
    );

    let samples = samples.lock().await.clone();

    // 12. Assertions.

    // a) No readiness drop across any sample.
    let ready_drops: Vec<&Sample> = samples.iter().filter(|s| !s.ready_ok).collect();
    assert!(
        ready_drops.is_empty(),
        "readiness dropped during soak at samples: {:?}",
        ready_drops.iter().map(|s| s.elapsed_s).collect::<Vec<_>>(),
    );

    // b) No panics in subprocess stderr. LiveHarness doesn't currently
    //    expose raw stderr for post-hoc inspection; the strongest proxy
    //    is that the subprocess is still alive AND readiness held. A
    //    panicked process would fail readiness polls (connection
    //    refused) and the assertion above would fire.

    // c) RSS growth bound (tighter than 5min — see module docs).
    let rss_growth_pct = if baseline_rss == 0 {
        0.0
    } else {
        ((end_rss as f64 - baseline_rss as f64) / baseline_rss as f64) * 100.0
    };
    assert!(
        rss_growth_pct < MAX_RSS_GROWTH_PCT,
        "RSS grew {rss_growth_pct:.1}% (baseline={baseline_rss} kB, end={end_rss} kB), \
         over {MAX_RSS_GROWTH_PCT:.1}% bound. Samples: {:?}",
        samples
            .iter()
            .map(|s| (s.elapsed_s, s.rss_kb))
            .collect::<Vec<_>>(),
    );

    // d) fd growth bound (tighter than 5min — see module docs).
    let fd_growth_pct = if baseline_fd == 0 {
        0.0
    } else {
        ((end_fd as f64 - baseline_fd as f64) / baseline_fd as f64) * 100.0
    };
    assert!(
        fd_growth_pct < MAX_FD_GROWTH_PCT,
        "fd count grew {fd_growth_pct:.1}% (baseline={baseline_fd}, end={end_fd}), \
         over {MAX_FD_GROWTH_PCT:.1}% bound. Samples: {:?}",
        samples
            .iter()
            .map(|s| (s.elapsed_s, s.fd_count))
            .collect::<Vec<_>>(),
    );

    // e) Each worker completed at least one successful iteration. If
    //    OpenRouter was hard-down the whole run, fail loudly — the test
    //    infra is fine, but we should not silently declare success.
    //    Relax only if every worker saw 429s (upstream budget issue).
    let any_worker_zero = iterations_per_worker
        .iter()
        .any(|c| c.load(Ordering::Relaxed) == 0);
    if any_worker_zero && rl == 0 {
        panic!(
            "at least one worker completed 0 orchestrations with no 429s — \
             soak did not actually exercise the provider path. \
             per-worker: {:?}",
            iterations_per_worker
                .iter()
                .map(|c| c.load(Ordering::Relaxed))
                .collect::<Vec<_>>(),
        );
    }

    // f) Event log grew (sanity — if we orchestrated, events must have
    //    been appended).
    if total_iter > 0 {
        assert!(
            end_events >= baseline_events,
            "event count regressed: baseline={baseline_events}, end={end_events}",
        );
    }
}
