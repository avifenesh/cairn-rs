//! Build script: embed git commit SHA and build date into the binary so
//! `/v1/system/info` can report real values instead of `"dev"` /
//! `"build date not embedded"` placeholders.
//!
//! Precedence for each value:
//!   1. `GIT_COMMIT` / `BUILD_DATE` already set in the environment (CI can inject)
//!   2. Fallback: shell out to `git` / capture UTC date at build time
//!   3. Last resort: `"unknown"` — we never emit the old placeholder strings
//!
//! Re-runs only when the build inputs change; we don't force a rebuild on
//! every invocation.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Format the current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Implemented directly against `SystemTime` so we don't shell out to `date`
/// (not portable — fails on Windows) and don't pull in a build-dependency just
/// for a timestamp. The proleptic Gregorian calendar is correct for all dates
/// after 1582 and is what ISO-8601 specifies.
/// Proleptic Gregorian leap-year rule.
fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn iso8601_utc_now() -> String {
    let secs = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => return "unknown".to_owned(),
    };

    // Break seconds-since-epoch into YMD HMS (UTC, no leap seconds).
    let day_secs = 86_400u64;
    let days = secs / day_secs;
    let rem = secs % day_secs;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;

    // 1970-01-01 is day 0. Walk years + months.
    let mut year: u64 = 1970;
    let mut days_left = days;
    loop {
        let leap = is_leap(year);
        let year_days = if leap { 366 } else { 365 };
        if days_left < year_days {
            break;
        }
        days_left -= year_days;
        year += 1;
    }
    let leap = is_leap(year);
    let month_lens: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u64 = 0;
    while month < 12 && days_left >= month_lens[month as usize] {
        days_left -= month_lens[month as usize];
        month += 1;
    }
    let day = days_left + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month + 1,
        day,
        hour,
        min,
        sec,
    )
}

fn main() {
    // Capture rustc version so `/v1/system/info.rust_version` reports the
    // toolchain (e.g. `rustc 1.95.0 (abc123 2026-04-10)`) instead of the crate
    // version. Falls back to "unknown" if `rustc` isn't on PATH (rare inside a
    // cargo build, but defensive).
    let rust_version = Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned()))
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());

    let git_commit = std::env::var("GIT_COMMIT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_owned());

    let build_date = std::env::var("BUILD_DATE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(iso8601_utc_now);

    println!("cargo:rustc-env=GIT_COMMIT={git_commit}");
    println!("cargo:rustc-env=BUILD_DATE={build_date}");
    println!("cargo:rustc-env=RUSTC_VERSION={rust_version}");

    // Re-run when env overrides change OR when HEAD moves, so that
    // `GIT_COMMIT`/`BUILD_DATE` don't go stale across incremental rebuilds on
    // a developer machine. We only watch `.git/HEAD` and the packed-refs file
    // (both rewritten by `git commit` / `git checkout`), which is cheap —
    // not every file in the repo. CI builds inject the env vars explicitly
    // so these file-watches are a no-op there.
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=BUILD_DATE");
    println!("cargo:rerun-if-changed=build.rs");
    if std::path::Path::new("../../.git/HEAD").exists() {
        println!("cargo:rerun-if-changed=../../.git/HEAD");
    }
    if std::path::Path::new("../../.git/packed-refs").exists() {
        println!("cargo:rerun-if-changed=../../.git/packed-refs");
    }
}
