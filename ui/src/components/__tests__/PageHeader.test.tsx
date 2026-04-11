import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { PageHeader } from "../PageHeader";

describe("PageHeader", () => {
  it("renders title", () => {
    render(<PageHeader title="Dashboard" />);
    expect(screen.getByText("Dashboard")).toBeInTheDocument();
  });

  it("renders subtitle when provided", () => {
    render(<PageHeader title="Runs" subtitle="Active agent runs" />);
    expect(screen.getByText("Runs")).toBeInTheDocument();
    expect(screen.getByText("Active agent runs")).toBeInTheDocument();
  });

  it("does not render subtitle when omitted", () => {
    const { container } = render(<PageHeader title="Simple" />);
    // Only the title text should be present, not a subtitle p
    const paragraphs = container.querySelectorAll("p");
    // Should not have subtitle paragraph
    const texts = Array.from(paragraphs).map((p) => p.textContent);
    expect(texts).not.toContain("");
  });

  it("renders section label above title", () => {
    render(<PageHeader title="Costs" sectionLabel="Cost Tracking" />);
    expect(screen.getByText("Cost Tracking")).toBeInTheDocument();
    expect(screen.getByText("Costs")).toBeInTheDocument();
  });

  it("renders actions slot", () => {
    render(
      <PageHeader
        title="Test"
        actions={<button>Refresh</button>}
      />,
    );
    expect(screen.getByText("Refresh")).toBeInTheDocument();
  });

  it("renders status indicator", () => {
    render(
      <PageHeader
        title="Deploy"
        status={<span data-testid="status">v1.0</span>}
      />,
    );
    expect(screen.getByTestId("status")).toBeInTheDocument();
  });

  it("renders back button and calls onBack", () => {
    const onBack = vi.fn();
    render(<PageHeader title="Detail" onBack={onBack} />);
    const backBtn = screen.getByText("Back");
    expect(backBtn).toBeInTheDocument();
    fireEvent.click(backBtn);
    expect(onBack).toHaveBeenCalledOnce();
  });

  it("uses custom back label", () => {
    render(<PageHeader title="Detail" onBack={() => {}} backLabel="Go back" />);
    expect(screen.getByText("Go back")).toBeInTheDocument();
  });

  it("does not render back button when onBack is omitted", () => {
    render(<PageHeader title="No Back" />);
    expect(screen.queryByText("Back")).not.toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(<PageHeader title="Custom" className="mt-8" />);
    expect(container.firstElementChild!.className).toContain("mt-8");
  });

  it("renders all props together", () => {
    const onBack = vi.fn();
    render(
      <PageHeader
        title="Full Header"
        subtitle="With everything"
        sectionLabel="Section"
        onBack={onBack}
        backLabel="Return"
        actions={<button>Action</button>}
        status={<span>Online</span>}
      />,
    );
    expect(screen.getByText("Full Header")).toBeInTheDocument();
    expect(screen.getByText("With everything")).toBeInTheDocument();
    expect(screen.getByText("Section")).toBeInTheDocument();
    expect(screen.getByText("Return")).toBeInTheDocument();
    expect(screen.getByText("Action")).toBeInTheDocument();
    expect(screen.getByText("Online")).toBeInTheDocument();
  });
});
