//! Source-code parser for `axum::Router::route` registrations.
//!
//! Used by the compat-inventory test to extract `(method, path)` pairs from
//! `crates/cairn-app/src/router.rs` and `crates/cairn-app/src/bin_router.rs`
//! without booting the server or introspecting `axum::Router` (which does not
//! expose a public iteration API on 0.7).
//!
//! The parser recognises the syntactic form:
//!
//! ```ignore
//! .route("<path>", <methods-and-handlers>)
//! ```
//!
//! where `<methods-and-handlers>` is any expression — we scan the balanced
//! parenthesised body for top-level identifiers `get`, `post`, `put`,
//! `delete`, `patch` immediately followed by `(`. Chained forms such as
//! `get(h).post(h2)` emit two entries.
//!
//! `#[cfg(...)]`-gated routes are NOT emitted; those need manual curation if
//! the feature set they live behind ever becomes part of the public contract.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ParsedRoute {
    pub method: String,
    pub path: String,
}

pub fn parse_route_file(path: &Path) -> Vec<ParsedRoute> {
    let source =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("read {} failed: {e}", path.display()));
    parse_route_source(&source)
}

fn parse_route_source(source: &str) -> Vec<ParsedRoute> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < n {
        // Find the next `.route(` outside of comments / strings.
        let Some(idx) = find_next_route_call(bytes, i) else {
            break;
        };

        // Skip if this `.route(` sits on a line annotated by a `#[cfg(...)]`
        // attribute — we conservatively ignore feature-gated routes.
        let after_paren = idx + ".route(".len();
        if preceding_line_is_cfg_gated(bytes, idx) {
            i = after_paren;
            continue;
        }

        // Parse the first argument: expect either a string literal (path) or
        // `&path` (the fold-driven registrations in `build_catalog_routes`,
        // which we pick up via `preserved_route_catalog()` elsewhere).
        let mut j = after_paren;
        while j < n && is_ws(bytes[j]) {
            j += 1;
        }
        if j >= n || bytes[j] != b'"' {
            // Not a literal path — skip past this call.
            i = after_paren;
            continue;
        }

        // Extract path string literal (no escape handling beyond skipping
        // `\"`; route paths never need other escapes in practice).
        let path_start = j + 1;
        let mut k = path_start;
        while k < n && bytes[k] != b'"' {
            if bytes[k] == b'\\' && k + 1 < n {
                k += 2;
            } else {
                k += 1;
            }
        }
        if k >= n {
            break;
        }
        let path = std::str::from_utf8(&bytes[path_start..k])
            .expect("route path is valid utf8")
            .to_owned();

        // Find the balanced close paren of `.route(`.
        let (body_start, body_end, after) = match balanced_close(bytes, after_paren) {
            Some(v) => v,
            None => break,
        };

        let body = &source[body_start..body_end];
        for method in extract_top_level_methods(body) {
            out.push(ParsedRoute {
                method,
                path: path.clone(),
            });
        }

        i = after;
    }

    out
}

/// Scan for the next `.route(` that is outside strings, comments, and
/// `.nest(...)` blocks. Routes nested under a mount-path prefix are NOT
/// full paths — their effective URL is `<prefix>/<sub>` and the full path
/// is expected to be carried by `preserved_route_catalog()` instead.
fn find_next_route_call(bytes: &[u8], start: usize) -> Option<usize> {
    let pat = b".route(";
    let nest_pat = b".nest(";
    let n = bytes.len();
    let mut i = start;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_string: Option<u8> = None;
    let mut raw_hashes: usize = 0;

    while i < n {
        let b = bytes[i];
        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if b == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(quote) = in_string {
            if b == b'\\' && raw_hashes == 0 {
                i += 2;
                continue;
            }
            if b == quote {
                // Raw strings end on `"` followed by the matching `#` count;
                // we approximate by treating any run of `#`s after `"` as part
                // of the terminator. Good enough for router source files.
                if raw_hashes == 0 {
                    in_string = None;
                } else {
                    let mut hashes = 0usize;
                    let mut j = i + 1;
                    while j < n && bytes[j] == b'#' && hashes < raw_hashes {
                        hashes += 1;
                        j += 1;
                    }
                    if hashes == raw_hashes {
                        in_string = None;
                        raw_hashes = 0;
                        i = j;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }
        // Not in a comment or string.
        if b == b'/' && i + 1 < n {
            if bytes[i + 1] == b'/' {
                in_line_comment = true;
                i += 2;
                continue;
            }
            if bytes[i + 1] == b'*' {
                in_block_comment = true;
                i += 2;
                continue;
            }
        }
        if b == b'"' {
            in_string = Some(b'"');
            raw_hashes = 0;
            i += 1;
            continue;
        }
        if b == b'r' && i + 1 < n && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') {
            let mut j = i + 1;
            let mut hashes = 0usize;
            while j < n && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && bytes[j] == b'"' {
                in_string = Some(b'"');
                raw_hashes = hashes;
                i = j + 1;
                continue;
            }
        }
        if b == b'.' && i + nest_pat.len() <= n && &bytes[i..i + nest_pat.len()] == nest_pat {
            // Skip the entire balanced body of this `.nest(` call. Any
            // `.route(` inside registers a SUB-path under a mount prefix;
            // the full path belongs to `preserved_route_catalog()`.
            let open_paren = i + nest_pat.len() - 1; // points at `(`
            if let Some((_, _, after)) = balanced_close(bytes, open_paren + 1) {
                i = after;
                continue;
            } else {
                // Unterminated `.nest(` — bail out defensively.
                return None;
            }
        }
        if b == b'.' && i + pat.len() <= n && &bytes[i..i + pat.len()] == pat {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find the balanced `)` matching the `(` at `open_paren - 1`. Returns
/// `(body_start, body_end, after_close)` where the body is the slice strictly
/// between the parens.
fn balanced_close(bytes: &[u8], open_paren: usize) -> Option<(usize, usize, usize)> {
    let n = bytes.len();
    let mut depth = 1i32;
    let mut i = open_paren;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_string: Option<u8> = None;

    while i < n {
        let b = bytes[i];
        if in_line_comment {
            if b == b'\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if b == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(q) = in_string {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                in_string = None;
            }
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < n {
            if bytes[i + 1] == b'/' {
                in_line_comment = true;
                i += 2;
                continue;
            }
            if bytes[i + 1] == b'*' {
                in_block_comment = true;
                i += 2;
                continue;
            }
        }
        if b == b'"' {
            in_string = Some(b'"');
            i += 1;
            continue;
        }
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return Some((open_paren, i, i + 1));
            }
        }
        i += 1;
    }
    None
}

/// Extract HTTP method identifiers (`get`, `post`, ...) used at the top
/// level of the route body. Does NOT descend into inner parens — that
/// correctly skips handler arguments like `with_state(foo.bar())`. Skips
/// string literals and `// ... ` / `/* ... */` comments.
fn extract_top_level_methods(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let n = bytes.len();
    let mut methods = Vec::new();
    let mut depth = 0i32;
    let mut i = 0usize;

    // Skip a line-comment starting at `i` (assumes `//`). Returns new index.
    fn skip_line_comment(bytes: &[u8], mut i: usize) -> usize {
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        i
    }
    // Skip a block-comment starting at `i` (assumes `/*`). Returns index
    // just past the closing `*/`, or `bytes.len()` if unterminated.
    fn skip_block_comment(bytes: &[u8], mut i: usize) -> usize {
        // `i` points at `/`, step past `/*`
        i += 2;
        while i + 1 < bytes.len() {
            if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                return i + 2;
            }
            i += 1;
        }
        bytes.len()
    }
    // Skip a double-quoted string literal starting at `i` (assumes `"`).
    fn skip_string(bytes: &[u8], mut i: usize) -> usize {
        i += 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                return i + 1;
            }
            i += 1;
        }
        bytes.len()
    }

    // Skip over the first argument (the path literal / `&path`) up to the
    // first top-level `,`.
    while i < n {
        let b = bytes[i];
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if b == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if b == b'(' || b == b'[' || b == b'{' {
            depth += 1;
        } else if b == b')' || b == b']' || b == b'}' {
            depth -= 1;
        } else if b == b',' && depth == 0 {
            i += 1;
            break;
        }
        i += 1;
    }

    // Now walk the handler expression looking for top-level `method(`.
    while i < n {
        let b = bytes[i];
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            i = skip_line_comment(bytes, i);
            continue;
        }
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if b == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if b == b'(' || b == b'[' || b == b'{' {
            depth += 1;
            i += 1;
            continue;
        }
        if b == b')' || b == b']' || b == b'}' {
            depth -= 1;
            i += 1;
            continue;
        }
        // Track depth==0 identifiers. Chained method calls like
        // `get(...).post(...)` sit at depth 0 because `.post(` opens a new
        // paren after the previous `)` closes.
        if depth == 0 && is_ident_start(b) {
            let start = i;
            while i < n && is_ident_cont(bytes[i]) {
                i += 1;
            }
            let ident = &body[start..i];
            // Skip whitespace then check for `(`.
            let mut j = i;
            while j < n && is_ws(bytes[j]) {
                j += 1;
            }
            if j < n && bytes[j] == b'(' {
                if let Some(m) = http_method_for(ident) {
                    methods.push(m.to_owned());
                }
            }
            continue;
        }
        i += 1;
    }

    methods
}

fn http_method_for(ident: &str) -> Option<&'static str> {
    match ident {
        "get" => Some("GET"),
        "post" => Some("POST"),
        "put" => Some("PUT"),
        "delete" => Some("DELETE"),
        "patch" => Some("PATCH"),
        _ => None,
    }
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Returns true if the `.route(` call starting at `route_idx` belongs to a
/// statement guarded by `#[cfg(...)]`.
///
/// Walks back over the run of contiguous attribute lines (`#[...]`) that sit
/// directly above the `.route` call's statement. Returns true if any of them
/// is `#[cfg`. A non-attribute, non-blank line ends the run — this avoids
/// false positives from unrelated `#[cfg]` lines further up.
fn preceding_line_is_cfg_gated(bytes: &[u8], route_idx: usize) -> bool {
    // Move back to the start of the current line.
    let mut i = route_idx;
    while i > 0 && bytes[i - 1] != b'\n' {
        i -= 1;
    }
    // Walk back over contiguous `#[...]` attribute lines (skipping blanks).
    // Any `#[cfg(...)]` in the run gates this route.
    loop {
        if i == 0 {
            return false;
        }
        i -= 1; // skip the '\n' that ended the previous line
        let line_end = i;
        while i > 0 && bytes[i - 1] != b'\n' {
            i -= 1;
        }
        let line = &bytes[i..line_end];
        let trimmed = trim_ascii(line);
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with(b"#[cfg") {
            return true;
        }
        if trimmed.starts_with(b"#[") {
            // Some other attribute (e.g. `#[must_use]`). Keep walking — a
            // subsequent line might hold the `#[cfg]`.
            continue;
        }
        return false;
    }
}

fn trim_ascii(s: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = s.len();
    while start < end && is_ws(s[start]) {
        start += 1;
    }
    while end > start && is_ws(s[end - 1]) {
        end -= 1;
    }
    &s[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_line_route() {
        let src = r#"
            .route("/health", get(h))
        "#;
        let parsed = parse_route_source(src);
        assert_eq!(
            parsed,
            vec![ParsedRoute {
                method: "GET".into(),
                path: "/health".into()
            }]
        );
    }

    #[test]
    fn parses_multiline_route() {
        let src = "
            .route(
                \"/v1/x\",
                post(handler),
            )
        ";
        let parsed = parse_route_source(src);
        assert_eq!(
            parsed,
            vec![ParsedRoute {
                method: "POST".into(),
                path: "/v1/x".into()
            }]
        );
    }

    #[test]
    fn parses_chained_methods() {
        let src = r#".route("/v1/y", get(h).post(h2).delete(h3))"#;
        let parsed = parse_route_source(src);
        let methods: Vec<_> = parsed.iter().map(|r| r.method.clone()).collect();
        assert_eq!(methods, vec!["GET", "POST", "DELETE"]);
        assert!(parsed.iter().all(|r| r.path == "/v1/y"));
    }

    #[test]
    fn skips_fold_style_route() {
        // `.route(&path, get(...))` has no string literal → skipped here.
        // The catalog fold is accounted for separately via
        // `preserved_route_catalog()`.
        let src = r#".route(&path, get(h))"#;
        assert!(parse_route_source(src).is_empty());
    }

    #[test]
    fn ignores_strings_and_comments() {
        let src = r#"
            // .route("/fake", get(h))
            /* .route("/alsofake", post(h)) */
            let s = ".route(\"/alsoalsofake\", get(h))";
            .route("/real", get(h))
        "#;
        let parsed = parse_route_source(src);
        assert_eq!(
            parsed,
            vec![ParsedRoute {
                method: "GET".into(),
                path: "/real".into()
            }]
        );
    }

    #[test]
    fn skips_cfg_gated_route() {
        let src = "
            #[cfg(feature = \"debug-endpoints\")]
            let router = router.route(\"/gated\", get(h));
            let other = x.route(\"/kept\", post(h));
        ";
        let parsed = parse_route_source(src);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "/kept");
    }

    #[test]
    fn skips_methods_inside_comments_within_route_body() {
        // The classic false-positive: an HTTP method ident sits inside an
        // inline comment. We must not count it.
        let src = r#".route("/path", /* get(h) */ post(h2))"#;
        let parsed = parse_route_source(src);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].method, "POST");
    }

    #[test]
    fn skips_methods_inside_line_comment_within_route_body() {
        let src = "
            .route(\"/p\", // get(ignored)
                post(h))
        ";
        let parsed = parse_route_source(src);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].method, "POST");
    }

    #[test]
    fn cfg_gate_walks_back_over_stacked_attributes() {
        // `#[cfg]` stacked under another attribute still gates the route.
        let src = "
            #[cfg(feature = \"x\")]
            #[must_use]
            let r = x.route(\"/gated\", get(h));
        ";
        let parsed = parse_route_source(src);
        assert!(parsed.is_empty(), "stacked-attr cfg gate not detected");
    }

    #[test]
    fn cfg_gate_does_not_leak_across_unrelated_statements() {
        // An unrelated `#[cfg]` two statements back must not gate a later
        // non-attributed route.
        let src = "
            #[cfg(feature = \"x\")]
            fn f() {}
            let r = x.route(\"/kept\", post(h));
        ";
        let parsed = parse_route_source(src);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].path, "/kept");
    }

    #[test]
    fn skips_routes_inside_nest_blocks() {
        // `.nest("/prefix", { ... .route("/sub", ...) ... })` — the
        // inner `.route("/sub")` is a sub-path under the nest prefix, not a
        // top-level route. Must be skipped; the full path
        // `/prefix/sub` is supposed to live in preserved_route_catalog().
        let src = r#"
            x.nest("/v1/decisions", {
                axum::Router::new()
                    .route("/", get(h))
                    .route("/:id", get(h))
                    .route("/cache", get(h))
            })
            .route("/v1/real", post(h))
        "#;
        let parsed = parse_route_source(src);
        assert_eq!(parsed.len(), 1, "got {:?}", parsed);
        assert_eq!(parsed[0].path, "/v1/real");
        assert_eq!(parsed[0].method, "POST");
    }

    #[test]
    fn does_not_double_count_with_state_parens() {
        let src = r#".route("/v1/z", get(handler).with_state(state.clone()))"#;
        let parsed = parse_route_source(src);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].method, "GET");
    }
}
