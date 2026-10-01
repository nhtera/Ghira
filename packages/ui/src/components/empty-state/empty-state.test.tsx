// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "../../platform/platform";
import { EmptyState } from "./empty-state";

afterEach(cleanup);

describe("EmptyState", () => {
  it("library: title, platform body, record and import actions", async () => {
    const onPrimary = vi.fn();
    const onSecondary = vi.fn();
    const { rerender } = render(<EmptyState kind="library" onPrimary={onPrimary} onSecondary={onSecondary} />);
    expect(screen.getByRole("heading", { name: "Record your first meeting" })).toBeTruthy();
    expect(screen.getByText(/on this Mac/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Record call" }));
    await userEvent.click(screen.getByRole("button", { name: "Import a file" }));
    expect(onPrimary).toHaveBeenCalledOnce();
    expect(onSecondary).toHaveBeenCalledOnce();
    rerender(
      <PlatformProvider value="win">
        <EmptyState kind="library" />
      </PlatformProvider>,
    );
    expect(screen.getByText(/on this PC/)).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("search: names the query as a text node", () => {
    render(<EmptyState kind="search" query="<b>chot</b>" />);
    expect(screen.getByRole("heading").textContent).toBe("No meetings match “<b>chot</b>”");
    expect(screen.getByRole("heading").querySelector("b")).toBeNull();
  });

  it.each([
    ["people", "People appear after your first meeting"],
    ["ask", "Ask works once you have a few meetings"],
    ["import", "Nothing in the queue"],
  ] as const)("%s has icon, title and body", (kind, title) => {
    const { container } = render(<EmptyState kind={kind} />);
    expect(screen.getByRole("heading", { name: title })).toBeTruthy();
    expect(container.querySelector("svg[data-icon]")).toBeTruthy();
    expect(container.querySelector("p")?.textContent).toBeTruthy();
  });

  it("import offers Choose files only when given a handler", async () => {
    const onPrimary = vi.fn();
    render(<EmptyState kind="import" onPrimary={onPrimary} />);
    await userEvent.click(screen.getByRole("button", { name: "Choose files…" }));
    expect(onPrimary).toHaveBeenCalledOnce();
  });
});
