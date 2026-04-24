//! Secret redaction for error messages and log strings.
//!
//! Any string that crosses a trust boundary — HTTP response body, `tracing`
//! event, `ProviderError` payload — must pass through [`redact_secrets`] so
//! that an operator-misconfigured API key never ends up in the UI or a log
//! file.
//!
//! The redactor targets secrets that the provider layer actually handles:
//!
//! - Sensitive URL query parameters (`api_key=`, `token=`, `apikey=`,
//!   `password=`, `secret=`, `bearer=`, `access_token=`, `refresh_token=`,
//!   `key=`).
//! - `Authorization: Bearer <token>` and `x-api-key: <value>` header
//!   fragments that some upstream providers echo back into error bodies.
//! - Opaque-looking provider keys detected by prefix (`sk-`, `sk-ant-`,
//!   `xai-`, `gsk_`, `AIza`, `ghp_`, `github_pat_`, `xoxb-`, `xoxa-`,
//!   `xoxp-`, …) followed by ≥20 key-shaped characters.
//!
//! We intentionally do **not** redact bare long hex / base64 strings that
//! lack any contextual marker — they collide with request ids, SHA-256
//! digests, JSON-Web-Key thumbprints, etc. Over-redaction would make
//! production errors unreadable. A secret that ships without any prefix or
//! query/header context is not something this layer can reliably recognise;
//! the upstream fix for those is never to log the raw body in the first
//! place, which is enforced separately.

use once_cell::sync::Lazy;
use regex::Regex;

const REDACTED: &str = "[REDACTED]";

/// Query-parameter names whose values are treated as secrets.
const SENSITIVE_PARAMS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "token",
    "access_token",
    "refresh_token",
    "password",
    "secret",
    "bearer",
    "key",
    "auth",
];

// `?api_key=VALUE` / `&token=VALUE` etc. — case-insensitive on the name,
// greedy-but-bounded on the value so we don't eat subsequent `&` pairs.
static QUERY_PARAM_RE: Lazy<Regex> = Lazy::new(|| {
    let names = SENSITIVE_PARAMS.join("|");
    let pattern = format!(r"(?i)([?&](?:{names})=)([^&\s\x22\x27]+)");
    Regex::new(&pattern).expect("valid query-param redaction regex")
});

// `Authorization: Bearer <token>` and similar. Matches the header name up to
// end-of-line or common body-delimiter punctuation.
static AUTH_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)(authorization\s*[:=]\s*(?:bearer|basic|token)\s+)([^\s"'<>,\)]+)"#)
        .expect("valid auth-header regex")
});

// `x-api-key: <value>`, `api-key: <value>`, `x-goog-api-key: <value>`, ...
static API_KEY_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)((?:x-api-key|api-key|x-goog-api-key|anthropic-api-key|openai-api-key)\s*[:=]\s*)([^\s"'<>,\)]+)"#)
        .expect("valid api-key header regex")
});

// Bare provider-key literals. Each alternative is anchored on a known prefix
// so we only redact values that *look* like keys rather than any long
// hex/base64 string.
static PROVIDER_KEY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)
          \b(
              sk-ant-[A-Za-z0-9_\-]{20,}                 # Anthropic
            | sk-proj-[A-Za-z0-9_\-]{20,}                # OpenAI project keys
            | sk-or-v1-[A-Za-z0-9_\-]{20,}               # OpenRouter
            | sk-[A-Za-z0-9]{20,}                        # OpenAI-style generic
            | xai-[A-Za-z0-9]{20,}                       # xAI
            | gsk_[A-Za-z0-9]{20,}                       # Groq
            | AIza[0-9A-Za-z_\-]{20,}                    # Google API keys
            | ghp_[A-Za-z0-9]{20,}                       # GitHub personal token
            | gho_[A-Za-z0-9]{20,}                       # GitHub OAuth token
            | ghs_[A-Za-z0-9]{20,}                       # GitHub server token
            | ghu_[A-Za-z0-9]{20,}                       # GitHub user token
            | github_pat_[A-Za-z0-9_]{20,}               # GitHub fine-grained PAT
            | xox[abprs]-[A-Za-z0-9\-]{10,}              # Slack tokens
            | Bearer\s+[A-Za-z0-9_\-\.]{20,}             # Inline bearer
          )\b
        ",
    )
    .expect("valid provider-key regex")
});

/// Redact every secret-shaped substring from `text`.
///
/// Returns a new `String` with each hit replaced by `[REDACTED]` (or
/// `<name>=[REDACTED]` for query parameters so the param name is preserved
/// as a useful debugging signal).
pub fn redact_secrets(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // 1. Query-param secrets: keep the name, redact the value.
    let out = QUERY_PARAM_RE.replace_all(text, |caps: &regex::Captures<'_>| {
        format!("{}{}", &caps[1], REDACTED)
    });

    // 2. Authorization headers: keep the scheme label, redact the credential.
    let out = AUTH_HEADER_RE.replace_all(&out, |caps: &regex::Captures<'_>| {
        format!("{}{}", &caps[1], REDACTED)
    });

    // 3. Raw API-key headers.
    let out = API_KEY_HEADER_RE.replace_all(&out, |caps: &regex::Captures<'_>| {
        format!("{}{}", &caps[1], REDACTED)
    });

    // 4. Known provider-key literals anywhere else in the string.
    PROVIDER_KEY_RE.replace_all(&out, REDACTED).into_owned()
}

/// Convenience wrapper used by the provider layer when it needs to pipe a
/// potentially-unsafe raw response body into a `ProviderError`.
///
/// Guarantees:
///
/// - The returned string is both redacted *and* truncated to the existing
///   [`truncate_raw_response`] ceiling so `ProviderError` payloads stay
///   bounded.
/// - Secrets are replaced with the `[REDACTED]` marker **before** the
///   truncator sees the string, so no secret material can survive past
///   the cutoff. (The marker itself is a short literal — a marker that
///   straddles the cutoff can still be split on a char boundary, which
///   is harmless: the intent is to guarantee secret removal, not
///   cosmetic marker integrity.)
///
/// [`truncate_raw_response`]: crate::error::truncate_raw_response
pub fn redact_and_truncate(raw: &str) -> String {
    let redacted = redact_secrets(raw);
    crate::error::truncate_raw_response(&redacted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_query_param_api_key() {
        let input = "https://api.example.com/v1/models?api_key=sk-abcdef123456789012345&foo=bar";
        let out = redact_secrets(input);
        assert!(!out.contains("sk-abcdef"), "leaked: {out}");
        assert!(out.contains("api_key=[REDACTED]"));
        assert!(out.contains("foo=bar"));
    }

    #[test]
    fn redacts_multiple_query_params() {
        let input = "url=https://x.com/?token=abc123&api_key=xyz789&other=keep";
        let out = redact_secrets(input);
        assert!(out.contains("token=[REDACTED]"));
        assert!(out.contains("api_key=[REDACTED]"));
        assert!(out.contains("other=keep"));
    }

    #[test]
    fn redacts_authorization_bearer_header() {
        let input = "request failed: Authorization: Bearer sk-abcdef1234567890abcdefxyz";
        let out = redact_secrets(input);
        assert!(!out.contains("sk-abcdef"), "leaked: {out}");
        assert!(out.to_lowercase().contains("bearer [redacted]"));
    }

    #[test]
    fn redacts_x_api_key_header() {
        let input = "headers: {x-api-key: sk-ant-abcdef1234567890abcdefxyz, other: val}";
        let out = redact_secrets(input);
        assert!(!out.contains("sk-ant-abcdef"), "leaked: {out}");
    }

    #[test]
    fn redacts_anthropic_key_prefix() {
        let input = "error: invalid key sk-ant-api03-abcdef1234567890ABCDEF_xyz provided";
        let out = redact_secrets(input);
        assert!(!out.contains("sk-ant-api03-abcdef"), "leaked: {out}");
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_openai_project_key() {
        let input = "key=sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX";
        let out = redact_secrets(input);
        assert!(!out.contains("sk-proj-ABCDEFGH"), "leaked: {out}");
    }

    #[test]
    fn redacts_openrouter_key() {
        let input = "bad key sk-or-v1-abcdef1234567890abcdefABCDEFX end";
        let out = redact_secrets(input);
        assert!(!out.contains("sk-or-v1-abcdef"), "leaked: {out}");
    }

    #[test]
    fn redacts_xai_key() {
        let input = "Bearer xai-abcdef1234567890abcdef";
        let out = redact_secrets(input);
        assert!(!out.contains("xai-abcdef"), "leaked: {out}");
    }

    #[test]
    fn redacts_groq_key() {
        let input = "using gsk_abcdef1234567890ABCDEF1234 failed";
        let out = redact_secrets(input);
        assert!(!out.contains("gsk_abcdef"), "leaked: {out}");
    }

    #[test]
    fn redacts_google_api_key() {
        let input = "AIzaSyAbcdef1234567890ABCDEFXyz";
        let out = redact_secrets(input);
        assert!(!out.contains("AIzaSyAbcdef"), "leaked: {out}");
    }

    #[test]
    fn redacts_github_pat() {
        let input = "token ghp_abcdef1234567890ABCDEF1234 expired";
        let out = redact_secrets(input);
        assert!(!out.contains("ghp_abcdef"), "leaked: {out}");

        let input2 = "github_pat_11ABCDEFG0abcdef1234567890_xyzABCDEF";
        let out2 = redact_secrets(input2);
        assert!(!out2.contains("github_pat_11ABCDEF"), "leaked: {out2}");
    }

    #[test]
    fn redacts_slack_tokens() {
        let input = "webhook xoxb-1234567890-ABCDEFXYZ failed";
        let out = redact_secrets(input);
        assert!(!out.contains("xoxb-1234567890"), "leaked: {out}");
    }

    #[test]
    fn does_not_redact_plain_hex_hash() {
        // SHA-256 hash in a request id — no prefix, no context ⇒ keep as-is.
        let input = "request_id=3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b";
        let out = redact_secrets(input);
        assert!(
            out.contains("3a7bd3e2360a3d29eea436fcfb7e44c7"),
            "over-redacted: {out}"
        );
    }

    #[test]
    fn does_not_redact_short_values() {
        // "sk-short" is only 8 chars — below the 20-char threshold.
        let input = "key sk-short is fake";
        let out = redact_secrets(input);
        assert_eq!(out, input);
    }

    #[test]
    fn handles_empty_string() {
        assert_eq!(redact_secrets(""), "");
    }

    #[test]
    fn handles_string_without_secrets() {
        let input = "HTTP 500 internal server error at /v1/chat/completions";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn redacts_percent_encoded_reqwest_url() {
        // reqwest::Error::to_string() looks like:
        //   "error sending request for url (https://example.com/v1/models?api_key=sk-abcdef1234567890abcdef123): ..."
        let input = "error sending request for url (https://example.com/v1/models?api_key=sk-proj-ABCDEFGHIJKLMNOPQRST): dns error";
        let out = redact_secrets(input);
        assert!(!out.contains("sk-proj-ABCDEFGH"), "leaked: {out}");
        assert!(out.contains("api_key=[REDACTED]"));
    }

    #[test]
    fn redact_and_truncate_caps_length() {
        let big = "a".repeat(1000);
        let out = redact_and_truncate(&big);
        assert!(out.len() <= 520, "truncation not applied: {}", out.len());
    }

    #[test]
    fn redact_and_truncate_redacts_before_truncating() {
        // Put the secret near the end so naive truncation would keep it.
        let mut s = "x".repeat(450);
        s.push_str(" sk-ant-abcdef1234567890ABCDEFxyz tail");
        let out = redact_and_truncate(&s);
        assert!(!out.contains("sk-ant-abcdef"), "leaked: {out}");
    }

    #[test]
    fn redacts_basic_auth() {
        let input = "Authorization: Basic dXNlcjpwYXNzd29yZA1234567890abcd";
        let out = redact_secrets(input);
        assert!(!out.contains("dXNlcjpwYXNzd29yZA"), "leaked: {out}");
    }

    #[test]
    fn preserves_surrounding_text_context() {
        let input = "ProviderError(Http(\"connection refused for https://api.openai.com/?api_key=sk-abcdef1234567890abcdef123\"))";
        let out = redact_secrets(input);
        assert!(out.contains("connection refused"));
        assert!(out.contains("api.openai.com"));
        assert!(!out.contains("sk-abcdef"), "leaked: {out}");
    }
}
