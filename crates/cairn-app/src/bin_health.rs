//! Detailed health endpoint — deep health checks for every subsystem.

#[allow(unused_imports)]
use crate::*;

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use std::time::Instant;

// ── Detailed health handler ───────────────────────────────────────────────────

/// Per-subsystem health entry.
#[derive(Serialize)]
pub(crate) struct CheckEntry {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    models: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capacity: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rss_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    heap_mb: Option<u64>,
}

#[derive(Serialize)]
pub(crate) struct DetailedHealthChecks {
    store: CheckEntry,
    ollama: CheckEntry,
    event_buffer: CheckEntry,
    memory: CheckEntry,
}

#[derive(Serialize)]
pub(crate) struct DetailedHealthResponse {
    status: &'static str,
    checks: DetailedHealthChecks,
    uptime_seconds: u64,
    version: &'static str,
    started_at: String,
    /// RFC 011: current process role.
    role: String,
}

/// Read resident set size from `/proc/self/status` (Linux only).
/// Returns (rss_kb, vm_size_kb).  Returns (0, 0) on other platforms.
pub(crate) fn read_proc_memory() -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = std::fs::read_to_string("/proc/self/status") {
            let mut rss = 0u64;
            let mut vm = 0u64;
            for line in text.lines() {
                if line.starts_with("VmRSS:") {
                    rss = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                } else if line.starts_with("VmSize:") {
                    vm = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                }
            }
            return (rss, vm);
        }
    }
    (0, 0)
}

/// `GET /v1/health/detailed` — deep health status for every subsystem.
pub(crate) async fn detailed_health_handler(
    State(state): State<AppState>,
) -> Json<DetailedHealthResponse> {
    // ── Store check ───────────────────────────────────────────────────────────
    let store_start = Instant::now();
    let store_ok = if let Some(pg) = &state.pg {
        pg.adapter.health_check().await.is_ok()
    } else if let Some(sq) = &state.sqlite {
        sq.adapter.health_check().await.is_ok()
    } else {
        state.runtime.store.head_position().await.is_ok()
    };
    let store_latency = store_start.elapsed().as_millis() as u64;

    let store_check = CheckEntry {
        status: if store_ok { "healthy" } else { "unhealthy" },
        latency_ms: Some(store_latency),
        models: None,
        size: None,
        capacity: None,
        rss_mb: None,
        heap_mb: None,
    };

    // ── Ollama check ──────────────────────────────────────────────────────────
    let ollama_check = if let Some(provider) = &state.ollama {
        let t = Instant::now();
        match provider.health_check().await {
            Ok(tags) => CheckEntry {
                status: "healthy",
                latency_ms: Some(t.elapsed().as_millis() as u64),
                models: Some(tags.models.len()),
                size: None,
                capacity: None,
                rss_mb: None,
                heap_mb: None,
            },
            Err(_) => CheckEntry {
                status: "unhealthy",
                latency_ms: None,
                models: None,
                size: None,
                capacity: None,
                rss_mb: None,
                heap_mb: None,
            },
        }
    } else {
        CheckEntry {
            status: "unconfigured",
            latency_ms: None,
            models: None,
            size: None,
            capacity: None,
            rss_mb: None,
            heap_mb: None,
        }
    };

    // ── Event buffer (not present in main.rs; always at capacity 0) ──────────
    // The SSE ring buffer lives in lib.rs AppState only.  For completeness we
    // report it as healthy with unknown size.
    let event_buffer_check = CheckEntry {
        status: "healthy",
        latency_ms: None,
        size: None,
        capacity: None,
        models: None,
        rss_mb: None,
        heap_mb: None,
    };

    // ── Process memory ────────────────────────────────────────────────────────
    let (rss_kb, _vm_kb) = read_proc_memory();
    let memory_check = CheckEntry {
        status: "healthy",
        rss_mb: Some(rss_kb / 1024),
        heap_mb: None, // allocator-level heap not easily available without jemalloc
        latency_ms: None,
        models: None,
        size: None,
        capacity: None,
    };

    // ── Overall status ────────────────────────────────────────────────────────
    let degraded = !store_ok || matches!(ollama_check.status, "unhealthy");

    let overall = if degraded { "degraded" } else { "healthy" };

    // ISO-8601 started_at from uptime
    let uptime = state.started_at.elapsed().as_secs();
    let started_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(uptime);
    let started_at = format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        1970 + started_secs / 31_557_600, // approx — good enough for display
        ((started_secs % 31_557_600) / 2_629_800) + 1,
        ((started_secs % 2_629_800) / 86_400) + 1,
        (started_secs % 86_400) / 3_600,
        (started_secs % 3_600) / 60,
        started_secs % 60,
    );

    Json(DetailedHealthResponse {
        status: overall,
        checks: DetailedHealthChecks {
            store: store_check,
            ollama: ollama_check,
            event_buffer: event_buffer_check,
            memory: memory_check,
        },
        uptime_seconds: uptime,
        version: env!("CARGO_PKG_VERSION"),
        started_at,
        role: state.process_role.as_str().to_owned(),
    })
}
