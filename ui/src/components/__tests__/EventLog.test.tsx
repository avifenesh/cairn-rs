/**
 * EventLog tests — F33 regression.
 *
 * Primary concern: the component must NOT scroll the document
 * (window) when it mounts or when new rows arrive. Previously the
 * component used `element.scrollIntoView({ behavior: 'smooth' })`
 * on a sentinel inside the scroll container, which walked ancestors
 * and jumped the whole page to the component's location — causing
 * the dashboard to snap to the bottom of the viewport on every load.
 *
 * The fix moves scrolling to `element.scrollTop = scrollHeight` on
 * the inner container ref only, so the window is never touched.
 */

import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// useEventStream is imported by EventLog — stub it out so tests don't
// try to open an EventSource.
vi.mock("../../hooks/useEventStream", () => ({
  useEventStream: () => ({ events: [], status: "connected" }),
}));

import { EventLog } from "../EventLog";
import type { RecentEvent } from "../../lib/types";

const origScrollIntoView = HTMLElement.prototype.scrollIntoView;

beforeEach(() => {
  // Any call to scrollIntoView anywhere in the component tree fails
  // the test. This is how we prove the dashboard jump can't recur.
  HTMLElement.prototype.scrollIntoView = vi.fn(() => {
    throw new Error(
      "scrollIntoView was called — EventLog must not scroll the document",
    );
  });
});

afterEach(() => {
  HTMLElement.prototype.scrollIntoView = origScrollIntoView;
});

function makeEvent(i: number): RecentEvent {
  return {
    event_type: "run.state_changed",
    stored_at: Date.now() - i * 1000,
    data: { run_id: `run-${i}`, message: `event ${i}` },
  };
}

describe("EventLog — F33 (no document scroll)", () => {
  it("does not call scrollIntoView when mounting with seed events", () => {
    const events = Array.from({ length: 10 }, (_, i) => makeEvent(i));
    render(<EventLog initialEvents={events} />);
    expect(screen.getByRole("log")).toBeInTheDocument();
  });

  it("does not call scrollIntoView when mounting empty", () => {
    render(<EventLog initialEvents={[]} />);
    expect(screen.getByRole("log")).toBeInTheDocument();
  });

  it("mounts with a dedicated scroll container (scrollRef)", () => {
    render(<EventLog initialEvents={[makeEvent(0)]} />);
    const container = screen.getByRole("log");
    // The inner container must be the `overflow-y-auto` element. If
    // someone reintroduces a `scrollIntoView` on a sentinel child, the
    // `scrollIntoView` stub above will fire and fail the test.
    expect(container.className).toContain("overflow-y-auto");
  });
});
