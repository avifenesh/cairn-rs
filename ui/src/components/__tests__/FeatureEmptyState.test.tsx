import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { FeatureEmptyState } from "../FeatureEmptyState";

describe("FeatureEmptyState", () => {
  it("renders icon, title, and description", () => {
    render(
      <FeatureEmptyState
        icon={<span data-testid="icon">IC</span>}
        title="No data yet"
        description="Configure a provider to get started."
      />,
    );
    expect(screen.getByTestId("icon")).toBeInTheDocument();
    expect(screen.getByText("No data yet")).toBeInTheDocument();
    expect(screen.getByText("Configure a provider to get started.")).toBeInTheDocument();
  });

  it("renders action link when both label and href provided", () => {
    render(
      <FeatureEmptyState
        icon={<span>IC</span>}
        title="Empty"
        description="Desc"
        actionLabel="Go to Providers"
        actionHref="#providers"
      />,
    );
    const link = screen.getByText("Go to Providers");
    expect(link).toBeInTheDocument();
    expect(link.tagName).toBe("A");
    expect(link).toHaveAttribute("href", "#providers");
  });

  it("does not render action link when label is omitted", () => {
    render(
      <FeatureEmptyState
        icon={<span>IC</span>}
        title="Empty"
        description="Desc"
        actionHref="#providers"
      />,
    );
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("does not render action link when href is omitted", () => {
    render(
      <FeatureEmptyState
        icon={<span>IC</span>}
        title="Empty"
        description="Desc"
        actionLabel="Go"
      />,
    );
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(
      <FeatureEmptyState
        icon={<span>IC</span>}
        title="Empty"
        description="Desc"
        className="mt-8"
      />,
    );
    expect(container.firstElementChild!.className).toContain("mt-8");
  });

  it("renders icon inside a bordered container", () => {
    const { container } = render(
      <FeatureEmptyState
        icon={<span data-testid="icon">IC</span>}
        title="Empty"
        description="Desc"
      />,
    );
    const iconBox = container.querySelector(".rounded-xl");
    expect(iconBox).toBeInTheDocument();
    expect(iconBox!.className).toContain("border");
  });
});
