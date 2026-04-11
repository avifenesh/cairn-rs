import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ActionButton } from "../ActionButton";

describe("ActionButton", () => {
  it("renders children text", () => {
    render(<ActionButton>Click me</ActionButton>);
    expect(screen.getByText("Click me")).toBeInTheDocument();
  });

  it("calls onClick when clicked", () => {
    const handler = vi.fn();
    render(<ActionButton onClick={handler}>Go</ActionButton>);
    fireEvent.click(screen.getByText("Go"));
    expect(handler).toHaveBeenCalledOnce();
  });

  // 4 variants
  const variants = ["primary", "secondary", "danger", "ghost"] as const;

  for (const variant of variants) {
    it(`renders ${variant} variant`, () => {
      const { container } = render(
        <ActionButton variant={variant}>{variant}</ActionButton>,
      );
      const btn = container.querySelector("button")!;
      expect(btn).toBeInTheDocument();
      // Each variant has a distinct class
      if (variant === "primary") expect(btn.className).toContain("bg-indigo-600");
      if (variant === "secondary") expect(btn.className).toContain("dark:bg-zinc-900");
      if (variant === "danger") expect(btn.className).toContain("bg-red-600");
      if (variant === "ghost") expect(btn.className).toContain("border-transparent");
    });
  }

  // 3 sizes
  const sizes = ["sm", "md", "lg"] as const;

  for (const size of sizes) {
    it(`renders ${size} size`, () => {
      const { container } = render(
        <ActionButton size={size}>Sized</ActionButton>,
      );
      const btn = container.querySelector("button")!;
      if (size === "sm") expect(btn.className).toContain("text-[11px]");
      if (size === "md") expect(btn.className).toContain("text-[12px]");
      if (size === "lg") expect(btn.className).toContain("text-[13px]");
    });
  }

  it("renders loading state with spinner and disables button", () => {
    const handler = vi.fn();
    const { container } = render(
      <ActionButton loading onClick={handler}>Save</ActionButton>,
    );
    const btn = container.querySelector("button")!;
    expect(btn).toBeDisabled();
    expect(container.querySelector(".animate-spin")).toBeInTheDocument();
    fireEvent.click(btn);
    expect(handler).not.toHaveBeenCalled();
  });

  it("renders disabled state", () => {
    const handler = vi.fn();
    const { container } = render(
      <ActionButton disabled onClick={handler}>Disabled</ActionButton>,
    );
    const btn = container.querySelector("button")!;
    expect(btn).toBeDisabled();
    expect(btn.className).toContain("disabled:opacity-50");
  });

  it("renders icon before children", () => {
    const { container } = render(
      <ActionButton icon={<span data-testid="icon">+</span>}>Add</ActionButton>,
    );
    expect(screen.getByTestId("icon")).toBeInTheDocument();
    expect(screen.getByText("Add")).toBeInTheDocument();
    // Icon should come before text in DOM order
    const btn = container.querySelector("button")!;
    const children = Array.from(btn.childNodes);
    const iconIdx = children.findIndex(
      (n) => n instanceof HTMLElement && n.dataset.testid === "icon",
    );
    const textIdx = children.findIndex(
      (n) => n.textContent === "Add" && !(n instanceof HTMLElement && n.dataset.testid),
    );
    expect(iconIdx).toBeLessThan(textIdx);
  });

  it("prefers loading spinner over icon", () => {
    const { container } = render(
      <ActionButton loading icon={<span data-testid="icon">+</span>}>Save</ActionButton>,
    );
    expect(container.querySelector(".animate-spin")).toBeInTheDocument();
    expect(screen.queryByTestId("icon")).not.toBeInTheDocument();
  });

  it("defaults to type=button", () => {
    const { container } = render(<ActionButton>Btn</ActionButton>);
    expect(container.querySelector("button")!.type).toBe("button");
  });

  it("accepts type=submit", () => {
    const { container } = render(<ActionButton type="submit">Submit</ActionButton>);
    expect(container.querySelector("button")!.type).toBe("submit");
  });

  it("passes title attribute", () => {
    render(<ActionButton title="Save changes">Save</ActionButton>);
    expect(screen.getByTitle("Save changes")).toBeInTheDocument();
  });

  it("applies custom className", () => {
    const { container } = render(<ActionButton className="w-full">Wide</ActionButton>);
    const btn = container.querySelector("button")!;
    expect(btn.className).toContain("w-full");
  });

  it("sets aria-busy=true when loading", () => {
    const { container } = render(<ActionButton loading>Loading</ActionButton>);
    const btn = container.querySelector("button")!;
    expect(btn).toHaveAttribute("aria-busy", "true");
  });

  it("does not set aria-busy when not loading", () => {
    const { container } = render(<ActionButton>Normal</ActionButton>);
    const btn = container.querySelector("button")!;
    expect(btn).not.toHaveAttribute("aria-busy");
  });
});
