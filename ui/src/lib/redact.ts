// Client-side secret redaction — belt-and-suspenders defence against any
// backend error string that might slip through with an API key in it.
// The backend already redacts at every known boundary; this is a final
// safety net applied to every `ApiError.message` before it reaches the UI.
//
// Mirrors the patterns in crates/cairn-providers/src/redact.rs. Kept
// deliberately narrow: only known provider-key shapes and sensitive URL
// query params, so we never over-redact legitimate operator-facing detail
// like request ids or status codes.

const REDACTED = "[REDACTED]";

const SENSITIVE_PARAMS = [
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

const QUERY_PARAM_RE = new RegExp(
  `([?&](?:${SENSITIVE_PARAMS.join("|")})=)([^&\\s"']+)`,
  "gi",
);

const AUTH_HEADER_RE =
  /(authorization\s*[:=]\s*(?:bearer|basic|token)\s+)([^\s"'<>,)]+)/gi;

const API_KEY_HEADER_RE =
  /((?:x-api-key|api-key|x-goog-api-key|anthropic-api-key|openai-api-key)\s*[:=]\s*)([^\s"'<>,)]+)/gi;

// Match the Rust PROVIDER_KEY_RE shape: each alternative is anchored by
// \b word boundaries so we don't over-redact long alphanumeric ids that
// happen to contain a key-shaped substring. Kept in lockstep with
// crates/cairn-providers/src/redact.rs — divergence between the two
// implementations makes backend vs frontend error messages hard to
// correlate during incident review.
const PROVIDER_KEY_RE = new RegExp(
  `\\b(?:${[
    String.raw`sk-ant-[A-Za-z0-9_\-]{20,}`,
    String.raw`sk-proj-[A-Za-z0-9_\-]{20,}`,
    String.raw`sk-or-v1-[A-Za-z0-9_\-]{20,}`,
    String.raw`sk-[A-Za-z0-9]{20,}`,
    String.raw`xai-[A-Za-z0-9]{20,}`,
    String.raw`gsk_[A-Za-z0-9]{20,}`,
    String.raw`AIza[0-9A-Za-z_\-]{20,}`,
    String.raw`ghp_[A-Za-z0-9]{20,}`,
    String.raw`gho_[A-Za-z0-9]{20,}`,
    String.raw`ghs_[A-Za-z0-9]{20,}`,
    String.raw`ghu_[A-Za-z0-9]{20,}`,
    String.raw`github_pat_[A-Za-z0-9_]{20,}`,
    String.raw`xox[abprs]-[A-Za-z0-9\-]{10,}`,
    String.raw`Bearer\s+[A-Za-z0-9_\-.]{20,}`,
  ].join("|")})\\b`,
  "g",
);

/** Replace every secret-shaped substring in `text` with [REDACTED].
 *
 * Accepts non-string inputs defensively: callers sometimes funnel raw
 * JSON error bodies through here (`{ message: {...} }`, arrays, etc.),
 * and `.replace` on a non-string would throw. Non-strings are coerced
 * via `String()` before pattern matching. */
export function redactSecrets(text: unknown): string {
  if (text == null) return "";
  const s = typeof text === "string" ? text : String(text);
  if (!s) return s;
  let out = s.replace(QUERY_PARAM_RE, (_m, name) => `${name}${REDACTED}`);
  out = out.replace(AUTH_HEADER_RE, (_m, prefix) => `${prefix}${REDACTED}`);
  out = out.replace(API_KEY_HEADER_RE, (_m, prefix) => `${prefix}${REDACTED}`);
  out = out.replace(PROVIDER_KEY_RE, REDACTED);
  return out;
}
