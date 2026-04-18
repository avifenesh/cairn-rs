#![cfg(feature = "in-memory-runtime")]

//! Integration test: skill health matrix stub returns empty rows.
//!
//! `build_skill_health_matrix` is currently a stub that always returns an empty
//! `SkillHealthMatrix`. These tests verify the stub compiles and returns the
//! expected empty shape. When the implementation is fleshed out, expand with
//! event-driven assertions.

use cairn_evals::matrices::SkillHealthMatrix;
use cairn_evals::EvalRunService;

#[tokio::test]
async fn skill_health_matrix_stub_returns_empty() {
    let svc = EvalRunService::new();
    let tenant_id = cairn_domain::TenantId::new("test_tenant");

    let matrix: SkillHealthMatrix = svc.build_skill_health_matrix(&tenant_id).await.unwrap();
    assert!(matrix.rows.is_empty(), "stub returns empty matrix");
}
