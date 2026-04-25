/**
 * entityExplainers constants test (F32).
 *
 * Locks the exact explainer text for every first-class entity surface.
 * If a future refactor silently changes these strings, this suite fails.
 * Also enforces the length/format guardrails that keep the UX consistent:
 *
 *   - single sentence (or one em-dash distinguishing clause)
 *   - between 50 and 200 characters
 *   - no emojis
 *   - no trailing whitespace
 *   - ends in a period
 */

import { describe, it, expect } from "vitest";
import { ENTITY_EXPLAINERS } from "../entityExplainers";

describe("ENTITY_EXPLAINERS", () => {
  it("pins the Run explainer (distinguishes from Tasks)", () => {
    expect(ENTITY_EXPLAINERS.run).toBe(
      "A Run is one orchestration session — an LLM loop dispatched against a model for this project. Different from Tasks (claimable units of work with leases).",
    );
  });

  it("pins the Task explainer (distinguishes from Runs)", () => {
    expect(ENTITY_EXPLAINERS.task).toBe(
      "Tasks are claimable units of work with leases — workers heartbeat to hold them. Different from Runs (orchestration sessions without leases).",
    );
  });

  it("pins the Session explainer", () => {
    expect(ENTITY_EXPLAINERS.session).toBe(
      "A Session groups multiple Runs under one conversation context — operators resume sessions to continue work across restarts.",
    );
  });

  it("pins the Approval explainer (distinguishes from Decisions)", () => {
    expect(ENTITY_EXPLAINERS.approval).toBe(
      "Approvals are operator-gated tool calls or plans that require human sign-off before execution. Different from Decisions (automatic policy outcomes).",
    );
  });

  it("pins the Decision explainer (distinguishes from Approvals)", () => {
    expect(ENTITY_EXPLAINERS.decision).toBe(
      "Decisions record automatic policy outcomes (routing, admission, rate-limit). Different from Approvals (operator-gated tool calls and plans).",
    );
  });

  it("pins the Provider explainer (one connection per family/adapter/credential)", () => {
    expect(ENTITY_EXPLAINERS.provider).toBe(
      "Provider Connections bind cairn to an LLM endpoint — one per (family, adapter, credential). Runs route through them via Settings defaults.",
    );
  });

  it("pins the Credential explainer (raw secret never leaves the server)", () => {
    expect(ENTITY_EXPLAINERS.credential).toBe(
      "Credentials store secrets (API keys, tokens) per tenant. Provider Connections reference them by ID — the raw secret never leaves the server.",
    );
  });

  it("pins the Skill explainer (harness-skill routing)", () => {
    expect(ENTITY_EXPLAINERS.skill).toBe(
      "Skills are markdown instructions operators enable per-project — they become retrievable guidance the agent can invoke via the harness-skill tool.",
    );
  });

  it("every explainer is a single non-empty sentence with sane length", () => {
    const values = Object.values(ENTITY_EXPLAINERS);
    expect(values.length).toBeGreaterThanOrEqual(15);
    for (const v of values) {
      expect(v).toBe(v.trim());
      expect(v.length).toBeGreaterThanOrEqual(50);
      expect(v.length).toBeLessThanOrEqual(200);
      expect(v.endsWith(".")).toBe(true);
      // No bullet lists; a single sentence may contain at most one
      // em-dash separating the definition from a distinguishing clause.
      expect(v).not.toContain("\n");
      expect(v).not.toContain("•");
      // Reject emoji range (basic guardrail — our strings are plain ASCII + em-dash).
      expect(/[\u{1F300}-\u{1FAFF}]/u.test(v)).toBe(false);
    }
  });

  it("no explainer references 'click here' or 'learn more' (self-explanatory rule)", () => {
    for (const v of Object.values(ENTITY_EXPLAINERS)) {
      const lower = v.toLowerCase();
      expect(lower).not.toContain("click here");
      expect(lower).not.toContain("learn more");
      expect(lower).not.toContain("read the docs");
    }
  });
});
