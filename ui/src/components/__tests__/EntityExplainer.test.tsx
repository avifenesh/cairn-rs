/**
 * EntityExplainer component tests (F32).
 *
 * Verifies the shared one-line entity explainer renders the text it is
 * given, applies muted-foreground styling, and honours the `testId`
 * override. These tests protect the styling contract — page-level tests
 * (in `ui/src/pages/__tests__/`) lock the actual strings in
 * `entityExplainers.ts`.
 */

import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { EntityExplainer } from "../EntityExplainer";
import { ENTITY_EXPLAINERS } from "../../lib/entityExplainers";

describe("EntityExplainer", () => {
  it("renders the given text inside a <p>", () => {
    render(<EntityExplainer>Hello, world.</EntityExplainer>);
    const el = screen.getByText("Hello, world.");
    expect(el.tagName).toBe("P");
  });

  it("defaults to data-testid 'entity-explainer'", () => {
    render(<EntityExplainer>foo</EntityExplainer>);
    expect(screen.getByTestId("entity-explainer")).toBeInTheDocument();
  });

  it("honours a custom testId override", () => {
    render(<EntityExplainer testId="custom-id">foo</EntityExplainer>);
    expect(screen.getByTestId("custom-id")).toBeInTheDocument();
  });

  it("applies muted 11px styling", () => {
    render(<EntityExplainer>styled</EntityExplainer>);
    const el = screen.getByText("styled");
    expect(el.className).toContain("text-[11px]");
    expect(el.className).toContain("text-gray-500");
    expect(el.className).toContain("dark:text-zinc-500");
  });

  it("merges extra className prop", () => {
    render(<EntityExplainer className="mt-4">x</EntityExplainer>);
    const el = screen.getByText("x");
    expect(el.className).toContain("mt-4");
  });

  it("renders a real ENTITY_EXPLAINERS string unmodified", () => {
    const text = ENTITY_EXPLAINERS.run;
    render(<EntityExplainer>{text}</EntityExplainer>);
    expect(screen.getByText(text)).toBeInTheDocument();
  });
});
