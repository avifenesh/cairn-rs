//! Entity extraction pipeline for knowledge chunks (GAP-009).
//!
//! Extracts named entities (persons, organizations, locations, facts) from
//! text chunks during ingest. The baseline implementation uses heuristic
//! pattern matching with no external dependencies; an LLM-backed extractor
//! can be plugged in via the `EntityExtractor` trait.
//!
//! # Usage in the ingest pipeline
//! After chunking, call `EntityExtractor::extract()` for each chunk and
//! store the result in `ChunkRecord::entities`.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

// ── Request / Result ───────────────────────────────────────────────────────

/// Request to extract named entities from a text chunk.
#[derive(Clone, Debug)]
pub struct EntityExtractionRequest {
    /// The text to extract entities from.
    pub text: String,
    /// Project scope for tenant isolation (passed to LLM-backed extractors).
    pub project: ProjectKey,
    /// Whether to extract person names.
    pub extract_persons: bool,
    /// Whether to extract organization names.
    pub extract_orgs: bool,
    /// Whether to extract location names.
    pub extract_locations: bool,
    /// Whether to extract standalone facts (declarative sentences).
    pub extract_facts: bool,
}

impl EntityExtractionRequest {
    /// Create a request that extracts all entity types.
    pub fn all(text: String, project: ProjectKey) -> Self {
        Self {
            text,
            project,
            extract_persons: true,
            extract_orgs: true,
            extract_locations: true,
            extract_facts: true,
        }
    }
}

/// Result of entity extraction for one text chunk.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EntityExtractionResult {
    /// Person names found in the text (e.g. `"Alan Turing"`, `"Marie Curie"`).
    pub persons: Vec<String>,
    /// Organization names found (e.g. `"Anthropic"`, `"Google DeepMind"`).
    pub orgs: Vec<String>,
    /// Location names found (e.g. `"San Francisco"`, `"United Kingdom"`).
    pub locations: Vec<String>,
    /// Short declarative fact sentences extracted from the text.
    pub facts: Vec<String>,
    /// SHA-style content hash of the source text (for provenance).
    pub source_text_hash: String,
}

impl EntityExtractionResult {
    /// Collect all extracted entities into a flat deduplicated list.
    ///
    /// Returns persons + orgs + locations in insertion order, deduplicated.
    /// Used to populate `ChunkRecord::entities`.
    pub fn all_entities(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for e in self
            .persons
            .iter()
            .chain(self.orgs.iter())
            .chain(self.locations.iter())
        {
            let key = e.to_lowercase();
            if seen.insert(key) {
                out.push(e.clone());
            }
        }
        out
    }

    /// Whether any entities or facts were extracted.
    pub fn is_empty(&self) -> bool {
        self.persons.is_empty()
            && self.orgs.is_empty()
            && self.locations.is_empty()
            && self.facts.is_empty()
    }
}

// ── Trait ─────────────────────────────────────────────────────────────────

/// Entity extractor boundary.
///
/// The baseline `RegexEntityExtractor` uses heuristic pattern matching.
/// An LLM-backed extractor can implement this trait for higher recall.
pub trait EntityExtractor: Send + Sync {
    fn extract(&self, request: &EntityExtractionRequest) -> EntityExtractionResult;
}

// ── RegexEntityExtractor ──────────────────────────────────────────────────

/// Baseline heuristic entity extractor — no external dependencies.
///
/// Uses character-level scanning and simple token patterns:
/// - **Persons**: consecutive Title Case tokens (2-4 words) that don't match
///   known org suffixes, preceded by title honorifics or isolation cues.
/// - **Orgs**: tokens ending in known corporate/institutional suffixes, or
///   sequences of Title Case words immediately before such suffixes.
/// - **Locations**: Title Case words following spatial prepositions
///   (`in`, `at`, `from`, `to`, `near`, `of`) or matching a
///   curated country/city list.
/// - **Facts**: short declarative sentences (subject + "is/was/are/were/has/can/will").
///
/// Precision is moderate; recall is intentionally conservative to avoid noise.
pub struct RegexEntityExtractor;

impl RegexEntityExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RegexEntityExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityExtractor for RegexEntityExtractor {
    fn extract(&self, request: &EntityExtractionRequest) -> EntityExtractionResult {
        let hash = content_hash(&request.text);
        let mut result = EntityExtractionResult {
            source_text_hash: hash,
            ..Default::default()
        };

        if request.extract_persons || request.extract_orgs || request.extract_locations {
            let tokens = tokenize(&request.text);
            let spans = extract_capitalized_spans(&tokens);

            for span in &spans {
                let text = span.join(" ");
                if request.extract_orgs && is_org_span(span) {
                    result.orgs.push(text);
                } else if request.extract_persons && is_person_span(span) {
                    result.persons.push(text);
                }
            }

            if request.extract_locations {
                let locs = extract_locations(&request.text, &tokens);
                result.locations = locs;
            }
        }

        if request.extract_facts {
            result.facts = extract_facts(&request.text);
        }

        // Deduplicate each list.
        dedup_preserve_order(&mut result.persons);
        dedup_preserve_order(&mut result.orgs);
        dedup_preserve_order(&mut result.locations);
        dedup_preserve_order(&mut result.facts);

        result
    }
}

// ── Tokenization ──────────────────────────────────────────────────────────

/// Common words that start sentences but are not proper nouns.
const STOPWORDS: &[&str] = &[
    "The", "A", "An", "This", "That", "These", "Those", "It", "Its",
    "He", "She", "They", "We", "I", "You", "My", "Your", "Our", "Their",
    "His", "Her", "Its", "One", "Some", "Any", "All", "Each", "Every",
    "In", "At", "On", "For", "To", "By", "From", "With", "Of", "And",
    "But", "Or", "Not", "No", "So", "As", "If", "When", "Where", "While",
    "Although", "Because", "Since", "After", "Before", "During", "Among",
    "Between", "Into", "Through", "About", "Above", "Below", "Against",
    "Also", "Both", "Just", "Only", "Even", "Then", "Than", "There",
    "Here", "Now", "Still", "However", "Therefore", "Thus",
];

/// A token: stripped text.
#[derive(Debug, Clone)]
struct Token {
    text: String,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for word in text.split_whitespace() {
        // Strip leading punctuation.
        let clean = word.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '"' | '(' | '['));
        // Strip trailing punctuation.
        let clean = clean.trim_end_matches(|c: char| matches!(c, '.' | '!' | '?' | ',' | ';' | ':' | '"' | ')' | ']' | '\''));
        if !clean.is_empty() {
            tokens.push(Token { text: clean.to_owned() });
        }
    }
    tokens
}

/// Returns true if the token starts with uppercase and has at least one lowercase letter.
/// This accepts "London", "Turing", "OpenAI", "DeepMind" but not "USA", "AI", "HTTP".
fn is_title_case(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else { return false };
    if !first.is_uppercase() {
        return false;
    }
    // Must have at least one lowercase letter (rejects pure all-caps like "LLC", "NATO")
    word.chars().any(|c| c.is_lowercase())
}

/// Returns true if the token is in the org suffix list (handles all-caps like "LLC").
fn is_org_suffix_token(word: &str) -> bool {
    ORG_SUFFIXES.contains(&word)
}

/// Returns true if the word is a stopword that should not start a proper noun span.
fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

/// Extract spans of consecutive proper-noun tokens.
///
/// A proper noun token is either:
/// - Title case and not a stopword, or
/// - An org suffix token (like "LLC", "Inc") that extends an existing span
///
/// Single-token spans are only kept for known org names.
fn extract_capitalized_spans(tokens: &[Token]) -> Vec<Vec<String>> {
    let mut spans: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();

    for token in tokens {
        let proper = is_title_case(&token.text) && !is_stopword(&token.text);
        let org_suffix = is_org_suffix_token(&token.text);

        if proper || (org_suffix && !current.is_empty()) {
            current.push(token.text.clone());
        } else {
            flush_span(&mut current, &mut spans);
            current.clear();
        }
    }
    flush_span(&mut current, &mut spans);

    // Also capture honorific-led names that may have been broken by the above loop.
    let honorifics = [
        "Mr.", "Mrs.", "Ms.", "Dr.", "Prof.", "Sir", "Dame", "Lord", "Lady",
        "President", "Director", "CEO", "CTO", "CFO", "VP",
    ];
    for (i, token) in tokens.iter().enumerate() {
        if honorifics.contains(&token.text.as_str()) {
            let mut span = vec![token.text.clone()];
            for j in (i + 1)..(i + 4).min(tokens.len()) {
                if is_title_case(&tokens[j].text) && !is_stopword(&tokens[j].text) {
                    span.push(tokens[j].text.clone());
                } else {
                    break;
                }
            }
            if span.len() >= 2 {
                spans.push(span);
            }
        }
    }

    spans
}

fn flush_span(current: &mut Vec<String>, spans: &mut Vec<Vec<String>>) {
    if current.len() >= 2 {
        spans.push(current.clone());
    } else if current.len() == 1 {
        // Single-token spans only kept if it's a known org name.
        if ORG_PREFIXES.contains(&current[0].as_str()) {
            spans.push(current.clone());
        }
    }
}

// ── Org suffix detection ───────────────────────────────────────────────────

const ORG_SUFFIXES: &[&str] = &[
    "Inc", "Inc.", "Corp", "Corp.", "Ltd", "Ltd.", "LLC", "LLP",
    "Co", "Co.", "Company", "Companies", "Group", "Groups",
    "Foundation", "Institute", "University", "College", "School",
    "Association", "Society", "Council", "Agency",
    "Labs", "Lab", "Technologies", "Technology", "Systems", "Solutions",
    "Services", "Ventures", "Capital", "Partners", "Consulting",
    "Network", "Networks", "Media", "Global", "International",
    "Incorporated", "Limited",
];

const ORG_PREFIXES: &[&str] = &[
    "Google", "Microsoft", "Apple", "Amazon", "Meta", "OpenAI", "Anthropic",
    "Nvidia", "Intel", "IBM", "Oracle", "Salesforce", "Adobe", "Netflix",
    "Twitter", "LinkedIn", "Facebook", "Instagram", "Tesla", "SpaceX",
    "Stripe", "Uber", "Airbnb", "Databricks", "Snowflake", "Palantir",
    "DeepMind", "DeepSeek", "Mistral", "Cohere", "Stability", "Midjourney",
    "Stanford", "MIT", "Harvard", "Oxford", "Cambridge", "Berkeley",
    "NASA", "CERN", "WHO", "UNESCO", "UNICEF", "NATO",
];

fn is_org_span(span: &[String]) -> bool {
    // Single known org name (e.g. "OpenAI", "Google").
    if span.len() == 1 && ORG_PREFIXES.contains(&span[0].as_str()) {
        return true;
    }
    // Multi-word: last token is an org suffix (e.g. "Anthropic Inc", "Google LLC").
    if let Some(last) = span.last() {
        if ORG_SUFFIXES.contains(&last.as_str()) || is_org_suffix_token(last) {
            return true;
        }
    }
    // Multi-word starting with a known org prefix (e.g. "Google DeepMind").
    if let Some(first) = span.first() {
        if ORG_PREFIXES.contains(&first.as_str()) {
            return true;
        }
    }
    // Any token is a known org prefix.
    span.iter().any(|t| ORG_PREFIXES.contains(&t.as_str()))
}

fn is_person_span(span: &[String]) -> bool {
    // Single-token spans were already filtered to known orgs in flush_span.
    if span.len() < 2 {
        return false;
    }
    // Reject if it looks like an org.
    if is_org_span(span) {
        return false;
    }
    // Accept 2-4 Title Case words as a person name.
    span.len() <= 4
}

// ── Location extraction ───────────────────────────────────────────────────

const SPATIAL_PREPOSITIONS: &[&str] = &["in", "at", "from", "to", "near", "of", "outside", "inside", "across"];

const KNOWN_LOCATIONS: &[&str] = &[
    // Countries
    "Afghanistan", "Albania", "Algeria", "Argentina", "Australia", "Austria",
    "Belgium", "Brazil", "Canada", "Chile", "China", "Colombia", "Croatia",
    "Czech", "Denmark", "Egypt", "Finland", "France", "Germany", "Greece",
    "Hungary", "India", "Indonesia", "Iran", "Iraq", "Ireland", "Israel",
    "Italy", "Japan", "Jordan", "Kenya", "Korea", "Malaysia", "Mexico",
    "Morocco", "Netherlands", "NewZealand", "Nigeria", "Norway", "Pakistan",
    "Peru", "Philippines", "Poland", "Portugal", "Romania", "Russia",
    "SaudiArabia", "Singapore", "SouthAfrica", "Spain", "Sweden", "Switzerland",
    "Taiwan", "Thailand", "Turkey", "Ukraine", "UnitedKingdom", "UK", "USA",
    "UnitedStates", "Vietnam",
    // Major cities
    "Amsterdam", "Athens", "Bangkok", "Barcelona", "Beijing", "Berlin",
    "Brussels", "Buenos", "Cairo", "Chicago", "Dubai", "Geneva", "Helsinki",
    "HongKong", "Istanbul", "Jakarta", "Johannesburg", "Kyoto", "Lagos",
    "Lima", "Lisbon", "London", "LosAngeles", "Madrid", "Melbourne",
    "MexicoCity", "Miami", "Milan", "Moscow", "Mumbai", "Munich", "Nairobi",
    "NewYork", "Oslo", "Paris", "Prague", "Rome", "SanFrancisco", "Santiago",
    "Seoul", "Shanghai", "Singapore", "Stockholm", "Sydney", "Taipei",
    "Tehran", "Tokyo", "Toronto", "Vancouver", "Vienna", "Warsaw", "Zurich",
    // US States
    "Alabama", "Alaska", "Arizona", "Arkansas", "California", "Colorado",
    "Connecticut", "Delaware", "Florida", "Georgia", "Hawaii", "Idaho",
    "Illinois", "Indiana", "Iowa", "Kansas", "Kentucky", "Louisiana",
    "Maine", "Maryland", "Massachusetts", "Michigan", "Minnesota", "Missouri",
    "Montana", "Nebraska", "Nevada", "NewHampshire", "NewJersey", "NewMexico",
    "NewYork", "NorthCarolina", "Ohio", "Oklahoma", "Oregon", "Pennsylvania",
    "Tennessee", "Texas", "Utah", "Vermont", "Virginia", "Washington",
    "Wisconsin",
];

fn extract_locations(text: &str, tokens: &[Token]) -> Vec<String> {
    let mut locations = Vec::new();

    // 1. Capture Title Case words following spatial prepositions.
    for (i, token) in tokens.iter().enumerate() {
        let lower = token.text.to_lowercase();
        if SPATIAL_PREPOSITIONS.contains(&lower.as_str()) {
            // Capture up to 3 following Title Case tokens as a location phrase.
            let mut loc_tokens = Vec::new();
            for j in (i + 1)..(i + 4).min(tokens.len()) {
                if is_title_case(&tokens[j].text) {
                    loc_tokens.push(tokens[j].text.clone());
                } else {
                    break;
                }
            }
            if !loc_tokens.is_empty() {
                locations.push(loc_tokens.join(" "));
            }
        }
    }

    // 2. Match known location names (single or compound) in the text.
    let text_lower = text.to_lowercase();
    for loc in KNOWN_LOCATIONS {
        // Simple substring check: convert known location to space-separated form.
        let normalized = camel_to_spaces(loc);
        if text_lower.contains(&normalized.to_lowercase()) {
            locations.push(normalized);
        }
    }

    locations
}

/// Convert CamelCase location names to spaced form for matching.
fn camel_to_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

// ── Fact extraction ───────────────────────────────────────────────────────

const FACT_VERBS: &[&str] = &[
    " is ", " was ", " are ", " were ", " has ", " have ", " had ",
    " can ", " will ", " would ", " does ", " did ",
    " became ", " becomes ", " remains ", " remained ",
    " contains ", " includes ", " requires ", " produces ",
];

fn extract_facts(text: &str) -> Vec<String> {
    let mut facts = Vec::new();

    for sentence in split_sentences(text) {
        let s = sentence.trim();
        if s.len() < 20 || s.len() > 300 {
            continue;
        }
        let lower = s.to_lowercase();
        let has_fact_verb = FACT_VERBS.iter().any(|v| lower.contains(v));
        if has_fact_verb {
            // Trim trailing punctuation for clean storage.
            let clean = s.trim_end_matches(|c: char| ".!?,;:".contains(c)).trim().to_owned();
            if !clean.is_empty() {
                facts.push(clean);
            }
        }
    }

    facts
}

fn split_sentences(text: &str) -> Vec<&str> {
    // Simple sentence splitter on `. `, `! `, `? ` boundaries.
    let mut sentences = Vec::new();
    let mut start = 0;

    let bytes = text.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        let ch = bytes[i] as char;
        let next = bytes[i + 1] as char;
        if matches!(ch, '.' | '!' | '?') && (next == ' ' || next == '\n') {
            let sentence = text[start..=i].trim();
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            start = i + 2;
        }
    }
    // Final sentence (may not end with punctuation).
    let remainder = text[start..].trim();
    if !remainder.is_empty() {
        sentences.push(remainder);
    }
    sentences
}

// ── Utilities ─────────────────────────────────────────────────────────────

fn content_hash(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn dedup_preserve_order(v: &mut Vec<String>) {
    let mut seen = HashSet::new();
    v.retain(|s| seen.insert(s.to_lowercase()));
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{ProjectKey, TenantId, WorkspaceId};

    fn project() -> ProjectKey {
        ProjectKey::new(TenantId::new("t1"), WorkspaceId::new("w1"), "p1".to_owned())
    }

    fn extractor() -> RegexEntityExtractor {
        RegexEntityExtractor::new()
    }

    #[test]
    fn extract_person_names() {
        let req = EntityExtractionRequest {
            text: "Alan Turing worked at Bletchley Park. Marie Curie discovered polonium."
                .to_owned(),
            project: project(),
            extract_persons: true,
            extract_orgs: false,
            extract_locations: false,
            extract_facts: false,
        };
        let result = extractor().extract(&req);
        assert!(
            result.persons.iter().any(|p| p.contains("Alan") || p.contains("Turing")),
            "expected Alan Turing in persons, got: {:?}", result.persons
        );
        assert!(
            result.persons.iter().any(|p| p.contains("Marie") || p.contains("Curie")),
            "expected Marie Curie in persons, got: {:?}", result.persons
        );
    }

    #[test]
    fn extract_org_names_with_suffix() {
        let req = EntityExtractionRequest {
            text: "Anthropic Inc was founded in 2021. OpenAI develops GPT models."
                .to_owned(),
            project: project(),
            extract_persons: false,
            extract_orgs: true,
            extract_locations: false,
            extract_facts: false,
        };
        let result = extractor().extract(&req);
        assert!(
            result.orgs.iter().any(|o| o.contains("Anthropic")),
            "expected Anthropic in orgs, got: {:?}", result.orgs
        );
        assert!(
            result.orgs.iter().any(|o| o.contains("OpenAI")),
            "expected OpenAI in orgs, got: {:?}", result.orgs
        );
    }

    #[test]
    fn extract_locations_from_prepositions() {
        let req = EntityExtractionRequest {
            text: "The conference was held in San Francisco and researchers came from London."
                .to_owned(),
            project: project(),
            extract_persons: false,
            extract_orgs: false,
            extract_locations: true,
            extract_facts: false,
        };
        let result = extractor().extract(&req);
        assert!(
            result.locations.iter().any(|l| l.contains("San") || l.contains("Francisco") || l.contains("London")),
            "expected San Francisco or London in locations, got: {:?}", result.locations
        );
    }

    #[test]
    fn extract_facts_from_declarative_sentences() {
        let req = EntityExtractionRequest {
            text: "Rust is a systems programming language. Python was created by Guido. Java has a garbage collector."
                .to_owned(),
            project: project(),
            extract_persons: false,
            extract_orgs: false,
            extract_locations: false,
            extract_facts: true,
        };
        let result = extractor().extract(&req);
        assert!(
            result.facts.len() >= 2,
            "expected at least 2 facts, got {:?}", result.facts
        );
    }

    #[test]
    fn all_entities_deduplicates() {
        let result = EntityExtractionResult {
            persons: vec!["Alan Turing".to_owned(), "Alan Turing".to_owned()],
            orgs: vec!["Anthropic".to_owned()],
            locations: vec!["London".to_owned()],
            facts: vec![],
            source_text_hash: "abc".to_owned(),
        };
        let all = result.all_entities();
        // Alan Turing appears twice in persons but only once in all_entities.
        assert_eq!(all.iter().filter(|e| e.contains("Alan")).count(), 1);
        assert_eq!(all.len(), 3); // Alan Turing, Anthropic, London
    }

    #[test]
    fn empty_text_returns_empty_result() {
        let req = EntityExtractionRequest::all(String::new(), project());
        let result = extractor().extract(&req);
        assert!(result.is_empty());
    }

    #[test]
    fn source_text_hash_is_stable() {
        let text = "Stable content for hashing.".to_owned();
        let req1 = EntityExtractionRequest::all(text.clone(), project());
        let req2 = EntityExtractionRequest::all(text, project());
        let r1 = extractor().extract(&req1);
        let r2 = extractor().extract(&req2);
        assert_eq!(r1.source_text_hash, r2.source_text_hash);
    }

    #[test]
    fn different_texts_produce_different_hashes() {
        let req1 = EntityExtractionRequest::all("text one".to_owned(), project());
        let req2 = EntityExtractionRequest::all("text two".to_owned(), project());
        let r1 = extractor().extract(&req1);
        let r2 = extractor().extract(&req2);
        assert_ne!(r1.source_text_hash, r2.source_text_hash);
    }

    #[test]
    fn extract_disabled_fields_return_empty() {
        let req = EntityExtractionRequest {
            text: "Alan Turing at Anthropic Inc in London is a scientist.".to_owned(),
            project: project(),
            extract_persons: false,
            extract_orgs: false,
            extract_locations: false,
            extract_facts: false,
        };
        let result = extractor().extract(&req);
        assert!(result.persons.is_empty());
        assert!(result.orgs.is_empty());
        assert!(result.locations.is_empty());
        assert!(result.facts.is_empty());
    }
}
