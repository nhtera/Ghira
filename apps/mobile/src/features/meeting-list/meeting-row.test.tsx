// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, describe, expect, it, vi } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { PlatformProvider } from "@ghi/ui";
import type { MeetingRow } from "../../bindings";
import { MeetingRowView } from "./meeting-row";

const row = {
  gid: "m1",
  title: "Weekly 1:1",
  startedAt: 1_700_000_000_000,
  durationMs: 18 * 60_000,
  people: [{ name: "Linh", colorSlot: 1 }],
  summary: null,
} as MeetingRow;

function show(
  handlers: Partial<{
    onOpen: () => void;
    onRetry: () => void;
    onDelete: () => void;
  }> = {},
) {
  const props = {
    onOpen: vi.fn(),
    onRetry: vi.fn(),
    onDelete: vi.fn(),
    ...handlers,
  };
  render(
    <I18nextProvider i18n={initMobileI18n("en")}>
      <PlatformProvider value="ios">
        <MeetingRowView row={row} chip={{ kind: "failed" }} {...props} />
      </PlatformProvider>
    </I18nextProvider>,
  );
  return props;
}

const touch = (x: number, y = 0) => ({ touches: [{ clientX: x, clientY: y }] });

describe("MeetingRowView", () => {
  afterEach(cleanup);

  it("opens on tap and retries from the failed chip", () => {
    const p = show();
    fireEvent.click(screen.getByRole("button", { name: /^Weekly 1:1/ }));
    expect(p.onOpen).toHaveBeenCalledOnce();
    fireEvent.click(
      screen.getByRole("button", { name: "Failed · Tap to retry" }),
    );
    expect(p.onRetry).toHaveBeenCalledOnce();
  });

  it("swipes left to reveal Delete and asks before deleting", () => {
    const p = show();
    const surface = screen.getByRole("button", { name: /^Weekly 1:1/ })
      .parentElement as HTMLElement;
    fireEvent.touchStart(surface, touch(300));
    fireEvent.touchMove(surface, touch(200));
    fireEvent.touchEnd(surface);
    expect(surface.style.transform).toBe("translateX(-96px)");
    fireEvent.click(screen.getByRole("button", { name: /^Delete/ }));
    expect(screen.getByRole("alertdialog").textContent).toContain(
      "Delete “Weekly 1:1”",
    );
    expect(p.onDelete).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(p.onDelete).toHaveBeenCalledOnce();
  });

  it("a short or vertical drag stays closed", () => {
    show();
    const surface = screen.getByRole("button", { name: /^Weekly 1:1/ })
      .parentElement as HTMLElement;
    fireEvent.touchStart(surface, touch(300, 0));
    fireEvent.touchMove(surface, touch(290, 80));
    fireEvent.touchEnd(surface);
    expect(surface.style.transform).toBe("translateX(0px)");
  });
});
