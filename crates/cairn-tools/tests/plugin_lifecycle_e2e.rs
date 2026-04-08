//! RFC 007 plugin lifecycle end-to-end integration test.
//!
//! Validates the full plugin management pipeline:
//!   (1) register a plugin with capabilities via InMemoryPluginRegistry
//!   (2) verify it appears in the registry (get + list_all)
//!   (3) verify capabilities list is correct
//!   (4) register the same plugin in StdioPluginHost and verify Discovered state
//!   (5) unregister from the registry
//!   (6) verify it no longer appears in the registry
//!   (7) duplicate registration is rejected (AlreadyRegistered)
//!   (8) unregister non-existent plugin returns NotFound
//!   (9) multiple plugins in registry are all listed

use cairn_domain::ExecutionClass;
use cairn_tools::permissions::{DeclaredPermissions, Permission};
use cairn_tools::{
    InMemoryPluginRegistry, PluginCapability, PluginHost, PluginLimits, PluginManifest,
    PluginRegistry, PluginState, StdioPluginHost,
};

fn manifest(id: &str, capabilities: Vec<PluginCapability>) -> PluginManifest {
    PluginManifest {
        id: id.to_owned(),
        name: format!("{id} Plugin"),
        version: "1.0.0".to_owned(),
        command: vec!["plugin-binary".to_owned(), "--serve".to_owned()],
        capabilities,
        permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
        limits: Some(PluginLimits {
            max_concurrency: Some(4),
            default_timeout_ms: Some(30_000),
        }),
        execution_class: ExecutionClass::SupervisedProcess,
        description: Some("Test plugin for lifecycle validation".to_owned()),
        homepage: Some("https://example.test/plugin".to_owned()),
    }
}

fn tool_provider_manifest(id: &str, tools: &[&str]) -> PluginManifest {
    manifest(
        id,
        vec![PluginCapability::ToolProvider {
            tools: tools.iter().map(|t| t.to_string()).collect(),
        }],
    )
}

// ── (1)+(2) Register and verify it appears ───────────────────────────────

#[test]
fn register_plugin_appears_in_registry() {
    let registry = InMemoryPluginRegistry::new();

    registry
        .register(tool_provider_manifest("com.test.alpha", &["alpha.run"]))
        .unwrap();

    let found = registry.get("com.test.alpha").unwrap();
    assert_eq!(found.id, "com.test.alpha");
    assert_eq!(found.name, "com.test.alpha Plugin");
    assert_eq!(found.version, "1.0.0");

    let all = registry.list_all();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, "com.test.alpha");
}

// ── (3) Capabilities list is correct ─────────────────────────────────────

#[test]
fn registered_plugin_capabilities_are_preserved() {
    let registry = InMemoryPluginRegistry::new();

    let caps = vec![
        PluginCapability::ToolProvider {
            tools: vec!["search".to_owned(), "index".to_owned()],
        },
        PluginCapability::PostTurnHook,
    ];
    registry.register(manifest("com.test.multi", caps)).unwrap();

    let found = registry.get("com.test.multi").unwrap();
    assert_eq!(found.capabilities.len(), 2);

    let has_tool_provider = found.capabilities.iter().any(|c| {
        matches!(c, PluginCapability::ToolProvider { tools } if tools.contains(&"search".to_owned()))
    });
    let has_post_turn = found
        .capabilities
        .iter()
        .any(|c| matches!(c, PluginCapability::PostTurnHook));

    assert!(has_tool_provider, "ToolProvider capability must be present");
    assert!(has_post_turn, "PostTurnHook capability must be present");
}

#[test]
fn tool_names_within_capability_are_preserved() {
    let registry = InMemoryPluginRegistry::new();
    registry
        .register(tool_provider_manifest(
            "com.test.git",
            &["git.status", "git.diff", "git.log"],
        ))
        .unwrap();

    let found = registry.get("com.test.git").unwrap();
    if let PluginCapability::ToolProvider { tools } = &found.capabilities[0] {
        assert_eq!(tools.len(), 3);
        assert!(tools.contains(&"git.status".to_owned()));
        assert!(tools.contains(&"git.diff".to_owned()));
        assert!(tools.contains(&"git.log".to_owned()));
    } else {
        panic!("expected ToolProvider capability");
    }
}

// ── (4) Register in StdioPluginHost — Discovered state ───────────────────

#[test]
fn register_in_host_sets_discovered_state() {
    let mut host = StdioPluginHost::new();
    host.register(tool_provider_manifest("com.test.host", &["host.tool"]))
        .unwrap();

    assert_eq!(
        host.state("com.test.host"),
        Some(PluginState::Discovered),
        "plugin must be in Discovered state after register"
    );
}

#[test]
fn host_and_registry_track_same_plugin_independently() {
    let registry = InMemoryPluginRegistry::new();
    let mut host = StdioPluginHost::new();
    let m = tool_provider_manifest("com.test.dual", &["dual.tool"]);

    registry.register(m.clone()).unwrap();
    host.register(m).unwrap();

    assert!(registry.get("com.test.dual").is_some());
    assert_eq!(host.state("com.test.dual"), Some(PluginState::Discovered));
}

// ── (5)+(6) Unregister — no longer in registry ───────────────────────────

#[test]
fn unregister_removes_plugin_from_registry() {
    let registry = InMemoryPluginRegistry::new();
    registry
        .register(tool_provider_manifest("com.test.remove", &["r.tool"]))
        .unwrap();
    assert!(registry.get("com.test.remove").is_some());

    registry.unregister("com.test.remove").unwrap();

    assert!(
        registry.get("com.test.remove").is_none(),
        "plugin must be absent after unregister"
    );
    assert!(!registry
        .list_all()
        .iter()
        .any(|m| m.id == "com.test.remove"));
}

// ── (7) Duplicate registration rejected ──────────────────────────────────

#[test]
fn duplicate_register_returns_already_registered_error() {
    let registry = InMemoryPluginRegistry::new();
    registry
        .register(tool_provider_manifest("com.test.dup", &["d.tool"]))
        .unwrap();

    let result = registry.register(tool_provider_manifest("com.test.dup", &["d.tool"]));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        cairn_tools::RegistryError::AlreadyRegistered(id) if id == "com.test.dup"
    ));
}

#[test]
fn host_duplicate_register_returns_error() {
    let mut host = StdioPluginHost::new();
    host.register(tool_provider_manifest("com.test.hostdup", &["t"]))
        .unwrap();

    let result = host.register(tool_provider_manifest("com.test.hostdup", &["t"]));
    assert!(result.is_err(), "host must reject duplicate register");
}

// ── (8) Unregister non-existent returns NotFound ──────────────────────────

#[test]
fn unregister_nonexistent_returns_not_found_error() {
    let registry = InMemoryPluginRegistry::new();

    let result = registry.unregister("com.test.ghost");
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        cairn_tools::RegistryError::NotFound(_)
    ));
}

// ── (9) Multiple plugins all listed ──────────────────────────────────────

#[test]
fn multiple_plugins_all_appear_in_list_all() {
    let registry = InMemoryPluginRegistry::new();
    let ids = ["com.test.a", "com.test.b", "com.test.c"];
    for id in &ids {
        registry
            .register(tool_provider_manifest(id, &["tool"]))
            .unwrap();
    }

    let all = registry.list_all();
    assert_eq!(all.len(), 3);
    for id in &ids {
        assert!(
            all.iter().any(|m| m.id == *id),
            "plugin {id} must appear in list_all"
        );
    }
}

#[test]
fn registry_is_empty_after_all_plugins_unregistered() {
    let registry = InMemoryPluginRegistry::new();
    registry
        .register(tool_provider_manifest("com.test.x", &["x"]))
        .unwrap();
    registry
        .register(tool_provider_manifest("com.test.y", &["y"]))
        .unwrap();

    registry.unregister("com.test.x").unwrap();
    registry.unregister("com.test.y").unwrap();

    assert!(registry.list_all().is_empty());
}

// ── Metadata fields preserved ─────────────────────────────────────────────

#[test]
fn description_and_homepage_are_stored_in_registry() {
    let registry = InMemoryPluginRegistry::new();
    registry
        .register(manifest(
            "com.test.meta",
            vec![PluginCapability::EvalScorer],
        ))
        .unwrap();

    let found = registry.get("com.test.meta").unwrap();
    assert_eq!(
        found.description.as_deref(),
        Some("Test plugin for lifecycle validation")
    );
    assert_eq!(
        found.homepage.as_deref(),
        Some("https://example.test/plugin")
    );
}
