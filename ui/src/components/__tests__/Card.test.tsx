import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { Card, CardHeader } from "../Card";

describe("Card", () => {
  it("renders children", () => {
    render(<Card>Card content</Card>);
    expect(screen.getByText("Card content")).toBeInTheDocument();
  });

  it("renders default variant with padding and rounded-lg", () => {
    const { container } = render(<Card>Default</Card>);
    const card = container.firstElementChild!;
    expect(card.className).toContain("p-4");
    expect(card.className).toContain("rounded-lg");
    expect(card.className).toContain("border");
  });

  it("renders shell variant without padding", () => {
    const { container } = render(<Card variant="shell">Shell</Card>);
    const card = container.firstElementChild!;
    expect(card.className).not.toContain("p-4");
    expect(card.className).toContain("overflow-hidden");
    expect(card.className).toContain("rounded-lg");
  });

  it("renders inner variant with elevated background", () => {
    const { container } = render(<Card variant="inner">Inner</Card>);
    const card = container.firstElementChild!;
    expect(card.className).toContain("bg-white");
    expect(card.className).toContain("overflow-hidden");
  });

  it("applies custom className", () => {
    const { container } = render(<Card className="mt-4">Custom</Card>);
    const card = container.firstElementChild!;
    expect(card.className).toContain("mt-4");
  });
});

describe("CardHeader", () => {
  it("renders title text", () => {
    render(<CardHeader>My Section</CardHeader>);
    expect(screen.getByText("My Section")).toBeInTheDocument();
  });

  it("renders actions slot", () => {
    render(
      <CardHeader actions={<button>Action</button>}>
        Title
      </CardHeader>,
    );
    expect(screen.getByText("Title")).toBeInTheDocument();
    expect(screen.getByText("Action")).toBeInTheDocument();
  });

  it("renders with border-b and uppercase styling", () => {
    const { container } = render(<CardHeader>Header</CardHeader>);
    const header = container.firstElementChild!;
    expect(header.className).toContain("border-b");
    const label = header.querySelector("p")!;
    expect(label.className).toContain("uppercase");
    expect(label.className).toContain("tracking-wider");
  });

  it("applies custom className", () => {
    const { container } = render(<CardHeader className="bg-red-500">Test</CardHeader>);
    const header = container.firstElementChild!;
    expect(header.className).toContain("bg-red-500");
  });
});
