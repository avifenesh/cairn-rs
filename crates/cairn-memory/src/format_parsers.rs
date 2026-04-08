//! RFC 003 format parsers for the ingest pipeline.
//!
//! Structured parsers that go beyond the basic `normalize()` in `pipeline.rs`:
//!
//! - **MarkdownParser**: headings as metadata, clean text, code block extraction
//! - **HtmlParser**: tag stripping with structure hints, heading/list preservation
//! - **StructuredJsonParser**: key-value extraction as searchable text
//! - **TextNormalizer**: Unicode normalization, whitespace cleanup, encoding fixes
//! - **DocumentMetadata**: title, author, date, language extraction
//! - **ContentDeduplicator**: hash-based dedup with seen-set tracking
//!
//! All parsers are dependency-free (no regex crate).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

// ── Parsed output ────────────────────────────────────────────────────────────

/// Structured output from any format parser.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedDocument {
    /// Clean text content suitable for chunking.
    pub text: String,
    /// Extracted metadata (title, author, date, language, headings, etc.).
    pub metadata: DocumentMetadata,
    /// Code blocks extracted from the document (for code-aware retrieval).
    pub code_blocks: Vec<CodeBlock>,
    /// Structural hints (heading hierarchy, list items) for context.
    pub structure_hints: Vec<StructureHint>,
}

/// Metadata extracted from document content or headers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub language: Option<String>,
    /// All headings found in the document, in order.
    pub headings: Vec<Heading>,
    /// Arbitrary key-value pairs extracted from frontmatter or meta tags.
    pub extra: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureHint {
    Heading { level: u8, text: String },
    ListItem { text: String },
    Paragraph,
}

// ── Gap 1: Markdown Parser ───────────────────────────────────────────────────

pub struct MarkdownParser;

impl MarkdownParser {
    /// Parse markdown into clean text with metadata extraction.
    pub fn parse(input: &str) -> ParsedDocument {
        let mut text_lines: Vec<String> = Vec::new();
        let mut metadata = DocumentMetadata::default();
        let mut code_blocks = Vec::new();
        let mut structure_hints = Vec::new();
        let mut in_code_fence = false;
        let mut code_lang: Option<String> = None;
        let mut code_buf = String::new();
        let mut in_frontmatter = false;

        for (line_idx, line) in input.lines().enumerate() {
            let trimmed = line.trim();

            // YAML frontmatter: --- block at top of file.
            if line_idx == 0 && trimmed == "---" {
                in_frontmatter = true;
                continue;
            }
            if in_frontmatter {
                if trimmed == "---" {
                    in_frontmatter = false;
                    continue;
                }
                parse_frontmatter_line(trimmed, &mut metadata);
                continue;
            }

            // Code fences.
            if trimmed.starts_with("```") {
                if in_code_fence {
                    code_blocks.push(CodeBlock {
                        language: code_lang.take(),
                        content: code_buf.trim().to_owned(),
                    });
                    code_buf.clear();
                    in_code_fence = false;
                } else {
                    in_code_fence = true;
                    let lang = trimmed.trim_start_matches('`').trim();
                    code_lang = if lang.is_empty() {
                        None
                    } else {
                        Some(lang.to_owned())
                    };
                }
                continue;
            }

            if in_code_fence {
                if !code_buf.is_empty() {
                    code_buf.push('\n');
                }
                code_buf.push_str(line);
                // Also include code in the clean text.
                text_lines.push(line.to_owned());
                continue;
            }

            // Headings.
            if trimmed.starts_with('#') {
                let level = trimmed.chars().take_while(|c| *c == '#').count().min(6) as u8;
                let heading_text = trimmed[level as usize..].trim().to_owned();
                metadata.headings.push(Heading {
                    level,
                    text: heading_text.clone(),
                });
                structure_hints.push(StructureHint::Heading {
                    level,
                    text: heading_text.clone(),
                });
                // First h1 becomes the title if none set.
                if level == 1 && metadata.title.is_none() {
                    metadata.title = Some(heading_text.clone());
                }
                text_lines.push(heading_text);
                continue;
            }

            // List items.
            if trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || (trimmed.len() > 2
                    && trimmed.as_bytes()[0].is_ascii_digit()
                    && trimmed.contains(". "))
            {
                let item_text = strip_list_marker(trimmed);
                structure_hints.push(StructureHint::ListItem {
                    text: item_text.clone(),
                });
                text_lines.push(strip_md_inline(trimmed));
                continue;
            }

            // Regular text: strip inline formatting.
            let clean = strip_md_inline(trimmed);
            if clean.is_empty() {
                structure_hints.push(StructureHint::Paragraph);
            }
            text_lines.push(clean);
        }

        // Use frontmatter title if no h1 title found.
        if metadata.title.is_none() && !metadata.headings.is_empty() {
            metadata.title = Some(metadata.headings[0].text.clone());
        }

        ParsedDocument {
            text: collapse_blank_lines(&text_lines.join("\n")),
            metadata,
            code_blocks,
            structure_hints,
        }
    }
}

fn parse_frontmatter_line(line: &str, metadata: &mut DocumentMetadata) {
    if let Some((key, value)) = line.split_once(':') {
        let key = key.trim().to_lowercase();
        let value = value.trim().trim_matches('"').trim_matches('\'').to_owned();
        match key.as_str() {
            "title" => metadata.title = Some(value),
            "author" => metadata.author = Some(value),
            "date" => metadata.date = Some(value),
            "language" | "lang" => metadata.language = Some(value),
            _ => {
                metadata.extra.insert(key, value);
            }
        }
    }
}

fn strip_list_marker(line: &str) -> String {
    if line.starts_with("- ") || line.starts_with("* ") {
        line[2..].to_owned()
    } else if let Some(pos) = line.find(". ") {
        if line[..pos].chars().all(|c| c.is_ascii_digit()) {
            line[pos + 2..].to_owned()
        } else {
            line.to_owned()
        }
    } else {
        line.to_owned()
    }
}

/// Strip inline markdown formatting (bold, italic, links, images, code).
fn strip_md_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '!' if chars.peek() == Some(&'[') => {
                chars.next();
                let mut text = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    text.push(c);
                }
                if chars.peek() == Some(&'(') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&text);
            }
            '[' => {
                let mut text = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    text.push(c);
                }
                if chars.peek() == Some(&'(') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                out.push_str(&text);
            }
            '*' | '_' => {
                while chars.peek() == Some(&ch) {
                    chars.next();
                }
            }
            '`' => {
                while chars.peek() == Some(&'`') {
                    chars.next();
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

// ── Gap 2: HTML Parser ───────────────────────────────────────────────────────

pub struct HtmlParser;

impl HtmlParser {
    /// Parse HTML into clean text with structure hints and metadata.
    pub fn parse(input: &str) -> ParsedDocument {
        let mut text_lines: Vec<String> = Vec::new();
        let mut metadata = DocumentMetadata::default();
        let mut structure_hints = Vec::new();
        let mut chars = input.chars().peekable();
        let mut current_line = String::new();
        let mut in_head = false;
        let mut in_title = false;
        let mut in_script = false;
        let mut in_style = false;
        let mut title_buf = String::new();

        while let Some(ch) = chars.next() {
            if ch == '<' {
                let mut tag = String::new();
                for tc in chars.by_ref() {
                    if tc == '>' {
                        break;
                    }
                    tag.push(tc);
                }
                let tag_raw = tag.trim().to_owned();
                let tag_lower = tag_raw.to_lowercase();
                // Preserve the leading '/' for closing tags.
                let tag_name = if tag_lower.starts_with('/') {
                    let rest = tag_lower[1..]
                        .split(|c: char| c.is_whitespace())
                        .next()
                        .unwrap_or("");
                    format!("/{rest}")
                } else {
                    tag_lower
                        .split(|c: char| c.is_whitespace() || c == '/')
                        .next()
                        .unwrap_or("")
                        .to_owned()
                };
                let tag_name = tag_name.as_str();

                // Track head/title/script/style sections.
                match tag_name {
                    "head" => in_head = true,
                    "/head" => in_head = false,
                    "title" if in_head => in_title = true,
                    "/title" => {
                        if in_title {
                            metadata.title = Some(title_buf.trim().to_owned());
                            in_title = false;
                        }
                    }
                    "script" => in_script = true,
                    "/script" => in_script = false,
                    "style" => in_style = true,
                    "/style" => in_style = false,
                    _ => {}
                }

                if in_script || in_style {
                    continue;
                }

                // Extract meta tags (use raw tag to preserve content casing).
                if in_head && tag_name == "meta" {
                    parse_meta_tag(&tag_raw.to_lowercase(), &tag_raw, &mut metadata);
                }

                // Block-level elements emit line breaks.
                if matches!(
                    tag_name,
                    "br" | "p"
                        | "/p"
                        | "div"
                        | "/div"
                        | "li"
                        | "tr"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "/h1"
                        | "/h2"
                        | "/h3"
                        | "/h4"
                        | "/h5"
                        | "/h6"
                ) {
                    if !current_line.trim().is_empty() {
                        text_lines.push(current_line.trim().to_owned());
                    }
                    current_line.clear();
                }

                // Heading structure hints.
                if tag_name.starts_with('h')
                    && tag_name.len() == 2
                    && tag_name.as_bytes()[1].is_ascii_digit()
                {
                    let level = (tag_name.as_bytes()[1] - b'0').min(6);
                    // Capture heading text until closing tag.
                    let mut heading_text = String::new();
                    while let Some(&next_ch) = chars.peek() {
                        if next_ch == '<' {
                            break;
                        }
                        heading_text.push(chars.next().unwrap());
                    }
                    let heading_text = heading_text.trim().to_owned();
                    if !heading_text.is_empty() {
                        metadata.headings.push(Heading {
                            level,
                            text: heading_text.clone(),
                        });
                        structure_hints.push(StructureHint::Heading {
                            level,
                            text: heading_text.clone(),
                        });
                        if level == 1 && metadata.title.is_none() {
                            metadata.title = Some(heading_text.clone());
                        }
                        text_lines.push(heading_text);
                    }
                }

                // List items.
                if tag_name == "li" {
                    structure_hints.push(StructureHint::ListItem {
                        text: String::new(),
                    });
                }

                continue;
            }

            if ch == '&' {
                let mut entity = String::new();
                for ec in chars.by_ref() {
                    if ec == ';' {
                        break;
                    }
                    entity.push(ec);
                    if entity.len() > 8 {
                        break;
                    }
                }
                let decoded = match entity.as_str() {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    "nbsp" => ' ',
                    _ => {
                        current_line.push('&');
                        current_line.push_str(&entity);
                        continue;
                    }
                };
                if in_title {
                    title_buf.push(decoded);
                } else if !in_script && !in_style && !in_head {
                    current_line.push(decoded);
                }
                continue;
            }

            if in_title {
                title_buf.push(ch);
            } else if !in_script && !in_style && !in_head {
                current_line.push(ch);
            }
        }

        if !current_line.trim().is_empty() {
            text_lines.push(current_line.trim().to_owned());
        }

        ParsedDocument {
            text: collapse_blank_lines(&text_lines.join("\n")),
            metadata,
            code_blocks: vec![],
            structure_hints,
        }
    }
}

fn parse_meta_tag(tag_lower: &str, tag_raw: &str, metadata: &mut DocumentMetadata) {
    let name = extract_attr(tag_lower, "name");
    // Extract content from the raw (case-preserved) tag.
    let content = extract_attr(tag_raw, "content").or_else(|| extract_attr(tag_raw, "Content"));
    if let (Some(name), Some(content)) = (name, content) {
        match name.as_str() {
            "author" => metadata.author = Some(content),
            "date" => metadata.date = Some(content),
            "language" | "lang" => metadata.language = Some(content),
            "description" => {
                metadata.extra.insert("description".into(), content);
            }
            _ => {}
        }
    }
}

fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
    let search = format!("{attr_name}=");
    let idx = tag.find(&search)?;
    let rest = &tag[idx + search.len()..];
    let rest = rest.trim_start();
    if rest.starts_with('"') {
        let end = rest[1..].find('"')?;
        Some(rest[1..1 + end].to_owned())
    } else if rest.starts_with('\'') {
        let end = rest[1..].find('\'')?;
        Some(rest[1..1 + end].to_owned())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        Some(rest[..end].to_owned())
    }
}

// ── Gap 3: Structured JSON Parser ────────────────────────────────────────────

pub struct StructuredJsonParser;

impl StructuredJsonParser {
    /// Parse a JSON document into searchable text with key-value extraction.
    pub fn parse(input: &str) -> ParsedDocument {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
            return ParsedDocument {
                text: input.to_owned(),
                metadata: DocumentMetadata::default(),
                code_blocks: vec![],
                structure_hints: vec![],
            };
        };

        let mut metadata = DocumentMetadata::default();
        let mut text_parts = Vec::new();

        // Extract top-level metadata hints.
        if let Some(obj) = value.as_object() {
            if let Some(title) = obj.get("title").and_then(|v| v.as_str()) {
                metadata.title = Some(title.to_owned());
            }
            if let Some(author) = obj.get("author").and_then(|v| v.as_str()) {
                metadata.author = Some(author.to_owned());
            }
            if let Some(date) = obj.get("date").and_then(|v| v.as_str()) {
                metadata.date = Some(date.to_owned());
            }
            if let Some(lang) = obj.get("language").and_then(|v| v.as_str()) {
                metadata.language = Some(lang.to_owned());
            }
        }

        // Extract all key-value pairs as "key: value" text.
        collect_kv_pairs(&value, "", &mut text_parts);

        ParsedDocument {
            text: text_parts.join("\n"),
            metadata,
            code_blocks: vec![],
            structure_hints: vec![],
        }
    }
}

fn collect_kv_pairs(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match val {
                    serde_json::Value::String(s) if !s.trim().is_empty() => {
                        out.push(format!("{path}: {s}"));
                    }
                    serde_json::Value::Number(n) => {
                        out.push(format!("{path}: {n}"));
                    }
                    serde_json::Value::Bool(b) => {
                        out.push(format!("{path}: {b}"));
                    }
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        collect_kv_pairs(val, &path, out);
                    }
                    _ => {}
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                let path = format!("{prefix}[{i}]");
                collect_kv_pairs(item, &path, out);
            }
        }
        serde_json::Value::String(s) if !s.trim().is_empty() => {
            out.push(if prefix.is_empty() {
                s.clone()
            } else {
                format!("{prefix}: {s}")
            });
        }
        _ => {}
    }
}

// ── Gap 4: Text Normalizer ───────────────────────────────────────────────────

pub struct TextNormalizer;

impl TextNormalizer {
    /// Normalize text: Unicode NFC-like cleanup, whitespace normalization,
    /// control character removal, encoding fixes.
    pub fn normalize(input: &str) -> String {
        let mut out = String::with_capacity(input.len());

        for ch in input.chars() {
            // Remove control characters except newlines and tabs.
            if ch.is_control() && ch != '\n' && ch != '\t' {
                continue;
            }
            // Replace non-breaking spaces and other Unicode spaces with regular space.
            if ch == '\u{00A0}'   // NBSP
                || ch == '\u{2000}' // EN QUAD
                || ch == '\u{2001}' // EM QUAD
                || ch == '\u{2002}' // EN SPACE
                || ch == '\u{2003}' // EM SPACE
                || ch == '\u{200A}' // HAIR SPACE
                || ch == '\u{202F}' // NARROW NO-BREAK SPACE
                || ch == '\u{205F}' // MEDIUM MATHEMATICAL SPACE
                || ch == '\u{3000}'
            // IDEOGRAPHIC SPACE
            {
                out.push(' ');
                continue;
            }
            // Replace smart quotes with ASCII quotes.
            if ch == '\u{201C}' || ch == '\u{201D}' {
                out.push('"');
                continue;
            }
            if ch == '\u{2018}' || ch == '\u{2019}' {
                out.push('\'');
                continue;
            }
            // Replace em/en dashes with hyphens.
            if ch == '\u{2013}' || ch == '\u{2014}' {
                out.push('-');
                continue;
            }
            // Replace ellipsis with three dots.
            if ch == '\u{2026}' {
                out.push_str("...");
                continue;
            }
            // Replace tab with space.
            if ch == '\t' {
                out.push(' ');
                continue;
            }
            out.push(ch);
        }

        // Collapse multiple spaces into one (preserve newlines).
        collapse_spaces(&out)
    }
}

fn collapse_spaces(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;
    for ch in text.chars() {
        if ch == ' ' {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            prev_space = false;
            result.push(ch);
        }
    }
    result
}

// ── Gap 5: Metadata Extraction ───────────────────────────────────────────────

/// Extract metadata from raw document content by sniffing patterns.
///
/// Works across formats — looks for common patterns like frontmatter,
/// HTML meta tags, and JSON fields.
pub fn extract_metadata(content: &str) -> DocumentMetadata {
    let mut metadata = DocumentMetadata::default();

    // Try YAML frontmatter.
    if content.starts_with("---\n") || content.starts_with("---\r\n") {
        let end = content[4..].find("\n---").map(|i| i + 4);
        if let Some(end) = end {
            let frontmatter = &content[4..end];
            for line in frontmatter.lines() {
                parse_frontmatter_line(line.trim(), &mut metadata);
            }
        }
    }

    // Try JSON metadata.
    if content.trim_start().starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(obj) = val.as_object() {
                if let Some(t) = obj.get("title").and_then(|v| v.as_str()) {
                    metadata.title = metadata.title.or(Some(t.to_owned()));
                }
                if let Some(a) = obj.get("author").and_then(|v| v.as_str()) {
                    metadata.author = metadata.author.or(Some(a.to_owned()));
                }
                if let Some(d) = obj.get("date").and_then(|v| v.as_str()) {
                    metadata.date = metadata.date.or(Some(d.to_owned()));
                }
            }
        }
    }

    // Try first heading as title.
    if metadata.title.is_none() {
        for line in content.lines().take(10) {
            let trimmed = line.trim();
            if trimmed.starts_with("# ") {
                metadata.title = Some(trimmed[2..].trim().to_owned());
                break;
            }
            // HTML title.
            if let Some(start) = trimmed.find("<title>") {
                if let Some(end) = trimmed.find("</title>") {
                    let title = &trimmed[start + 7..end];
                    metadata.title = Some(title.trim().to_owned());
                    break;
                }
            }
        }
    }

    // Detect language from common patterns.
    if metadata.language.is_none() {
        if let Some(lang) = detect_language_hint(content) {
            metadata.language = Some(lang);
        }
    }

    metadata
}

fn detect_language_hint(content: &str) -> Option<String> {
    let first_500: String = content.chars().take(500).collect();
    let lower = first_500.to_lowercase();

    if lower.contains("lang=\"en\"") || lower.contains("language: en") {
        return Some("en".into());
    }
    if lower.contains("lang=\"de\"") || lower.contains("language: de") {
        return Some("de".into());
    }
    if lower.contains("lang=\"fr\"") || lower.contains("language: fr") {
        return Some("fr".into());
    }
    if lower.contains("lang=\"es\"") || lower.contains("language: es") {
        return Some("es".into());
    }
    if lower.contains("lang=\"ja\"") || lower.contains("language: ja") {
        return Some("ja".into());
    }

    None
}

// ── Gap 6: Content Deduplicator ──────────────────────────────────────────────

/// Hash-based content deduplicator.
///
/// Tracks seen content hashes and reports whether a document has already
/// been ingested. Uses the same hash function as `pipeline::compute_content_hash`.
pub struct ContentDeduplicator {
    seen: HashSet<String>,
}

impl ContentDeduplicator {
    pub fn new() -> Self {
        Self {
            seen: HashSet::new(),
        }
    }

    /// Pre-populate with existing hashes (e.g. from the store).
    pub fn with_existing(existing: HashSet<String>) -> Self {
        Self { seen: existing }
    }

    /// Check if content has been seen before. Returns `true` if it's a duplicate.
    pub fn is_duplicate(&self, content: &str) -> bool {
        let hash = content_hash(content);
        self.seen.contains(&hash)
    }

    /// Record content as seen. Returns `true` if it was new (not a duplicate).
    pub fn record(&mut self, content: &str) -> bool {
        let hash = content_hash(content);
        self.seen.insert(hash)
    }

    /// Check and record in one step. Returns `true` if the content is new.
    pub fn check_and_record(&mut self, content: &str) -> bool {
        let hash = content_hash(content);
        self.seen.insert(hash) // insert returns true if new
    }

    pub fn seen_count(&self) -> usize {
        self.seen.len()
    }
}

impl Default for ContentDeduplicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a stable content hash for dedup (same as pipeline::compute_content_hash).
pub fn content_hash(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn collapse_blank_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_blank = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank && !result.is_empty() {
                result.push('\n');
            }
            prev_blank = true;
        } else {
            if prev_blank && !result.is_empty() {
                result.push('\n');
            }
            result.push_str(trimmed);
            result.push('\n');
            prev_blank = false;
        }
    }
    result.trim().to_owned()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Markdown parser ──────────────────────────────────────────────────

    #[test]
    fn markdown_extracts_headings() {
        let doc = MarkdownParser::parse("# Title\n\nSome text.\n\n## Section\n\nMore text.");
        assert_eq!(doc.metadata.title.as_deref(), Some("Title"));
        assert_eq!(doc.metadata.headings.len(), 2);
        assert_eq!(doc.metadata.headings[0].level, 1);
        assert_eq!(doc.metadata.headings[1].text, "Section");
    }

    #[test]
    fn markdown_extracts_code_blocks() {
        let md = "# Guide\n\n```rust\nfn main() {}\n```\n\nDone.";
        let doc = MarkdownParser::parse(md);
        assert_eq!(doc.code_blocks.len(), 1);
        assert_eq!(doc.code_blocks[0].language.as_deref(), Some("rust"));
        assert!(doc.code_blocks[0].content.contains("fn main()"));
    }

    #[test]
    fn markdown_strips_inline_formatting() {
        let md = "This is **bold** and *italic* and `code`.";
        let doc = MarkdownParser::parse(md);
        assert!(doc.text.contains("bold"));
        assert!(doc.text.contains("italic"));
        assert!(doc.text.contains("code"));
        assert!(!doc.text.contains("**"));
        assert!(!doc.text.contains("*italic*"));
    }

    #[test]
    fn markdown_extracts_links() {
        let md = "Click [here](http://example.com) for info.";
        let doc = MarkdownParser::parse(md);
        assert!(doc.text.contains("here"));
        assert!(!doc.text.contains("http://example.com"));
    }

    #[test]
    fn markdown_frontmatter_extracts_metadata() {
        let md = "---\ntitle: My Doc\nauthor: Jane\ndate: 2026-01-01\nlanguage: en\n---\n\n# Content\n\nText.";
        let doc = MarkdownParser::parse(md);
        assert_eq!(doc.metadata.title.as_deref(), Some("My Doc"));
        assert_eq!(doc.metadata.author.as_deref(), Some("Jane"));
        assert_eq!(doc.metadata.date.as_deref(), Some("2026-01-01"));
        assert_eq!(doc.metadata.language.as_deref(), Some("en"));
    }

    #[test]
    fn markdown_list_items_captured() {
        let md = "- Item one\n- Item two\n* Item three";
        let doc = MarkdownParser::parse(md);
        let items: Vec<_> = doc
            .structure_hints
            .iter()
            .filter(|h| matches!(h, StructureHint::ListItem { .. }))
            .collect();
        assert_eq!(items.len(), 3);
    }

    // ── HTML parser ──────────────────────────────────────────────────────

    #[test]
    fn html_extracts_text() {
        let html = "<p>Hello</p><p>World</p>";
        let doc = HtmlParser::parse(html);
        assert!(doc.text.contains("Hello"));
        assert!(doc.text.contains("World"));
        assert!(!doc.text.contains("<p>"));
    }

    #[test]
    fn html_extracts_title() {
        let html = "<html><head><title>My Page</title></head><body><p>Content</p></body></html>";
        let doc = HtmlParser::parse(html);
        assert_eq!(doc.metadata.title.as_deref(), Some("My Page"));
    }

    #[test]
    fn html_extracts_headings() {
        let html = "<h1>Main Title</h1><h2>Section</h2><p>Text</p>";
        let doc = HtmlParser::parse(html);
        assert_eq!(doc.metadata.headings.len(), 2);
        assert_eq!(doc.metadata.headings[0].text, "Main Title");
        assert_eq!(doc.metadata.headings[0].level, 1);
    }

    #[test]
    fn html_decodes_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D</p>";
        let doc = HtmlParser::parse(html);
        assert!(doc.text.contains("A & B < C > D"));
    }

    #[test]
    fn html_strips_script_and_style() {
        let html = "<p>Hello</p><script>alert('xss')</script><style>body{}</style><p>World</p>";
        let doc = HtmlParser::parse(html);
        assert!(doc.text.contains("Hello"));
        assert!(doc.text.contains("World"));
        assert!(!doc.text.contains("alert"));
        assert!(!doc.text.contains("body{}"));
    }

    #[test]
    fn html_extracts_meta_author() {
        let html =
            "<html><head><meta name=\"author\" content=\"Alice\"></head><body>Text</body></html>";
        let doc = HtmlParser::parse(html);
        assert_eq!(doc.metadata.author.as_deref(), Some("Alice"));
    }

    // ── Structured JSON parser ───────────────────────────────────────────

    #[test]
    fn json_extracts_kv_pairs() {
        let json = r#"{"title": "Report", "summary": "Q1 results", "metrics": {"revenue": 100}}"#;
        let doc = StructuredJsonParser::parse(json);
        assert!(doc.text.contains("title: Report"));
        assert!(doc.text.contains("summary: Q1 results"));
        assert!(doc.text.contains("metrics.revenue: 100"));
    }

    #[test]
    fn json_extracts_metadata() {
        let json = r#"{"title": "My Doc", "author": "Bob", "date": "2026-04-06"}"#;
        let doc = StructuredJsonParser::parse(json);
        assert_eq!(doc.metadata.title.as_deref(), Some("My Doc"));
        assert_eq!(doc.metadata.author.as_deref(), Some("Bob"));
    }

    #[test]
    fn json_handles_arrays() {
        let json = r#"{"items": ["alpha", "beta", "gamma"]}"#;
        let doc = StructuredJsonParser::parse(json);
        assert!(doc.text.contains("alpha"));
        assert!(doc.text.contains("beta"));
        assert!(doc.text.contains("gamma"));
    }

    #[test]
    fn json_invalid_falls_back() {
        let doc = StructuredJsonParser::parse("not json");
        assert_eq!(doc.text, "not json");
    }

    // ── Text normalizer ──────────────────────────────────────────────────

    #[test]
    fn normalizer_removes_control_chars() {
        let input = "Hello\x00\x01World\x07";
        let result = TextNormalizer::normalize(input);
        assert_eq!(result, "HelloWorld");
    }

    #[test]
    fn normalizer_replaces_smart_quotes() {
        let input = "\u{201C}Hello\u{201D} and \u{2018}world\u{2019}";
        let result = TextNormalizer::normalize(input);
        assert_eq!(result, "\"Hello\" and 'world'");
    }

    #[test]
    fn normalizer_replaces_unicode_spaces() {
        let input = "Hello\u{00A0}World\u{2003}End";
        let result = TextNormalizer::normalize(input);
        assert_eq!(result, "Hello World End");
    }

    #[test]
    fn normalizer_replaces_dashes_and_ellipsis() {
        let input = "A\u{2014}B and C\u{2026}D";
        let result = TextNormalizer::normalize(input);
        assert_eq!(result, "A-B and C...D");
    }

    #[test]
    fn normalizer_collapses_multiple_spaces() {
        let input = "Hello    World   End";
        let result = TextNormalizer::normalize(input);
        assert_eq!(result, "Hello World End");
    }

    #[test]
    fn normalizer_preserves_newlines() {
        let input = "Line one\nLine two";
        let result = TextNormalizer::normalize(input);
        assert_eq!(result, "Line one\nLine two");
    }

    // ── Metadata extraction ──────────────────────────────────────────────

    #[test]
    fn extract_metadata_from_frontmatter() {
        let content = "---\ntitle: My Doc\nauthor: Alice\n---\n\nContent here.";
        let meta = extract_metadata(content);
        assert_eq!(meta.title.as_deref(), Some("My Doc"));
        assert_eq!(meta.author.as_deref(), Some("Alice"));
    }

    #[test]
    fn extract_metadata_from_heading() {
        let content = "# My Title\n\nSome content.";
        let meta = extract_metadata(content);
        assert_eq!(meta.title.as_deref(), Some("My Title"));
    }

    #[test]
    fn extract_metadata_from_json() {
        let content = r#"{"title": "JSON Doc", "author": "Bob"}"#;
        let meta = extract_metadata(content);
        assert_eq!(meta.title.as_deref(), Some("JSON Doc"));
        assert_eq!(meta.author.as_deref(), Some("Bob"));
    }

    #[test]
    fn detect_language_from_html_attr() {
        let content = "<html lang=\"en\"><body>Content</body></html>";
        let meta = extract_metadata(content);
        assert_eq!(meta.language.as_deref(), Some("en"));
    }

    // ── Content deduplicator ─────────────────────────────────────────────

    #[test]
    fn dedup_detects_duplicates() {
        let mut dedup = ContentDeduplicator::new();
        assert!(dedup.check_and_record("Hello world"));
        assert!(!dedup.check_and_record("Hello world")); // duplicate
        assert!(dedup.check_and_record("Different text"));
    }

    #[test]
    fn dedup_pre_populated() {
        let hash = content_hash("existing content");
        let existing: HashSet<String> = [hash].into();
        let dedup = ContentDeduplicator::with_existing(existing);
        assert!(dedup.is_duplicate("existing content"));
        assert!(!dedup.is_duplicate("new content"));
    }

    #[test]
    fn dedup_count() {
        let mut dedup = ContentDeduplicator::new();
        dedup.record("a");
        dedup.record("b");
        dedup.record("a"); // duplicate, count stays 2
        assert_eq!(dedup.seen_count(), 2);
    }

    #[test]
    fn content_hash_is_stable() {
        let h1 = content_hash("test input");
        let h2 = content_hash("test input");
        assert_eq!(h1, h2);
        assert_ne!(h1, content_hash("different input"));
    }
}
