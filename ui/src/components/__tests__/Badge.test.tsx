import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { Badge } from "../Badge";

describe("Badge", () => {
  it("renders children text", () => {
    render(<Badge>Active</Badge>);
    expect(screen.getByText("Active")).toBeInTheDocument();
  });

  // All 8 variants
  const variants = [
    "success", "warning", "danger", "info",
    "purple", "sky", "neutral", "muted",
  ] as const;

  for (const variant of variants) {
    it(`renders ${variant} variant`, () => {
      render(<Badge variant={variant}>{variant}</Badge>);
      expect(screen.getByText(variant)).toBeInTheDocument();
    });
  }

  it("defaults to neutral variant", () => {
    const { container } = render(<Badge>Default</Badge>);
    const badge = container.firstElementChild!;
    // Neutral fills include text-gray-500
    expect(badge.className).toContain("text-gray");
  });

  it("renders dot indicator when dot=true", () => {
    const { container } = render(<Badge dot variant="success">OK</Badge>);
    const dot = container.querySelector(".rounded-full");
    expect(dot).toBeInTheDocument();
    expect(dot!.className).toContain("bg-emerald-500");
  });

  it("renders animated dot when dotPulse=true", () => {
    const { container } = render(<Badge dot dotPulse variant="danger">Error</Badge>);
    const dot = container.querySelector(".rounded-full");
    expect(dot).toBeInTheDocument();
    expect(dot!.className).toContain("animate-pulse");
  });

  it("does not render dot when dot=false", () => {
    const { container } = render(<Badge variant="success">OK</Badge>);
    // The only element should be the badge span itself, no child dot
    const dots = container.querySelectorAll(".w-1\\.5");
    expect(dots.length).toBe(0);
  });

  it("renders outlined style with border", () => {
    const { container } = render(<Badge outlined variant="info">Info</Badge>);
    const badge = container.firstElementChild!;
    expect(badge.className).toContain("border");
  });

  it("renders compact sizing", () => {
    const { container } = render(<Badge compact>Small</Badge>);
    const badge = container.firstElementChild!;
    expect(badge.className).toContain("text-[10px]");
  });

  it("renders standard sizing by default", () => {
    const { container } = render(<Badge>Normal</Badge>);
    const badge = container.firstElementChild!;
    expect(badge.className).toContain("text-[11px]");
  });

  it("applies custom className", () => {
    const { container } = render(<Badge className="ml-2">Custom</Badge>);
    const badge = container.firstElementChild!;
    expect(badge.className).toContain("ml-2");
  });

  it("combines outlined + compact + dot", () => {
    const { container } = render(
      <Badge outlined compact dot variant="warning">Warn</Badge>,
    );
    const badge = container.firstElementChild!;
    expect(badge.className).toContain("border");
    expect(badge.className).toContain("text-[10px]");
    const dot = container.querySelector(".rounded-full");
    expect(dot).toBeInTheDocument();
  });
});
