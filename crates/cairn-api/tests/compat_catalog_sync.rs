//! Compat inventory sync for HTTP routes and SSE events.
//!
//! # Route drift contract
//!
//! This test catches drift between the actual HTTP route surface and
//! `tests/compat/http_routes.tsv`. If it fails, the TSV is authoritative for
//! the `method + route + classification` triple but the code is authoritative
//! for whether a route exists at all.
//!
//! ## What is checked
//!
//! The "real" route set is the union of three code sources:
//!
//! 1. `cairn_api::http::preserved_route_catalog()` — the 132 Phase-0 preserved
//!    routes registered by the fold inside `AppBootstrap::build_catalog_routes`.
//! 2. Explicit `.route("/path", method(handler))` calls in
//!    `crates/cairn-app/src/router.rs` — the library router's non-catalog
//!    handlers chained after the fold.
//! 3. Explicit `.route(...)` calls in `crates/cairn-app/src/bin_router.rs` —
//!    production binary-specific routes merged over the catalog router.
//!
//! ## What to do on drift
//!
//! ```bash
//! cargo test -p cairn-api --test compat_catalog_sync \
//!     regenerate_http_routes_tsv -- --ignored --nocapture
//! ```
//!
//! That regen test rewrites `tests/compat/http_routes.tsv` from the code.
//! Review the diff, then commit. The regen preserves the
//! `query_or_body_detail` and `minimum_contract` columns for existing rows
//! and emits placeholder `-` values for newly discovered routes — triage
//! those placeholders in a follow-up.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use cairn_api::http::{preserved_route_catalog, HttpMethod, RouteClassification};
use cairn_api::sse::preserved_sse_catalog;

#[path = "support/route_source_parser.rs"]
mod route_source_parser;
use route_source_parser::{parse_route_file, ParsedRoute};

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative)
}

fn read_tsv(relative: &str, expected_columns: usize) -> Vec<Vec<String>> {
    let path = repo_file(relative);
    let contents = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));

    contents
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let columns: Vec<String> = line.split('\t').map(str::to_owned).collect();
            assert_eq!(
                columns.len(),
                expected_columns,
                "unexpected column count in {}: {}",
                path.display(),
                line
            );
            columns
        })
        .collect()
}

fn method_str(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Patch => "PATCH",
    }
}

fn classification_str(classification: RouteClassification) -> &'static str {
    match classification {
        RouteClassification::Preserve => "preserve",
        RouteClassification::Transitional => "transitional",
        RouteClassification::IntentionallyBroken => "intentionally_broken",
    }
}

/// Collect the authoritative set of `(method, path)` pairs registered by the
/// running server. Union of catalog + library router + binary router.
fn collect_real_routes() -> BTreeSet<(String, String)> {
    let mut set = BTreeSet::new();

    for entry in preserved_route_catalog() {
        set.insert((method_str(entry.method).to_owned(), entry.path));
    }

    let lib = parse_route_file(&repo_file("crates/cairn-app/src/router.rs"));
    let bin = parse_route_file(&repo_file("crates/cairn-app/src/bin_router.rs"));
    for ParsedRoute { method, path } in lib.into_iter().chain(bin) {
        set.insert((method, path));
    }

    set
}

#[test]
fn http_catalog_matches_compat_inventory() {
    let tsv_rows = read_tsv("tests/compat/http_routes.tsv", 5);
    let expected: BTreeSet<(String, String)> = tsv_rows
        .iter()
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect();

    let actual = collect_real_routes();

    if actual != expected {
        let missing_in_tsv: Vec<_> = actual.difference(&expected).collect();
        let stale_in_tsv: Vec<_> = expected.difference(&actual).collect();

        let mut msg = String::from(
            "tests/compat/http_routes.tsv drifted from actual router surface.\n\
             Run `cargo test -p cairn-api --test compat_catalog_sync \
             regenerate_http_routes_tsv -- --ignored --nocapture` to refresh.\n",
        );
        if !missing_in_tsv.is_empty() {
            msg.push_str(&format!(
                "\nRoutes present in code but missing from TSV ({}):\n",
                missing_in_tsv.len()
            ));
            for (m, p) in &missing_in_tsv {
                msg.push_str(&format!("  + {m}\t{p}\n"));
            }
        }
        if !stale_in_tsv.is_empty() {
            msg.push_str(&format!(
                "\nRoutes present in TSV but missing from code ({}):\n",
                stale_in_tsv.len()
            ));
            for (m, p) in &stale_in_tsv {
                msg.push_str(&format!("  - {m}\t{p}\n"));
            }
        }
        panic!("{msg}");
    }
}

/// Regenerate `tests/compat/http_routes.tsv` from the current code surface.
///
/// Preserves the `classification`, `query_or_body_detail`, and
/// `minimum_contract` columns of rows already present in the TSV.
/// Emits `preserve` / `-` / `-` placeholders for newly discovered routes.
#[test]
#[ignore = "regen tool: run explicitly with --ignored to rewrite the TSV"]
fn regenerate_http_routes_tsv() {
    let tsv_path = repo_file("tests/compat/http_routes.tsv");
    let existing_rows = read_tsv("tests/compat/http_routes.tsv", 5);

    /// Preserved columns for a route row: everything beyond `(method, path)`.
    struct RowMetadata {
        classification: String,
        detail: String,
        contract: String,
    }

    // Key existing rows by (method, path) so we can reuse their metadata.
    let mut existing: BTreeMap<(String, String), RowMetadata> = BTreeMap::new();
    for row in existing_rows {
        let key = (row[0].clone(), row[1].clone());
        existing.insert(
            key,
            RowMetadata {
                classification: row[2].clone(),
                detail: row[3].clone(),
                contract: row[4].clone(),
            },
        );
    }

    // Classification for catalog entries comes from the code, so prefer that
    // over whatever the TSV said (keeps `transitional` flags authoritative).
    let mut classification_override: BTreeMap<(String, String), String> = BTreeMap::new();
    for entry in preserved_route_catalog() {
        classification_override.insert(
            (method_str(entry.method).to_owned(), entry.path),
            classification_str(entry.classification).to_owned(),
        );
    }

    let real = collect_real_routes();

    let mut out = String::new();
    out.push_str("method\troute\tclassification\tquery_or_body_detail\tminimum_contract\n");
    for (method, path) in &real {
        let key = (method.clone(), path.clone());
        let (classification, detail, contract) = match existing.get(&key) {
            Some(row) => {
                let cls = classification_override
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| row.classification.clone());
                (cls, row.detail.clone(), row.contract.clone())
            }
            None => {
                let cls = classification_override
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "preserve".to_owned());
                (cls, "-".to_owned(), "-".to_owned())
            }
        };
        out.push_str(&format!(
            "{method}\t{path}\t{classification}\t{detail}\t{contract}\n"
        ));
    }

    write_atomically(&tsv_path, &out);
    eprintln!("wrote {} routes to {}", real.len(), tsv_path.display());
}

fn write_atomically(path: &Path, contents: &str) {
    let tmp = path.with_extension("tsv.tmp");
    fs::write(&tmp, contents).unwrap_or_else(|e| panic!("write {} failed: {e}", tmp.display()));
    fs::rename(&tmp, path).unwrap_or_else(|e| panic!("rename to {} failed: {e}", path.display()));
}

#[test]
fn sse_catalog_matches_compat_inventory() {
    let expected: BTreeSet<_> = read_tsv("tests/compat/sse_events.tsv", 3)
        .into_iter()
        .map(|row| format!("{}\t{}", row[0], row[1]))
        .collect();

    let actual: BTreeSet<_> = preserved_sse_catalog()
        .into_iter()
        .map(|entry| {
            format!(
                "{}\t{}",
                entry.name,
                classification_str(entry.classification)
            )
        })
        .collect();

    assert_eq!(
        actual, expected,
        "cairn-api SSE catalog drifted from tests/compat/sse_events.tsv"
    );
}

#[test]
fn phase0_required_http_is_backed_by_api_catalog() {
    let catalog = preserved_route_catalog();
    let requirements = fs::read_to_string(repo_file("tests/compat/phase0_required_http.txt"))
        .expect("failed to read phase0_required_http.txt");

    for requirement in requirements.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = requirement.split_whitespace();
        let method = parts.next().expect("missing method");
        let path = parts.next().expect("missing path");
        let base_path = path.split('?').next().expect("base path");

        let found = catalog
            .iter()
            .any(|entry| method_str(entry.method) == method && entry.path == base_path);

        assert!(
            found,
            "required phase0 HTTP surface `{requirement}` missing from cairn-api route catalog"
        );
    }
}

#[test]
fn phase0_required_sse_is_backed_by_api_catalog() {
    let catalog: BTreeSet<_> = preserved_sse_catalog()
        .into_iter()
        .map(|entry| entry.name)
        .collect();

    let requirements = fs::read_to_string(repo_file("tests/compat/phase0_required_sse.txt"))
        .expect("failed to read phase0_required_sse.txt");

    for event_name in requirements.lines().filter(|line| !line.trim().is_empty()) {
        assert!(
            catalog.contains(event_name),
            "required phase0 SSE event `{event_name}` missing from cairn-api SSE catalog"
        );
    }
}
