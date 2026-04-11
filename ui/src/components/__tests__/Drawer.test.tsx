import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { Drawer } from "../Drawer";

describe("Drawer", () => {
  it("renders nothing when open=false", () => {
    const { container } = render(
      <Drawer open={false} onClose={() => {}}>Content</Drawer>,
    );
    expect(container.innerHTML).toBe("");
  });

  it("renders children when open=true", () => {
    render(
      <Drawer open onClose={() => {}}>Drawer body</Drawer>,
    );
    expect(screen.getByText("Drawer body")).toBeInTheDocument();
  });

  it("renders title in header", () => {
    render(
      <Drawer open onClose={() => {}} title="My Drawer">Body</Drawer>,
    );
    expect(screen.getByText("My Drawer")).toBeInTheDocument();
  });

  it("does not render header when title is omitted", () => {
    const { container } = render(
      <Drawer open onClose={() => {}}>Body</Drawer>,
    );
    // No h-11 header div
    expect(container.querySelector(".h-11")).not.toBeInTheDocument();
  });

  it("renders footer when provided", () => {
    render(
      <Drawer open onClose={() => {}} footer={<button>Save</button>}>
        Body
      </Drawer>,
    );
    expect(screen.getByText("Save")).toBeInTheDocument();
  });

  it("calls onClose when backdrop is clicked", () => {
    const onClose = vi.fn();
    const { container } = render(
      <Drawer open onClose={onClose} title="Test">Body</Drawer>,
    );
    const backdrop = container.querySelector("[aria-hidden='true']");
    expect(backdrop).toBeInTheDocument();
    fireEvent.click(backdrop!);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose when close button is clicked", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose} title="Test">Body</Drawer>,
    );
    const closeBtn = screen.getByLabelText("Close");
    fireEvent.click(closeBtn);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose on Escape key", () => {
    const onClose = vi.fn();
    render(
      <Drawer open onClose={onClose} title="Test">Body</Drawer>,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not render backdrop when backdrop=false", () => {
    const { container } = render(
      <Drawer open onClose={() => {}} backdrop={false}>Body</Drawer>,
    );
    expect(container.querySelector("[aria-hidden='true']")).not.toBeInTheDocument();
  });

  // Accessibility
  it("has role=dialog and aria-modal=true", () => {
    render(
      <Drawer open onClose={() => {}} title="Dialog">Body</Drawer>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toBeInTheDocument();
    expect(dialog).toHaveAttribute("aria-modal", "true");
  });

  it("uses title as aria-label", () => {
    render(
      <Drawer open onClose={() => {}} title="Settings">Body</Drawer>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("aria-label", "Settings");
  });

  it("uses fallback aria-label when no title", () => {
    render(
      <Drawer open onClose={() => {}}>Body</Drawer>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("aria-label", "Drawer");
  });

  it("close button has aria-label=Close", () => {
    render(
      <Drawer open onClose={() => {}} title="Test">Body</Drawer>,
    );
    expect(screen.getByLabelText("Close")).toBeInTheDocument();
  });

  // Side variants
  it("renders on right side by default", () => {
    render(
      <Drawer open onClose={() => {}}>Body</Drawer>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog.className).toContain("right-0");
  });

  it("renders on left side", () => {
    render(
      <Drawer open onClose={() => {}} side="left">Body</Drawer>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog.className).toContain("left-0");
  });

  it("applies custom width", () => {
    render(
      <Drawer open onClose={() => {}} width="w-96">Body</Drawer>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog.className).toContain("w-96");
  });

  it("applies custom className", () => {
    render(
      <Drawer open onClose={() => {}} className="mt-4">Body</Drawer>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog.className).toContain("mt-4");
  });
});
