// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Button } from "../../primitives/button";
import { ConfirmArea, InlineConfirm } from "./inline-confirm";

afterEach(cleanup);

describe("InlineConfirm", () => {
  it("is an alert dialog named by the consequence, with Cancel focused", () => {
    render(<InlineConfirm question="Delete 142 meetings?" confirmLabel="Delete everything" onConfirm={() => {}} onCancel={() => {}} />);
    expect(screen.getByRole("alertdialog", { name: "Delete 142 meetings?" })).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
  });

  it("confirm, cancel and Escape call back", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<InlineConfirm question="Q?" confirmLabel="Delete" onConfirm={onConfirm} onCancel={onCancel} />);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onConfirm).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(2);
  });
});

describe("ConfirmArea", () => {
  const setup = (onConfirm = () => {}) =>
    render(
      <ConfirmArea
        question="Delete voice data?"
        confirmLabel="Delete voice data"
        onConfirm={onConfirm}
        trigger={(p) => <Button {...p}>Delete…</Button>}
      />,
    );

  it("replaces the trigger and moves focus in", async () => {
    setup();
    await userEvent.click(screen.getByRole("button", { name: "Delete…" }));
    expect(screen.queryByRole("button", { name: "Delete…" })).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
  });

  it("returns focus to the trigger on Escape and on Cancel", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("button", { name: "Delete…" }));
    await user.keyboard("{Escape}");
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Delete…" }));
    await user.click(screen.getByRole("button", { name: "Delete…" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Delete…" }));
  });

  it("confirm runs the action and restores the trigger", async () => {
    const onConfirm = vi.fn();
    setup(onConfirm);
    await userEvent.click(screen.getByRole("button", { name: "Delete…" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete voice data" }));
    expect(onConfirm).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Delete…" })).toBeTruthy();
  });
});
