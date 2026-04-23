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

fn main() {
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
        .or_else(|| {
            // `date -u +%Y-%m-%dT%H:%M:%SZ` is available on every target we ship on.
            Command::new("date")
                .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=GIT_COMMIT={git_commit}");
    println!("cargo:rustc-env=BUILD_DATE={build_date}");

    // Re-run when env overrides change. We intentionally do NOT watch HEAD —
    // rebuilding on every commit would thrash incremental builds; CI injects
    // `GIT_COMMIT`/`BUILD_DATE` explicitly for release artifacts.
    println!("cargo:rerun-if-env-changed=GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=BUILD_DATE");
    println!("cargo:rerun-if-changed=build.rs");
}
