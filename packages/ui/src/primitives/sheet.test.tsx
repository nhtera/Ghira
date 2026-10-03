// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Sheet, type SheetProps } from "./sheet";

afterEach(cleanup);

function setup(props: Partial<SheetProps> = {}) {
  const onOpenChange = vi.fn();
  const onDetentChange = vi.fn();
  render(
    <>
      <button type="button">Outside</button>
      <Sheet open onOpenChange={onOpenChange} onDetentChange={onDetentChange} title="Paused" closeLabel="Close" handleLabel="Resize sheet" {...props}>
        <button type="button">First</button>
        <button type="button">Second</button>
      </Sheet>
    </>,
  );
  return { onOpenChange, onDetentChange };
}

describe("Sheet", () => {
  it("is a dialog with a title, a close button and a drag handle", () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: "Paused" });
    expect(dialog.getAttribute("data-detent")).toBe("medium");
    expect(screen.getByRole("button", { name: "Close" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Resize sheet" }).getAttribute("aria-expanded")).toBe("false");
  });

  it("moves focus into the sheet and keeps Tab inside it", async () => {
    setup();
    const dialog = screen.getByRole("dialog");
    expect(dialog.contains(document.activeElement)).toBe(true);
    for (let i = 0; i < 8; i++) {
      await userEvent.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    await userEvent.tab({ shift: true });
    expect(dialog.contains(document.activeElement)).toBe(true);
  });

  it("Escape closes it", async () => {
    const { onOpenChange } = setup();
    await userEvent.keyboard("{Escape}");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("dismissible=false ignores Escape and has no close button", async () => {
    const { onOpenChange } = setup({ dismissible: false });
    await userEvent.keyboard("{Escape}");
    expect(onOpenChange).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
  });

  it("the handle toggles the detent", async () => {
    const { onDetentChange } = setup();
    const handle = screen.getByRole("button", { name: "Resize sheet" });
    await userEvent.click(handle);
    expect(onDetentChange).toHaveBeenLastCalledWith("large");
    expect(screen.getByRole("dialog").getAttribute("data-detent")).toBe("large");
    expect(handle.getAttribute("aria-expanded")).toBe("true");
    await userEvent.click(handle);
    expect(onDetentChange).toHaveBeenLastCalledWith("medium");
  });

  it("swiping the handle up expands, down shrinks, then dismisses", () => {
    const { onOpenChange, onDetentChange } = setup();
    const handle = screen.getByRole("button", { name: "Resize sheet" });
    const swipe = (from: number, to: number) => {
      fireEvent.pointerDown(handle, { clientY: from });
      fireEvent.pointerUp(handle, { clientY: to });
      fireEvent.click(handle);
    };
    swipe(300, 200);
    expect(onDetentChange).toHaveBeenLastCalledWith("large");
    swipe(200, 300);
    expect(onDetentChange).toHaveBeenLastCalledWith("medium");
    expect(onOpenChange).not.toHaveBeenCalled();
    swipe(200, 300);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("a cancelled gesture does not leave a swipe pending", () => {
    const { onOpenChange, onDetentChange } = setup();
    const handle = screen.getByRole("button", { name: "Resize sheet" });
    fireEvent.pointerDown(handle, { clientY: 300 });
    fireEvent.pointerCancel(handle);
    fireEvent.pointerUp(handle, { clientY: 100 });
    expect(onDetentChange).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("title and body scroll together; the footer is outside the scroll region", () => {
    setup({ footer: <button type="button">Save</button> });
    const dialog = screen.getByRole("dialog");
    const scroller = screen.getByText("Paused").parentElement!;
    expect(scroller.contains(screen.getByText("First"))).toBe(true);
    expect(scroller.contains(screen.getByRole("button", { name: "Save" }))).toBe(false);
    expect(dialog.querySelector("[data-sheet-footer]")?.contains(screen.getByRole("button", { name: "Save" }))).toBe(true);
  });

  it("the scroll region is not a keyboard stop while it does not scroll", () => {
    setup();
    expect(screen.getByText("Paused").parentElement!.getAttribute("tabindex")).toBeNull();
  });

  it("opens at the requested detent", () => {
    setup({ detent: "large" });
    expect(screen.getByRole("dialog").getAttribute("data-detent")).toBe("large");
  });
});
