// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ActionItem } from "./action-item";

afterEach(cleanup);

const owner = { name: "Minh", colorSlot: 4 };

describe("ActionItem", () => {
  it("is a checkbox named by the task text and toggles", async () => {
    const onToggle = vi.fn();
    render(<ActionItem text="Build file import" owner={owner} onToggle={onToggle} />);
    const box = screen.getByRole("checkbox", { name: "Build file import" });
    expect(box.getAttribute("aria-checked")).toBe("false");
    await userEvent.click(box);
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it("done is checked and struck through", () => {
    const { container } = render(<ActionItem text="Update roadmap" owner={owner} done />);
    expect(screen.getByRole("checkbox").getAttribute("aria-checked")).toBe("true");
    expect(container.querySelector("[data-done]")).toBeTruthy();
    expect(screen.getByText("Update roadmap").className).toContain("line-through");
  });

  it("shows the owner, or Unassigned without one", () => {
    const { rerender } = render(<ActionItem text="x" owner={owner} />);
    expect(screen.getByText("Minh")).toBeTruthy();
    rerender(<ActionItem text="x" owner={null} />);
    expect(screen.getByText("Unassigned")).toBeTruthy();
  });

  it("due soon and overdue carry an icon as well as text", () => {
    const { container, rerender } = render(<ActionItem text="x" owner={owner} due={{ text: "Due tomorrow", tone: "soon" }} />);
    expect(screen.getByText("Due tomorrow").getAttribute("data-tone")).toBe("soon");
    expect(container.querySelector('svg[data-icon="schedule"]')).toBeTruthy();
    rerender(<ActionItem text="x" owner={owner} due={{ text: "Overdue · 2 days", tone: "overdue" }} />);
    expect(screen.getByText("Overdue · 2 days").getAttribute("data-tone")).toBe("overdue");
    expect(container.querySelector('svg[data-icon="error"]')).toBeTruthy();
  });

  it("citation chips report which one was activated", async () => {
    const onCite = vi.fn();
    render(<ActionItem text="x" owner={owner} citations={[{ timeMs: 65_000 }]} onCite={onCite} />);
    await userEvent.click(screen.getByRole("button", { name: "Show in transcript 1:05" }));
    expect(onCite).toHaveBeenCalledWith(0, { timeMs: 65_000 });
  });
});
