//! Coverage test for per-subject HTTP API docs under `docs/api/`.
//!
//! Asserts that every route in `tests/compat/http_routes.tsv` appears in
//! exactly one `docs/api/*.md` file. Sibling to `compat_catalog_sync.rs`,
//! which keeps the TSV in sync with the live router. Together they form the
//! chain: router → TSV → per-subject docs.
//!
//! If this test fails, a new route has been added without being placed in a
//! subject doc, or a route has been listed in two subject files (ambiguous
//! partition).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative)
}

fn read_tsv_routes() -> BTreeSet<(String, String)> {
    let path = repo_file("tests/compat/http_routes.tsv");
    let contents = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    contents
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let mut cols = line.split('\t');
            let method = cols.next().expect("method").to_owned();
            let path = cols.next().expect("path").to_owned();
            (method, path)
        })
        .collect()
}

/// Collect `(method, path)` pairs mentioned in each subject doc. A line is
/// considered a route row if it is a markdown table row whose first two
/// backtick-quoted fields look like a method and a path.
fn read_doc_routes() -> BTreeMap<(String, String), Vec<String>> {
    let dir = repo_file("docs/api");
    let mut out: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();

    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()));

    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        // Skip the index.
        if file_name == "README.md" {
            continue;
        }
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        for line in contents.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with("| `") {
                continue;
            }
            // Expected format:
            //   | `METHOD` | `/path` | Classification | notes |
            let cells: Vec<&str> = trimmed.split('|').collect();
            if cells.len() < 4 {
                continue;
            }
            let method = extract_backticked(cells[1]);
            let route_path = extract_backticked(cells[2]);
            if let (Some(m), Some(p)) = (method, route_path) {
                if is_http_method(&m) && p.starts_with('/') {
                    out.entry((m, p)).or_default().push(file_name.clone());
                }
            }
        }
    }

    out
}

fn extract_backticked(cell: &str) -> Option<String> {
    let cell = cell.trim();
    let start = cell.find('`')?;
    let rest = &cell[start + 1..];
    let end = rest.find('`')?;
    Some(rest[..end].to_owned())
}

fn is_http_method(s: &str) -> bool {
    matches!(s, "GET" | "POST" | "PUT" | "DELETE" | "PATCH")
}

#[test]
fn every_tsv_route_is_documented_exactly_once() {
    let tsv = read_tsv_routes();
    let docs = read_doc_routes();

    let doc_keys: BTreeSet<(String, String)> = docs.keys().cloned().collect();

    let missing: Vec<_> = tsv.difference(&doc_keys).collect();
    let orphan: Vec<_> = doc_keys.difference(&tsv).collect();
    let duplicate: Vec<(&(String, String), &Vec<String>)> =
        docs.iter().filter(|(_, files)| files.len() > 1).collect();

    if !missing.is_empty() || !orphan.is_empty() || !duplicate.is_empty() {
        let mut msg = String::from(
            "docs/api/ coverage drifted from tests/compat/http_routes.tsv.\n\
             Every route in the TSV must appear in exactly one docs/api/*.md file.\n",
        );
        if !missing.is_empty() {
            msg.push_str(&format!(
                "\nRoutes in TSV but not documented ({}):\n",
                missing.len()
            ));
            for (m, p) in &missing {
                msg.push_str(&format!("  + {m}\t{p}\n"));
            }
        }
        if !orphan.is_empty() {
            msg.push_str(&format!(
                "\nRoutes documented but not in TSV ({}):\n",
                orphan.len()
            ));
            for (m, p) in &orphan {
                msg.push_str(&format!("  - {m}\t{p}\n"));
            }
        }
        if !duplicate.is_empty() {
            msg.push_str(&format!(
                "\nRoutes documented in multiple files ({}):\n",
                duplicate.len()
            ));
            for ((m, p), files) in &duplicate {
                msg.push_str(&format!("  * {m}\t{p}\t{:?}\n", files));
            }
        }
        panic!("{msg}");
    }
}
