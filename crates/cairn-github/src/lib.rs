//! cairn-github — GitHub App integration for Cairn.
//!
//! Provides:
//! - GitHub App JWT authentication and installation access tokens
//! - HMAC-SHA256 webhook signature verification
//! - GitHub REST API client for issues, branches, files, PRs, comments
//!
//! This crate has no dependencies on cairn internals — it's a standalone
//! GitHub App SDK that cairn-app and cairn-tools consume.

pub mod auth;
pub mod client;
pub mod error;
pub mod webhook;

pub use auth::{AppCredentials, AppInstallation, InstallationToken};
pub use client::GitHubClient;
pub use error::GitHubError;
pub use webhook::{verify_signature, WebhookEvent, WebhookEventPayload};
