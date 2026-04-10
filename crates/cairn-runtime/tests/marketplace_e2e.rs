//! Integration tests for RFC 015: Plugin Marketplace and Per-Project Scoping.
//!
//! These tests cover the sealed integration test list from RFC 015.
//! Tests requiring a live plugin host (spawn, drain, signal routing) are
//! deferred until the host extensions are complete.

use cairn_domain::contexts::{PluginCategory, SignalCaptureOverride};
use cairn_domain::ids::OperatorId;
use cairn_domain::tenancy::ProjectKey;
use cairn_runtime::services::marketplace_service::{
    is_plugin_tool_visible, is_signal_allowed, resolve_capture_policy, CredentialScopeKey,
    MarketplaceCommand, MarketplaceError, MarketplaceEvent, MarketplaceService, MarketplaceState,
    PluginEnablement, VerificationOutcome,
};
use std::sync::Arc;

fn operator() -> OperatorId {
    OperatorId::new("test-operator")
}

fn project(id: &str) -> ProjectKey {
    ProjectKey::new("acme", "eng", id)
}

fn setup_service() -> MarketplaceService<()> {
    let mut svc = MarketplaceService::new(Arc::new(()));
    svc.load_bundled_catalog();
    svc
}

// ── RFC 015 Integration Test 1: Bundled catalog listing ─────────────────────

#[test]
fn rfc015_test1_bundled_catalog_lists_github() {
    let svc = setup_service();
    let records = svc.list_all_records();

    // GitHub entry must appear
    let github = records
        .iter()
        .find(|r| r.plugin_id == "github")
        .expect("GitHub must be in bundled catalog");

    assert_eq!(github.state, MarketplaceState::Listed);
    assert_eq!(github.descriptor.category, PluginCategory::IssueTracker);
    assert_eq!(github.descriptor.tools.len(), 19);
    assert_eq!(github.descriptor.signal_sources.len(), 11);
    assert!(github.descriptor.download_url.is_some());
}

// ── RFC 015 Integration Test 2: Install flow ────────────────────────────────

#[test]
fn rfc015_test2_install_transitions_to_installed() {
    let mut svc = setup_service();

    let events = svc
        .handle_command(MarketplaceCommand::InstallPlugin {
            plugin_id: "github".into(),
            initiated_by: operator(),
        })
        .unwrap();

    // Must emit PluginInstallationStarted + PluginInstalled
    assert!(events.iter().any(|e| matches!(
        e,
        MarketplaceEvent::PluginInstallationStarted { plugin_id, .. } if plugin_id == "github"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        MarketplaceEvent::PluginInstalled { plugin_id, .. } if plugin_id == "github"
    )));

    // State is now Installed
    assert_eq!(
        svc.get_record("github").unwrap().state,
        MarketplaceState::Installed
    );
}

// ── RFC 015 Integration Test 3: Credential wizard ───────────────────────────

#[test]
fn rfc015_test3_credentials_require_installed_state() {
    let mut svc = setup_service();

    // Can't provide credentials before install
    let result = svc.handle_command(MarketplaceCommand::ProvidePluginCredentials {
        plugin_id: "github".into(),
        credentials: vec![("github_app_id".into(), "12345".into())],
        provided_by: operator(),
    });
    assert!(result.is_err());

    // Install first
    svc.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id: "github".into(),
        initiated_by: operator(),
    })
    .unwrap();

    // Now credentials work
    let events = svc
        .handle_command(MarketplaceCommand::ProvidePluginCredentials {
            plugin_id: "github".into(),
            credentials: vec![
                ("github_app_id".into(), "12345".into()),
                ("github_app_private_key".into(), "pem-data".into()),
            ],
            provided_by: operator(),
        })
        .unwrap();

    assert!(events.iter().any(|e| matches!(
        e,
        MarketplaceEvent::PluginCredentialsProvided { plugin_id, .. } if plugin_id == "github"
    )));
}

// ── RFC 015 Integration Test 4b: Ephemeral credential verification ──────────

#[test]
fn rfc015_test4b_verify_is_ephemeral_no_connected_state() {
    let mut svc = setup_service();
    svc.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id: "github".into(),
        initiated_by: operator(),
    })
    .unwrap();

    let events = svc
        .handle_command(MarketplaceCommand::VerifyPluginCredentials {
            plugin_id: "github".into(),
            credential_scope_key: None,
            verified_by: operator(),
        })
        .unwrap();

    // Must emit PluginCredentialsVerified with Ok outcome
    assert!(events.iter().any(|e| matches!(
        e,
        MarketplaceEvent::PluginCredentialsVerified {
            outcome: VerificationOutcome::Ok,
            ..
        }
    )));

    // State is STILL Installed — no Connected state exists
    assert_eq!(
        svc.get_record("github").unwrap().state,
        MarketplaceState::Installed
    );
}

// ── RFC 015 Integration Test 5: Per-project enable ──────────────────────────

#[test]
fn rfc015_test5_enable_for_project_with_allowlists() {
    let mut svc = setup_service();
    svc.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id: "github".into(),
        initiated_by: operator(),
    })
    .unwrap();

    let p1 = project("proj-alpha");

    let events = svc
        .handle_command(MarketplaceCommand::EnablePluginForProject {
            plugin_id: "github".into(),
            project: p1.clone(),
            tool_allowlist: Some(vec!["github.get_issue".into(), "github.list_issues".into()]),
            signal_allowlist: Some(vec!["github.issue.opened".into()]),
            signal_capture_override: Some(SignalCaptureOverride {
                graph_project: Some(false),
                memory_ingest: None,
            }),
            enabled_by: operator(),
        })
        .unwrap();

    // Event carries all three fields
    if let MarketplaceEvent::PluginEnabledForProject {
        tool_allowlist,
        signal_allowlist,
        signal_capture_override,
        ..
    } = &events[0]
    {
        assert_eq!(tool_allowlist.as_ref().unwrap().len(), 2);
        assert_eq!(signal_allowlist.as_ref().unwrap().len(), 1);
        assert_eq!(
            signal_capture_override.as_ref().unwrap().graph_project,
            Some(false)
        );
    } else {
        panic!("expected PluginEnabledForProject");
    }

    // Visibility context has the tools
    let ctx = svc.build_visibility_context(&p1, None);
    assert!(is_plugin_tool_visible(&ctx, "github", "github.get_issue"));
    assert!(!is_plugin_tool_visible(
        &ctx,
        "github",
        "github.create_pull_request"
    ));
}

// ── RFC 015 Integration Test 6: Per-project isolation ───────────────────────

#[test]
fn rfc015_test6_different_project_sees_nothing() {
    let mut svc = setup_service();
    svc.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id: "github".into(),
        initiated_by: operator(),
    })
    .unwrap();

    let p1 = project("proj-alpha");
    let p2 = project("proj-beta");

    svc.handle_command(MarketplaceCommand::EnablePluginForProject {
        plugin_id: "github".into(),
        project: p1.clone(),
        tool_allowlist: None,
        signal_allowlist: None,
        signal_capture_override: None,
        enabled_by: operator(),
    })
    .unwrap();

    // p1 sees tools
    let ctx1 = svc.build_visibility_context(&p1, None);
    assert!(is_plugin_tool_visible(&ctx1, "github", "github.get_issue"));

    // p2 sees nothing — plugin not enabled there
    let ctx2 = svc.build_visibility_context(&p2, None);
    assert!(!is_plugin_tool_visible(
        &ctx2,
        "github",
        "github.get_issue"
    ));
}

// ── RFC 015 Integration Test 7: Tool allowlist ──────────────────────────────

#[test]
fn rfc015_test7_tool_allowlist_restricts_visibility() {
    let mut svc = setup_service();
    svc.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id: "github".into(),
        initiated_by: operator(),
    })
    .unwrap();

    let p1 = project("proj-restricted");
    svc.handle_command(MarketplaceCommand::EnablePluginForProject {
        plugin_id: "github".into(),
        project: p1.clone(),
        tool_allowlist: Some(vec!["github.get_issue".into()]),
        signal_allowlist: None,
        signal_capture_override: None,
        enabled_by: operator(),
    })
    .unwrap();

    let ctx = svc.build_visibility_context(&p1, None);
    assert!(is_plugin_tool_visible(&ctx, "github", "github.get_issue"));
    assert!(!is_plugin_tool_visible(
        &ctx,
        "github",
        "github.list_issues"
    ));
    assert!(!is_plugin_tool_visible(
        &ctx,
        "github",
        "github.merge_pull_request"
    ));
}

// ── RFC 015 Integration Test 7a: Signal allowlist ───────────────────────────

#[test]
fn rfc015_test7a_signal_allowlist_restricts_routing() {
    let enablement = PluginEnablement {
        plugin_id: "github".into(),
        project: project("proj-filtered"),
        enabled: true,
        enabled_at: 0,
        enabled_by: operator(),
        tool_allowlist: None,
        signal_allowlist: Some(vec![
            "github.issue.opened".into(),
            "github.issue.labeled".into(),
        ]),
        signal_capture_override: None,
    };

    // Allowed types pass
    assert!(is_signal_allowed(&enablement, "github.issue.opened"));
    assert!(is_signal_allowed(&enablement, "github.issue.labeled"));

    // Non-allowed types are dropped before trigger evaluation
    assert!(!is_signal_allowed(
        &enablement,
        "github.pull_request.opened"
    ));
    assert!(!is_signal_allowed(
        &enablement,
        "github.rate_limit.warning"
    ));
}

// ── RFC 015 Integration Test 8b: Per-project signal capture override ────────

#[test]
fn rfc015_test8b_capture_override_disables_both_tracks() {
    let enablement = PluginEnablement {
        plugin_id: "github".into(),
        project: project("proj-compliance"),
        enabled: true,
        enabled_at: 0,
        enabled_by: operator(),
        tool_allowlist: None,
        signal_allowlist: None,
        signal_capture_override: Some(SignalCaptureOverride {
            graph_project: Some(false),
            memory_ingest: Some(false),
        }),
    };

    // Both tracks disabled even though plugin declares memory_ingest
    let policy = resolve_capture_policy(&enablement, true);
    assert!(!policy.graph_project);
    assert!(!policy.memory_ingest);
}

// ── RFC 015 Integration Test 11: EvalScorer manifest rejection ──────────────

#[test]
fn rfc015_test11_eval_scorer_rejected_at_install() {
    let mut svc = setup_service();

    // Create and list a plugin with EvalScorer category
    let mut desc = svc.get_record("github").unwrap().descriptor.clone();
    desc.id = "my-eval-scorer".into();
    desc.category = PluginCategory::EvalScorer;
    svc.list_plugin(desc);

    let result = svc.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id: "my-eval-scorer".into(),
        initiated_by: operator(),
    });

    assert!(matches!(result, Err(MarketplaceError::EvalScorerReserved)));
}

// ── Full lifecycle: list → install → credentials → enable → disable → uninstall

#[test]
fn rfc015_full_marketplace_lifecycle() {
    let mut svc = setup_service();

    // 1. Listed from catalog
    let github = svc.get_record("github").unwrap();
    assert_eq!(github.state, MarketplaceState::Listed);

    // 2. Install
    svc.handle_command(MarketplaceCommand::InstallPlugin {
        plugin_id: "github".into(),
        initiated_by: operator(),
    })
    .unwrap();

    // 3. Credentials
    svc.handle_command(MarketplaceCommand::ProvidePluginCredentials {
        plugin_id: "github".into(),
        credentials: vec![("github_app_id".into(), "12345".into())],
        provided_by: operator(),
    })
    .unwrap();

    // 4. Verify (ephemeral)
    svc.handle_command(MarketplaceCommand::VerifyPluginCredentials {
        plugin_id: "github".into(),
        credential_scope_key: Some(CredentialScopeKey("tenant-default".into())),
        verified_by: operator(),
    })
    .unwrap();
    assert_eq!(
        svc.get_record("github").unwrap().state,
        MarketplaceState::Installed
    );

    // 5. Enable for project
    let p1 = project("prod");
    svc.handle_command(MarketplaceCommand::EnablePluginForProject {
        plugin_id: "github".into(),
        project: p1.clone(),
        tool_allowlist: None,
        signal_allowlist: None,
        signal_capture_override: None,
        enabled_by: operator(),
    })
    .unwrap();
    assert!(svc.get_enablement("github", &p1).unwrap().enabled);

    // 6. Disable for project
    svc.handle_command(MarketplaceCommand::DisablePluginForProject {
        plugin_id: "github".into(),
        project: p1.clone(),
        disabled_by: operator(),
    })
    .unwrap();
    assert!(!svc.get_enablement("github", &p1).unwrap().enabled);

    // 7. Uninstall
    svc.handle_command(MarketplaceCommand::UninstallPlugin {
        plugin_id: "github".into(),
        uninstalled_by: operator(),
    })
    .unwrap();
    assert_eq!(
        svc.get_record("github").unwrap().state,
        MarketplaceState::Uninstalled
    );
}
