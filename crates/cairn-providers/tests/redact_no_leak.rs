//! End-to-end: no provider error path — Http, ResponseFormat, Provider,
//! Auth — may carry an API key into the `ProviderError` payload.
//!
//! We stage three scenarios against a local mock server:
//!
//! 1. Upstream returns a 401 body that echoes the submitted `Authorization`
//!    header (some real providers do this).
//! 2. Upstream returns a 500 body that inlines the submitted `api_key`
//!    query parameter.
//! 3. Upstream returns an unparseable body that gets stuffed into
//!    `ProviderError::ResponseFormat.raw_response`.
//!
//! In every case, the stringified error must NOT contain the key.

use cairn_providers::{
    ChatMessage, ChatProvider, ChatRole, MessageContent,
    wire::openai_compat::{OpenAiCompat, ProviderConfig},
};
use httpmock::prelude::*;

const FAKE_KEY: &str = "sk-ant-abcdef1234567890ABCDEFxyzLEAK";

fn provider(server: &MockServer) -> OpenAiCompat {
    OpenAiCompat::new(
        ProviderConfig::OPENAI,
        FAKE_KEY,
        Some(server.base_url()),
        Some("gpt-4.1-nano".to_owned()),
        None,
        None,
        Some(5),
    )
    .expect("provider should build")
}

fn user_msg() -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: ChatRole::User,
        content: "hi".to_owned(),
        content_type: MessageContent::Text,
    }]
}

#[tokio::test]
async fn provider_error_redacts_echoed_authorization_header() {
    let server = MockServer::start();
    // Upstream echoes the submitted Authorization header into its own error body.
    server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(401)
            .header("content-type", "application/json")
            .body(format!(
                r#"{{"error":{{"message":"invalid key Authorization: Bearer {FAKE_KEY}"}}}}"#,
            ));
    });

    let p = provider(&server);
    let err = p
        .chat_with_tools(&user_msg(), None, None)
        .await
        .expect_err("should error");
    let text = err.to_string();
    assert!(
        !text.contains(FAKE_KEY),
        "API key leaked into ProviderError: {text}"
    );
    assert!(
        text.contains("[REDACTED]"),
        "expected [REDACTED] in: {text}"
    );
}

#[tokio::test]
async fn provider_error_redacts_api_key_in_query_params() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(500)
            .header("content-type", "text/plain")
            .body(format!(
                "internal error at https://api.example.com/v1/chat?api_key={FAKE_KEY}&model=foo",
            ));
    });

    let p = provider(&server);
    let err = p
        .chat_with_tools(&user_msg(), None, None)
        .await
        .expect_err("should error");
    let text = err.to_string();
    assert!(
        !text.contains(FAKE_KEY),
        "API key leaked via query param into ProviderError: {text}"
    );
    assert!(text.contains("api_key=[REDACTED]"));
}

#[tokio::test]
async fn provider_error_redacts_key_in_unparseable_body() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        // 200 + unparseable JSON forces the ResponseFormat path which
        // embeds `raw_response` verbatim.
        then.status(200)
            .header("content-type", "application/json")
            .body(format!("not-json-but-contains-{FAKE_KEY}"));
    });

    let p = provider(&server);
    let err = p
        .chat_with_tools(&user_msg(), None, None)
        .await
        .expect_err("should error");
    let text = err.to_string();
    assert!(
        !text.contains(FAKE_KEY),
        "API key leaked into raw_response: {text}"
    );
}

#[tokio::test]
async fn provider_transport_error_redacts_key_in_url() {
    // Force a transport error and verify `reqwest::Error::to_string()` —
    // which embeds the request URL — gets redacted before it lands in
    // `ProviderError::Http`.
    //
    // Subtlety (addresses PR review feedback): `OpenAiCompat` builds the
    // request URL via `base_url.join(self.config.chat_endpoint)`, and
    // `Url::join` drops the base URL's query string AND its userinfo. So
    // embedding the secret as `?api_key=...` on `base_url` — or in the
    // userinfo — wouldn't actually survive into the request URL, making
    // the resulting assertion a false-negative trap (test passes even
    // if redaction regresses, because the secret never made it that far).
    //
    // What `Url::join` DOES preserve is the base URL's path when the
    // relative reference is itself relative and the base ends with `/`.
    // So we embed the secret as a path segment: `http://host/SECRET/` +
    // join `chat/completions` → `http://host/SECRET/chat/completions`.
    // reqwest's DNS-failure error echoes this full URL, giving us a
    // real leak to redact.
    let p = OpenAiCompat::new(
        ProviderConfig::OPENAI,
        FAKE_KEY,
        Some(format!(
            "http://does-not-resolve-{}.invalid/{FAKE_KEY}/",
            std::process::id()
        )),
        None,
        None,
        None,
        Some(1),
    )
    .expect("provider should build");

    let err = p
        .chat_with_tools(&user_msg(), None, None)
        .await
        .expect_err("should error");
    let text = err.to_string();
    // Sanity: the URL-carrying transport path actually fired. If this
    // fails, some earlier validation cut us off and the redaction
    // assertion below would be a false pass.
    assert!(
        text.contains("does-not-resolve"),
        "expected dns-error URL in transport error (was redaction short-circuited?): {text}"
    );
    // The key lived in the URL path segment; redaction must have
    // replaced it with the marker.
    assert!(
        text.contains("[REDACTED]"),
        "expected [REDACTED] marker in transport error: {text}"
    );
    assert!(
        !text.contains(FAKE_KEY),
        "API key leaked via URL in transport error: {text}"
    );
}
