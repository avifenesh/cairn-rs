import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { StatCard } from "../StatCard";

describe("StatCard", () => {
  it("renders label and value", () => {
    render(<StatCard label="Total" value={42} />);
    expect(screen.getByText("Total")).toBeInTheDocument();
    expect(screen.getByText("42")).toBeInTheDocument();
  });

  it("renders description when provided", () => {
    render(<StatCard label="Calls" value={10} description="last 24h" />);
    expect(screen.getByText("last 24h")).toBeInTheDocument();
  });

  it("renders string values", () => {
    render(<StatCard label="Cost" value="$1.23" />);
    expect(screen.getByText("$1.23")).toBeInTheDocument();
  });

  // Variant accent classes
  const variants = ["default", "success", "warning", "danger", "info"] as const;

  for (const variant of variants) {
    it(`renders ${variant} variant with correct accent`, () => {
      const { container } = render(
        <StatCard label={`${variant} card`} value={1} variant={variant} />,
      );
      const card = container.querySelector("[data-stat-card]")!;
      expect(card).toBeInTheDocument();
      // Each variant applies a border-l accent class
      expect(card.className).toContain("border-l-");
    });
  }

  it("renders compact mode without card background", () => {
    const { container } = render(
      <StatCard label="Compact" value={99} compact />,
    );
    const card = container.querySelector("[data-stat-card]")!;
    expect(card).toBeInTheDocument();
    // Compact mode: border-l-2 + pl-3, no rounded-lg or p-4
    expect(card.className).toContain("border-l-2");
    expect(card.className).toContain("pl-3");
    expect(card.className).not.toContain("rounded-lg");
  });

  it("renders compact mode with variant accent", () => {
    const { container } = render(
      <StatCard label="Compact Info" value={5} compact variant="info" />,
    );
    const card = container.querySelector("[data-stat-card]")!;
    expect(card.className).toContain("border-l-indigo-500");
  });

  it("renders loading state (standard)", () => {
    const { container } = render(
      <StatCard label="Loading" value={0} loading />,
    );
    // Loading state renders animate-pulse skeleton, not the actual label
    expect(container.querySelector(".animate-pulse")).toBeInTheDocument();
    expect(screen.queryByText("Loading")).not.toBeInTheDocument();
  });

  it("renders loading state (compact)", () => {
    const { container } = render(
      <StatCard label="Loading" value={0} loading compact />,
    );
    expect(container.querySelector(".animate-pulse")).toBeInTheDocument();
    expect(screen.queryByText("Loading")).not.toBeInTheDocument();
  });

  it("uses data-stat-label and data-stat-value attributes", () => {
    const { container } = render(<StatCard label="Metric" value={7} />);
    expect(container.querySelector("[data-stat-label]")).toBeInTheDocument();
    expect(container.querySelector("[data-stat-value]")).toBeInTheDocument();
  });

  it("applies custom className to standard card", () => {
    const { container } = render(<StatCard label="Custom" value={1} className="ml-4" />);
    const card = container.querySelector("[data-stat-card]")!;
    expect(card.className).toContain("ml-4");
  });

  it("applies custom className to compact card", () => {
    const { container } = render(<StatCard label="Custom" value={1} compact className="mr-2" />);
    const card = container.querySelector("[data-stat-card]")!;
    expect(card.className).toContain("mr-2");
  });

  it("renders data-stat-card on loading state (standard)", () => {
    const { container } = render(<StatCard label="Loading" value={0} loading />);
    expect(container.querySelector("[data-stat-card]")).toBeInTheDocument();
  });

  it("renders data-stat-card on loading state (compact)", () => {
    const { container } = render(<StatCard label="Loading" value={0} loading compact />);
    expect(container.querySelector("[data-stat-card]")).toBeInTheDocument();
  });

  it("uses consistent text-[20px] for value in both modes", () => {
    const { container: stdContainer } = render(<StatCard label="Std" value={42} />);
    const stdValue = stdContainer.querySelector("[data-stat-value]")!;
    expect(stdValue.className).toContain("text-[20px]");

    const { container: compactContainer } = render(<StatCard label="Cmp" value={42} compact />);
    const cmpValue = compactContainer.querySelector("[data-stat-value]")!;
    expect(cmpValue.className).toContain("text-[20px]");
  });

  it("renders help tooltip when help prop is provided", () => {
    const { container } = render(<StatCard label="With Help" value={5} help="Tooltip text" />);
    // HelpTooltip renders an element with the help text
    const label = container.querySelector("[data-stat-label]")!;
    // Should have more than just the label text (tooltip element too)
    expect(label.childNodes.length).toBeGreaterThan(1);
  });
});
