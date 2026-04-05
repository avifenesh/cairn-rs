//! Skills catalog integration tests (GAP-012).
//!
//! Validates the `SkillCatalog` + `SkillInvocation` pipeline end-to-end,
//! proving the skill marketplace compiles, registration works correctly,
//! filtering by tag returns the right subset, enable/disable toggles are
//! durable, and invocation status tracking round-trips through all states.
//!
//! Skill lifecycle:   Proposed (disabled) → Active (enabled) → disabled again
//! Invocation states: Running → Completed | Failed

use cairn_domain::skills::{
    Skill, SkillCatalog, SkillInvocation, SkillInvocationStatus, SkillStatus,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn skill(id: &str, tags: &[&str]) -> Skill {
    Skill {
        skill_id: id.to_owned(),
        name: format!("{id} skill"),
        description: format!("Invoke when the user needs {id}"),
        version: "1.0.0".to_owned(),
        entry_point: format!("skills/{id}/main.md"),
        required_permissions: vec![],
        tags: tags.iter().map(|t| t.to_string()).collect(),
        enabled: false,
        status: SkillStatus::Proposed,
    }
}

fn invocation(id: &str, skill_id: &str) -> SkillInvocation {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    SkillInvocation {
        invocation_id: id.to_owned(),
        skill_id: skill_id.to_owned(),
        args: serde_json::json!({}),
        result: None,
        status: SkillInvocationStatus::Running,
        created_at_ms: ts,
        updated_at_ms: ts,
    }
}

// ── 1. Create catalog and register 3 skills ───────────────────────────────────

#[test]
fn register_three_skills_catalog_has_three_entries() {
    let mut catalog = SkillCatalog::new();
    assert!(catalog.is_empty());

    catalog.register(skill("skill_a", &["coding", "review"]));
    catalog.register(skill("skill_b", &["research", "writing"]));
    catalog.register(skill("skill_c", &["coding", "data"]));

    assert_eq!(catalog.len(), 3);
    assert!(!catalog.is_empty());
}

// ── 2. list() returns all 3, sorted by skill_id ───────────────────────────────

#[test]
fn list_returns_all_skills_sorted_by_id() {
    let mut catalog = SkillCatalog::new();

    // Register in reverse order to verify sorting.
    catalog.register(skill("skill_c", &["data"]));
    catalog.register(skill("skill_a", &["coding"]));
    catalog.register(skill("skill_b", &["research"]));

    let all = catalog.list(&[]);
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].skill_id, "skill_a");
    assert_eq!(all[1].skill_id, "skill_b");
    assert_eq!(all[2].skill_id, "skill_c");
}

// ── 3. get() returns correct skill with all fields ────────────────────────────

#[test]
fn get_returns_skill_with_correct_fields() {
    let mut catalog = SkillCatalog::new();
    let mut s = skill("content-pipeline", &["writing", "publishing"]);
    s.version = "2.1.0".to_owned();
    s.required_permissions = vec!["file:read".to_owned(), "http:get".to_owned()];
    catalog.register(s);

    let found = catalog.get("content-pipeline").expect("must be registered");
    assert_eq!(found.skill_id, "content-pipeline");
    assert_eq!(found.version, "2.1.0");
    assert_eq!(found.required_permissions, vec!["file:read", "http:get"]);
    assert!(found.tags.contains(&"writing".to_owned()));
    assert!(found.tags.contains(&"publishing".to_owned()));
    assert_eq!(found.status, SkillStatus::Proposed, "new skills start as Proposed");
    assert!(!found.enabled, "new skills start disabled");
}

// ── 4. Disable skill_b → get() shows disabled=true, status unchanged ──────────

#[test]
fn disable_skill_b_shows_disabled_in_get() {
    let mut catalog = SkillCatalog::new();
    catalog.register(skill("skill_a", &["coding"]));
    catalog.register(skill("skill_b", &["research"]));
    catalog.register(skill("skill_c", &["data"]));

    // Enable all three first.
    catalog.enable("skill_a");
    catalog.enable("skill_b");
    catalog.enable("skill_c");

    assert!(catalog.get("skill_b").unwrap().enabled);
    assert_eq!(catalog.get("skill_b").unwrap().status, SkillStatus::Active);

    // Disable only skill_b.
    let ok = catalog.disable("skill_b");
    assert!(ok, "disable must return true for a registered skill");

    let b = catalog.get("skill_b").unwrap();
    assert!(!b.enabled, "skill_b must be disabled");
    // Status is NOT reset to Proposed by disable — it remains Active.
    // Re-enabling would keep it Active; only manual status changes affect status.
    assert_eq!(b.status, SkillStatus::Active,
        "disable does not roll back status; it only clears enabled flag");

    // skill_a and skill_c are unaffected.
    assert!(catalog.get("skill_a").unwrap().enabled);
    assert!(catalog.get("skill_c").unwrap().enabled);
}

// ── 5. Enable/disable toggle ─────────────────────────────────────────────────

#[test]
fn enable_disable_toggle_is_durable() {
    let mut catalog = SkillCatalog::new();
    catalog.register(skill("toggle_skill", &["general"]));

    // Initial: disabled + Proposed.
    assert!(!catalog.get("toggle_skill").unwrap().enabled);
    assert_eq!(catalog.get("toggle_skill").unwrap().status, SkillStatus::Proposed);

    // Enable → Active.
    catalog.enable("toggle_skill");
    assert!(catalog.get("toggle_skill").unwrap().enabled);
    assert_eq!(catalog.get("toggle_skill").unwrap().status, SkillStatus::Active);

    // Disable → not enabled (status stays Active).
    catalog.disable("toggle_skill");
    assert!(!catalog.get("toggle_skill").unwrap().enabled);
    assert_eq!(catalog.get("toggle_skill").unwrap().status, SkillStatus::Active);

    // Re-enable → still Active.
    catalog.enable("toggle_skill");
    assert!(catalog.get("toggle_skill").unwrap().enabled);
    assert_eq!(catalog.get("toggle_skill").unwrap().status, SkillStatus::Active);

    // Final enabled() count: only this one.
    assert_eq!(catalog.enabled().len(), 1);
}

// ── 6. enabled() reflects the current enabled set ─────────────────────────────

#[test]
fn enabled_returns_only_currently_enabled_skills() {
    let mut catalog = SkillCatalog::new();
    catalog.register(skill("s_a", &["x"]));
    catalog.register(skill("s_b", &["x"]));
    catalog.register(skill("s_c", &["x"]));

    catalog.enable("s_a");
    catalog.enable("s_c");

    let enabled = catalog.enabled();
    assert_eq!(enabled.len(), 2);
    let ids: Vec<_> = enabled.iter().map(|s| s.skill_id.as_str()).collect();
    assert!(ids.contains(&"s_a"));
    assert!(ids.contains(&"s_c"));
    assert!(!ids.contains(&"s_b"), "s_b was never enabled");

    // Disabling s_a removes it from the enabled set.
    catalog.disable("s_a");
    assert_eq!(catalog.enabled().len(), 1);
    assert_eq!(catalog.enabled()[0].skill_id, "s_c");
}

// ── 7. SkillInvocation — Running → Completed ─────────────────────────────────

#[test]
fn skill_invocation_running_to_completed() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut inv = SkillInvocation {
        invocation_id: "inv_ok".to_owned(),
        skill_id: "content-pipeline".to_owned(),
        args: serde_json::json!({ "topic": "AI safety", "format": "blog" }),
        result: None,
        status: SkillInvocationStatus::Running,
        created_at_ms: ts,
        updated_at_ms: ts,
    };

    assert_eq!(inv.status, SkillInvocationStatus::Running);
    assert!(inv.result.is_none());

    // Skill completes successfully.
    inv.result = Some(serde_json::json!({ "word_count": 800, "draft": "..." }));
    inv.status = SkillInvocationStatus::Completed;
    inv.updated_at_ms = ts + 3_000;

    assert_eq!(inv.status, SkillInvocationStatus::Completed);
    assert!(inv.result.is_some());
    assert_eq!(inv.result.as_ref().unwrap()["word_count"], 800);
    assert!(inv.updated_at_ms > inv.created_at_ms);
}

// ── 8. SkillInvocation — Running → Failed ────────────────────────────────────

#[test]
fn skill_invocation_running_to_failed() {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let mut inv = invocation("inv_fail", "decision-support");
    assert_eq!(inv.status, SkillInvocationStatus::Running);

    // Skill fails with an error payload.
    inv.result = Some(serde_json::json!({ "error": "upstream timeout after 30s" }));
    inv.status = SkillInvocationStatus::Failed;
    inv.updated_at_ms = ts + 30_000;

    assert_eq!(inv.status, SkillInvocationStatus::Failed);
    assert_eq!(
        inv.result.as_ref().unwrap()["error"],
        "upstream timeout after 30s"
    );
}

// ── 9. SkillInvocation status variants are terminal-aware ─────────────────────

#[test]
fn invocation_status_variants_are_all_distinct() {
    assert_ne!(SkillInvocationStatus::Running, SkillInvocationStatus::Completed);
    assert_ne!(SkillInvocationStatus::Running, SkillInvocationStatus::Failed);
    assert_ne!(SkillInvocationStatus::Completed, SkillInvocationStatus::Failed);
}

// ── 10. Tag-based filtering — single tag ──────────────────────────────────────

#[test]
fn tag_filter_single_tag_returns_matching_skills() {
    let mut catalog = SkillCatalog::new();

    catalog.register(skill("coder",    &["coding", "review"]));
    catalog.register(skill("writer",   &["writing", "publishing"]));
    catalog.register(skill("analyst",  &["data", "coding"]));

    let coding_skills = catalog.list(&["coding"]);
    assert_eq!(coding_skills.len(), 2, "coder and analyst both have 'coding'");
    let ids: Vec<_> = coding_skills.iter().map(|s| s.skill_id.as_str()).collect();
    assert!(ids.contains(&"coder"));
    assert!(ids.contains(&"analyst"));
    assert!(!ids.contains(&"writer"));

    let writing_skills = catalog.list(&["writing"]);
    assert_eq!(writing_skills.len(), 1);
    assert_eq!(writing_skills[0].skill_id, "writer");
}

// ── 11. Tag-based filtering — multi-tag AND semantics ────────────────────────

#[test]
fn tag_filter_multi_tag_requires_all_tags_present() {
    let mut catalog = SkillCatalog::new();

    // Only specialist has both "coding" AND "review".
    catalog.register(skill("specialist",   &["coding", "review", "security"]));
    catalog.register(skill("generalist",   &["coding", "writing"]));
    catalog.register(skill("reviewer",     &["review", "writing"]));

    // AND filter: must have BOTH "coding" AND "review".
    let both = catalog.list(&["coding", "review"]);
    assert_eq!(both.len(), 1, "only specialist has both tags");
    assert_eq!(both[0].skill_id, "specialist");

    // Three-tag AND: only specialist has all three.
    let all_three = catalog.list(&["coding", "review", "security"]);
    assert_eq!(all_three.len(), 1);
    assert_eq!(all_three[0].skill_id, "specialist");

    // Impossible combination: no skill has both "data" and "coding".
    let impossible = catalog.list(&["data", "coding"]);
    assert!(impossible.is_empty());
}

// ── 12. Tag-based filtering — no filter returns all ───────────────────────────

#[test]
fn tag_filter_empty_slice_returns_all_skills() {
    let mut catalog = SkillCatalog::new();
    catalog.register(skill("s1", &["a"]));
    catalog.register(skill("s2", &["b"]));
    catalog.register(skill("s3", &[]));   // no tags at all

    let all = catalog.list(&[]);
    assert_eq!(all.len(), 3, "no filter returns all including tag-less skills");
}

// ── 13. Tag-based filtering — no match returns empty ─────────────────────────

#[test]
fn tag_filter_unknown_tag_returns_empty() {
    let mut catalog = SkillCatalog::new();
    catalog.register(skill("s1", &["coding"]));
    catalog.register(skill("s2", &["research"]));

    let none = catalog.list(&["nonexistent-tag"]);
    assert!(none.is_empty(), "unknown tag must return empty slice");
}

// ── 14. Skill::new() constructor sets correct defaults ────────────────────────

#[test]
fn skill_new_constructor_sets_proposed_and_disabled() {
    let skill = Skill::new(
        "scaffolder",
        "Project Scaffolder",
        "Use when user wants to bootstrap a new project",
        "1.0.0",
        "skills/scaffolder/main.md",
    );

    assert_eq!(skill.skill_id, "scaffolder");
    assert_eq!(skill.name, "Project Scaffolder");
    assert_eq!(skill.version, "1.0.0");
    assert_eq!(skill.status, SkillStatus::Proposed, "new() defaults to Proposed");
    assert!(!skill.enabled, "new() defaults to disabled");
    assert!(skill.tags.is_empty(), "new() has no tags by default");
    assert!(skill.required_permissions.is_empty());
}

// ── 15. SkillStatus variants are distinct ────────────────────────────────────

#[test]
fn skill_status_variants_are_distinct() {
    assert_ne!(SkillStatus::Active, SkillStatus::Proposed);
    assert_ne!(SkillStatus::Active, SkillStatus::Rejected);
    assert_ne!(SkillStatus::Proposed, SkillStatus::Rejected);
}

// ── 16. enable/disable on unknown skill returns false ─────────────────────────

#[test]
fn enable_disable_unknown_skill_returns_false() {
    let mut catalog = SkillCatalog::new();
    assert!(!catalog.enable("ghost"),  "enable unknown returns false");
    assert!(!catalog.disable("ghost"), "disable unknown returns false");
}

// ── 17. Full marketplace workflow: register → enable → invoke → complete ──────

#[test]
fn full_skill_marketplace_workflow() {
    let mut catalog = SkillCatalog::new();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Register three skills with distinct tags.
    let mut decision_support = skill("decision-support", &["reasoning", "analysis"]);
    decision_support.required_permissions = vec!["file:read".to_owned()];
    catalog.register(decision_support);

    let mut code_reviewer = skill("code-reviewer", &["coding", "review"]);
    code_reviewer.required_permissions = vec!["file:read".to_owned(), "shell:exec".to_owned()];
    catalog.register(code_reviewer);

    catalog.register(skill("research-agent", &["research", "analysis"]));

    // Verify all 3 registered.
    assert_eq!(catalog.len(), 3);
    assert_eq!(catalog.list(&[]).len(), 3);

    // Enable two, leave research-agent disabled.
    catalog.enable("decision-support");
    catalog.enable("code-reviewer");

    assert_eq!(catalog.enabled().len(), 2);
    assert!(!catalog.get("research-agent").unwrap().enabled);

    // Tag filter: "analysis" matches decision-support and research-agent.
    let analysis = catalog.list(&["analysis"]);
    assert_eq!(analysis.len(), 2);

    // Tag filter: "coding" AND "review" → only code-reviewer.
    let review_coding = catalog.list(&["coding", "review"]);
    assert_eq!(review_coding.len(), 1);
    assert_eq!(review_coding[0].skill_id, "code-reviewer");

    // Invoke decision-support.
    let mut inv = SkillInvocation {
        invocation_id: "inv_workflow_1".to_owned(),
        skill_id: "decision-support".to_owned(),
        args: serde_json::json!({ "question": "Should we deploy on Friday?", "context": "low risk" }),
        result: None,
        status: SkillInvocationStatus::Running,
        created_at_ms: ts,
        updated_at_ms: ts,
    };
    assert_eq!(inv.status, SkillInvocationStatus::Running);

    // Skill completes.
    inv.result = Some(serde_json::json!({ "recommendation": "yes", "confidence": 0.85 }));
    inv.status = SkillInvocationStatus::Completed;
    inv.updated_at_ms = ts + 1_500;

    assert_eq!(inv.status, SkillInvocationStatus::Completed);
    assert_eq!(inv.result.as_ref().unwrap()["recommendation"], "yes");

    // Disable decision-support after use.
    catalog.disable("decision-support");
    assert!(!catalog.get("decision-support").unwrap().enabled);
    assert_eq!(catalog.enabled().len(), 1, "only code-reviewer remains enabled");
}
