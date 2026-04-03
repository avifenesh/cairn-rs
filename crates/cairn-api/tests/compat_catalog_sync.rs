use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use cairn_api::http::{preserved_route_catalog, HttpMethod, RouteClassification};
use cairn_api::sse::preserved_sse_catalog;

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

#[test]
fn http_catalog_matches_compat_inventory() {
    let expected: BTreeSet<_> = read_tsv("tests/compat/http_routes.tsv", 5)
        .into_iter()
        .map(|row| format!("{}\t{}\t{}", row[0], row[1], row[2]))
        .collect();

    let actual: BTreeSet<_> = preserved_route_catalog()
        .into_iter()
        .map(|entry| {
            format!(
                "{}\t{}\t{}",
                method_str(entry.method),
                entry.path,
                classification_str(entry.classification)
            )
        })
        .collect();

    assert_eq!(actual, expected, "cairn-api route catalog drifted from tests/compat/http_routes.tsv");
}

#[test]
fn sse_catalog_matches_compat_inventory() {
    let expected: BTreeSet<_> = read_tsv("tests/compat/sse_events.tsv", 3)
        .into_iter()
        .map(|row| format!("{}\t{}", row[0], row[1]))
        .collect();

    let actual: BTreeSet<_> = preserved_sse_catalog()
        .into_iter()
        .map(|entry| format!("{}\t{}", entry.name, classification_str(entry.classification)))
        .collect();

    assert_eq!(actual, expected, "cairn-api SSE catalog drifted from tests/compat/sse_events.tsv");
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

        let found = catalog.iter().any(|entry| {
            method_str(entry.method) == method && entry.path == base_path
        });

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
