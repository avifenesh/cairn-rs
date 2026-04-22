//! Schema parity check between the Postgres migrations and the SQLite
//! monolithic DDL.
//!
//! Both backends must expose the same set of tables so that behavior is
//! portable across deployments. This is an integration-style assertion
//! against the shipped schema sources (no live database required).
//!
//! Recommended by the portability audit — see the project memory note
//! `project_pg_specific_audit.md` §7 "Pattern Analysis".
//!
//! The test is `#[ignore]` by default: we want the check to exist in the
//! tree so `cargo test -- --ignored` can be wired into CI later, but
//! turning it on today would block unrelated work on known drift that
//! the user has not yet scoped a fix for. Un-ignore once parity is
//! restored.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Crate root: `crates/cairn-store`.
fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Scan a SQL or Rust source blob for `CREATE TABLE [IF NOT EXISTS] <name>`
/// and return the lowercased, schema-prefix-stripped table names.
///
/// Intentionally simple: we walk characters, lowercase-compare against
/// `create table`, then skip optional `if not exists` and read the next
/// identifier. Works for both `.sql` files and Rust sources containing
/// raw SQL in string literals because we do not care where the text
/// comes from — only that it contains the DDL.
///
/// Limitations (acceptable for a parity check):
/// - Ignores `CREATE VIRTUAL TABLE` (FTS5 virtual tables are SQLite-only).
/// - Ignores `CREATE INDEX` / `CREATE UNIQUE INDEX`.
/// - Does not attempt to parse SQL comments specially; identifiers
///   inside `-- CREATE TABLE foo` comments would be false positives.
///   None of our schema files contain such commented-out DDL today.
fn extract_table_names(source: &str) -> BTreeSet<String> {
    let lowered = source.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let mut tables: BTreeSet<String> = BTreeSet::new();

    // We align against the original-case source to preserve identifier
    // casing for the report, then normalize on insert.
    let src_bytes = source.as_bytes();

    let mut i = 0usize;
    while i < bytes.len() {
        // Find next occurrence of "create".
        let Some(rel) = lowered[i..].find("create") else {
            break;
        };
        let start = i + rel;
        // Require that "create" is not part of a longer word.
        if start > 0 {
            let prev = bytes[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                i = start + 6;
                continue;
            }
        }
        let mut cursor = start + "create".len();
        cursor = skip_ws(bytes, cursor);

        // Skip an optional `VIRTUAL` keyword so we can reject virtual tables.
        let mut is_virtual = false;
        if matches_keyword(bytes, cursor, b"virtual") {
            is_virtual = true;
            cursor += "virtual".len();
            cursor = skip_ws(bytes, cursor);
        }

        if !matches_keyword(bytes, cursor, b"table") {
            i = start + 6;
            continue;
        }
        cursor += "table".len();
        cursor = skip_ws(bytes, cursor);

        // Optional `IF NOT EXISTS`.
        if matches_keyword(bytes, cursor, b"if") {
            cursor += "if".len();
            cursor = skip_ws(bytes, cursor);
            if matches_keyword(bytes, cursor, b"not") {
                cursor += "not".len();
                cursor = skip_ws(bytes, cursor);
                if matches_keyword(bytes, cursor, b"exists") {
                    cursor += "exists".len();
                    cursor = skip_ws(bytes, cursor);
                }
            }
        }

        // Read identifier. Accept an optional `schema.` prefix.
        let (name, next) = read_identifier(src_bytes, cursor);
        if let Some(name) = name {
            if !is_virtual {
                // Strip schema prefix if present: `public.foo` -> `foo`.
                let bare = name
                    .rsplit('.')
                    .next()
                    .unwrap_or(&name)
                    .to_ascii_lowercase();
                tables.insert(bare);
            }
            i = next;
        } else {
            i = start + 6;
        }
    }

    tables
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    i
}

/// Returns true if the lowercased keyword matches at position `i` and is
/// followed by a non-identifier character (word-boundary check).
fn matches_keyword(bytes: &[u8], i: usize, kw: &[u8]) -> bool {
    if i + kw.len() > bytes.len() {
        return false;
    }
    if &bytes[i..i + kw.len()] != kw {
        return false;
    }
    let after = i + kw.len();
    if after == bytes.len() {
        return true;
    }
    let c = bytes[after];
    !(c.is_ascii_alphanumeric() || c == b'_')
}

/// Read an identifier starting at `i` (possibly containing a single `.`
/// for schema-prefix). Returns the identifier and the cursor just past it.
fn read_identifier(bytes: &[u8], start: usize) -> (Option<String>, usize) {
    // Allow optional `"ident"` quoting — strip the quotes.
    let mut i = start;
    let quoted = i < bytes.len() && bytes[i] == b'"';
    if quoted {
        i += 1;
    }
    let id_start = i;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    if id_start == i {
        return (None, start);
    }
    let raw = std::str::from_utf8(&bytes[id_start..i])
        .unwrap_or("")
        .to_string();
    let end = if quoted && i < bytes.len() && bytes[i] == b'"' {
        i + 1
    } else {
        i
    };
    if raw.is_empty() {
        (None, start)
    } else {
        (Some(raw), end)
    }
}

/// Collect table names from every `.sql` file under a directory.
fn tables_from_sql_dir(dir: &Path) -> BTreeSet<String> {
    let mut tables: BTreeSet<String> = BTreeSet::new();
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {:?}: {}", dir, e));
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let content =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {:?}: {}", path, e));
        for t in extract_table_names(&content) {
            tables.insert(t);
        }
    }
    tables
}

/// Internal bookkeeping tables (created by the migration runner, not
/// part of the product schema). These are excluded from parity.
fn infra_tables() -> BTreeSet<String> {
    ["_cairn_migrations"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[test]
#[ignore = "known schema drift; un-ignore when SQLite reaches parity with Postgres (see project_pg_specific_audit.md §V018, V019 and V016/V017 tables)"]
fn postgres_and_sqlite_define_the_same_tables() {
    let root = crate_root();

    // Postgres migrations live in two directories:
    //   crates/cairn-store/migrations/        (V001-V017)
    //   crates/cairn-store/src/pg/migrations/ (V018+)
    let mut pg_tables = tables_from_sql_dir(&root.join("migrations"));
    pg_tables.extend(tables_from_sql_dir(&root.join("src/pg/migrations")));

    // SQLite schema is a Rust source file with the DDL inside a raw
    // string literal. Read the file verbatim and pattern-match the DDL
    // inside — per the audit's suggestion, "good enough" beats a
    // tolerant string-literal parser.
    let sqlite_schema_path = root.join("src/sqlite/schema.rs");
    let sqlite_source = fs::read_to_string(&sqlite_schema_path)
        .unwrap_or_else(|e| panic!("read {:?}: {}", sqlite_schema_path, e));
    let mut sqlite_tables = extract_table_names(&sqlite_source);

    // Remove infrastructure tables from both sides.
    let infra = infra_tables();
    for t in &infra {
        pg_tables.remove(t);
        sqlite_tables.remove(t);
    }

    let only_in_postgres: BTreeSet<String> =
        pg_tables.difference(&sqlite_tables).cloned().collect();
    let only_in_sqlite: BTreeSet<String> = sqlite_tables.difference(&pg_tables).cloned().collect();

    if only_in_postgres.is_empty() && only_in_sqlite.is_empty() {
        return;
    }

    let mut msg = String::from("schema parity violation between Postgres and SQLite\n");
    msg.push_str(&format!(
        "\ntables only in Postgres ({}):\n",
        only_in_postgres.len()
    ));
    for t in &only_in_postgres {
        msg.push_str(&format!("  - {t}\n"));
    }
    msg.push_str(&format!(
        "\ntables only in SQLite ({}):\n",
        only_in_sqlite.len()
    ));
    for t in &only_in_sqlite {
        msg.push_str(&format!("  - {t}\n"));
    }
    msg.push_str("\nSee project_pg_specific_audit.md in project memory for context.");

    panic!("{msg}");
}

// ── unit checks for the parser ────────────────────────────────────────────

#[test]
fn parser_handles_if_not_exists_and_schema_prefix() {
    let sql = "CREATE TABLE foo (id INT);\n\
               CREATE TABLE IF NOT EXISTS public.bar (id INT);\n\
               create table \"baz\" (id int);";
    let tables = extract_table_names(sql);
    assert!(tables.contains("foo"), "missing foo: {:?}", tables);
    assert!(
        tables.contains("bar"),
        "missing bar (schema-prefix): {:?}",
        tables
    );
    assert!(tables.contains("baz"), "missing baz (quoted): {:?}", tables);
}

#[test]
fn parser_skips_create_index_and_create_virtual_table() {
    let sql = "CREATE TABLE real (id INT);\n\
               CREATE INDEX idx_real ON real(id);\n\
               CREATE UNIQUE INDEX idx_real_unique ON real(id);\n\
               CREATE VIRTUAL TABLE fts USING fts5(x);";
    let tables = extract_table_names(sql);
    assert_eq!(
        tables,
        ["real"]
            .iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>(),
        "indexes and virtual tables should not count"
    );
}

#[test]
fn parser_does_not_match_inside_identifiers() {
    // The literal word "createtable" (no space) must not match.
    let sql = "-- createtablefoo is not DDL\nCREATE TABLE real (id INT);";
    let tables = extract_table_names(sql);
    assert_eq!(tables.len(), 1);
    assert!(tables.contains("real"));
}
