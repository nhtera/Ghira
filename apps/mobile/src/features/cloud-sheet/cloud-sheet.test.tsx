// SPDX-License-Identifier: Apache-2.0
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { PlatformProvider } from "@ghi/ui";
import { CloudSheet } from "./cloud-sheet";

const calls = (name: string) => window.__ghiSettingsMock!.calls[name] ?? 0;

function renderSheet(
  props: Partial<React.ComponentProps<typeof CloudSheet>> = {},
) {
  const onOpenChange = vi.fn();
  render(
    <I18nextProvider i18n={initMobileI18n("en")}>
      <PlatformProvider value="ios">
        <CloudSheet
          meetingId="m-1"
          open
          onOpenChange={onOpenChange}
          {...props}
        />
      </PlatformProvider>
    </I18nextProvider>,
  );
  return { onOpenChange };
}

describe("CloudSheet", () => {
  beforeEach(async () => {
    await import("../../ipc");
    window.__ghiSettingsMock!.reset();
    window.__ghiSettingsMock!.keys.anthropic = true;
    window.__ghiSettingsMock!.offerCloud(true);
  });
  afterEach(cleanup);

  it("previews without sending, and sends once on the click", async () => {
    renderSheet();
    await screen.findByText("Exactly what will be sent");
    expect(calls("cloudPreview")).toBeGreaterThan(0);
    expect(calls("cloudSend")).toBe(0);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByText("Notes were rewritten with the cloud.");
    expect(calls("cloudSend")).toBe(1);
  });

  it("says General is sent when the meeting's template is one of the user's own, and only then", async () => {
    window.__ghiSettingsMock!.templateFallback = true;
    renderSheet();
    await screen.findByText("Exactly what will be sent");
    expect((await screen.findByTestId("template-fallback")).textContent).toMatch(/built-in General template is sent\. Your instructions never leave your computer/);
    cleanup();
    window.__ghiSettingsMock!.templateFallback = false;
    renderSheet();
    await screen.findByText("Exactly what will be sent");
    expect(screen.queryByTestId("template-fallback")).toBeNull();
  });

  it("cancel closes and never sends", async () => {
    const { onOpenChange } = renderSheet();
    await screen.findByText("Exactly what will be sent");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(calls("cloudSend")).toBe(0);
  });

  it("is blocked for a meeting with cloud AI off", async () => {
    window.__ghiSettingsMock!.cloudLocked.push("m-1");
    renderSheet();
    await screen.findByText(/Cloud is off for this meeting/);
    expect(
      (screen.getByRole("button", { name: "Send" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(calls("cloudSend")).toBe(0);
  });

  it("keeps the notes local when the send fails", async () => {
    window.__ghiSettingsMock!.failSend.push("m-1");
    renderSheet();
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Send" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByText(/your notes stay on this phone/);
  });
});
