//! Deprecated `/v1/tool-call-approvals/*` surface (F45).
//!
//! The unified approval API lives at `/v1/approvals/*` (see
//! [`crate::handlers::approvals`]). This module keeps the pre-F45 paths
//! live for zero-downtime migration but responds with **308 Permanent
//! Redirect** pointing at the unified location.
//!
//! 308 is preferred over 301 because `reqwest`, `axios`, `fetch`, and
//! cURL all preserve the method + body across 308 hops; 301 historically
//! rewrote POST→GET. Every tool-call client keeps working unchanged —
//! the redirect is transparent.
//!
//! The redirects are removed in a follow-up once telemetry shows the
//! legacy paths are no longer hit.

use axum::{
    extract::{OriginalUri, Path},
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

/// RFC 8594 — signals a deprecated endpoint.
static DEPRECATION: HeaderName = HeaderName::from_static("deprecation");
/// RFC 8288 — points at the successor URI family.
static LINK: HeaderName = HeaderName::from_static("link");

const PERMANENT_REDIRECT: StatusCode = StatusCode::PERMANENT_REDIRECT;

/// Minimal set for URL path segments: encode controls + characters that
/// would terminate or alter the path/query, leave common ID chars like
/// `_`, `-`, `.` alone. Matches RFC 3986 pchar minus subdelims that we
/// choose to escape for safety.
const PATH_SEGMENT_ESCAPE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/');

fn enc(raw: &str) -> String {
    utf8_percent_encode(raw, PATH_SEGMENT_ESCAPE).to_string()
}

fn redirect_to(target: String) -> Response {
    // 308 Permanent Redirect — preserves method + body.
    let mut resp = (PERMANENT_REDIRECT, format!("redirected to {target}\n")).into_response();
    let headers = resp.headers_mut();
    // Build header values from the target once; fall back to `/` only
    // on the (extremely unlikely) case the target fails HeaderValue
    // validation — e.g. embedded control chars. The percent-encoding
    // above already scrubs that set, so the fallback exists purely as
    // a defence-in-depth belt-and-braces.
    let loc =
        HeaderValue::try_from(target.as_str()).unwrap_or_else(|_| HeaderValue::from_static("/"));
    headers.insert(header::LOCATION, loc);
    // RFC 8594 — signals the endpoint is deprecated. `true` is the
    // RFC-recommended sentinel value for "deprecated as of now".
    headers.insert(DEPRECATION.clone(), HeaderValue::from_static("true"));
    // RFC 8288 — point at the successor. We can't cheaply build a
    // single Link string that preserves the pchar set of `target` and
    // also embeds `rel=` attributes without an allocation dance, so
    // build it here.
    let link = format!("<{target}>; rel=\"successor-version\"");
    if let Ok(v) = HeaderValue::try_from(link) {
        headers.insert(LINK.clone(), v);
    }
    resp
}

/// `GET /v1/tool-call-approvals` → `308 /v1/approvals`.
///
/// Query string is forwarded verbatim so the list filters
/// (`run_id`, `session_id`, `state`, ...) keep working. Any caller that
/// relies on tool-call-only results can opt in to `kind=tool_call`.
pub(crate) async fn redirect_list(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    let qs = uri.query().unwrap_or_default();
    let target = if qs.is_empty() {
        "/v1/approvals".to_owned()
    } else {
        format!("/v1/approvals?{qs}")
    };
    redirect_to(target)
}

pub(crate) async fn redirect_get(Path(call_id): Path<String>) -> impl IntoResponse {
    redirect_to(format!("/v1/approvals/{}", enc(&call_id)))
}

pub(crate) async fn redirect_approve(Path(call_id): Path<String>) -> impl IntoResponse {
    redirect_to(format!("/v1/approvals/{}/approve", enc(&call_id)))
}

pub(crate) async fn redirect_reject(Path(call_id): Path<String>) -> impl IntoResponse {
    redirect_to(format!("/v1/approvals/{}/reject", enc(&call_id)))
}

pub(crate) async fn redirect_amend(Path(call_id): Path<String>) -> impl IntoResponse {
    redirect_to(format!("/v1/approvals/{}/amend", enc(&call_id)))
}
