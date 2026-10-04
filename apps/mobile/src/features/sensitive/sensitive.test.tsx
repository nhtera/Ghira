// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, describe, expect, it, vi } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { PlatformProvider } from "@ghi/ui";
import type { DiscardPreview } from "../../bindings";
import { DiscardSheet } from "../record/discard-sheet";
import { MoreSheet } from "../record/more-sheet";
import { SensitiveBadge, SensitiveRow, SensitiveSheet } from ".";

const wrap = (ui: React.ReactNode) =>
  render(
    <I18nextProvider i18n={initMobileI18n("en")}>
      <PlatformProvider value="ios">{ui}</PlatformProvider>
    </I18nextProvider>,
  );

afterEach(cleanup);

describe("sensitive mode", () => {
  it("the badge says it in words", () => {
    wrap(<SensitiveBadge />);
    expect(screen.getByTestId("sensitive-badge").textContent).toBe("Sensitive · no audio kept");
  });

  it("the switch carries its promise and can be given a reason to stay off", () => {
    const onChange = vi.fn();
    const { rerender } = wrap(<SensitiveRow checked={false} onChange={onChange} />);
    expect(screen.getByText(/no audio is saved, nothing goes to the cloud/)).toBeTruthy();
    fireEvent.click(screen.getByRole("switch", { name: "Sensitive meeting" }));
    expect(onChange).toHaveBeenCalledWith(true);
    rerender(
      <I18nextProvider i18n={initMobileI18n("en")}>
        <PlatformProvider value="ios">
          <SensitiveRow checked={false} onChange={onChange} disabled hint="Needs the live transcript." />
        </PlatformProvider>
      </I18nextProvider>,
    );
    expect(screen.getByRole("switch", { name: "Sensitive meeting" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("Needs the live transcript.")).toBeTruthy();
  });

  it("asks before it deletes audio, and says what a recording can't undo", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    const { rerender } = wrap(<SensitiveSheet open recording onCancel={onCancel} onConfirm={onConfirm} />);
    expect(screen.getByText(/It can’t be turned off during this recording/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
    rerender(
      <I18nextProvider i18n={initMobileI18n("en")}>
        <PlatformProvider value="ios">
          <SensitiveSheet open recording={false} onCancel={onCancel} onConfirm={onConfirm} />
        </PlatformProvider>
      </I18nextProvider>,
    );
    expect(screen.getByText(/Its audio is deleted now/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Make sensitive" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("More offers sensitive mode while there is a transcript, and a sensitive recording can't leave it", () => {
    const onSensitive = vi.fn();
    const onDiscard = vi.fn();
    const base = { open: true, onClose: () => {}, onSensitive, onDiscard };
    const { rerender } = wrap(<MoreSheet {...base} sensitive={false} canSensitive />);
    fireEvent.click(screen.getByRole("button", { name: "Make sensitive…" }));
    expect(onSensitive).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Discard the last 5 minutes…" }));
    expect(onDiscard).toHaveBeenCalledWith(300);
    const again = (props: Partial<React.ComponentProps<typeof MoreSheet>>) =>
      rerender(
        <I18nextProvider i18n={initMobileI18n("en")}>
          <PlatformProvider value="ios">
            <MoreSheet {...base} sensitive={false} canSensitive {...props} />
          </PlatformProvider>
        </I18nextProvider>,
      );
    again({ canSensitive: false });
    expect(screen.getByRole("button", { name: "Make sensitive…" }).hasAttribute("disabled")).toBe(true);
    again({ sensitive: true });
    const on = screen.getByRole("button", { name: /Sensitive meeting · On until the recording ends/ });
    expect(on.hasAttribute("disabled")).toBe(true);
  });
});

describe("DiscardSheet", () => {
  const preview: DiscardPreview = { fromMs: 1000, lines: ["first line", "second line"], notes: ["a note"], marks: 2 };
  const ok = (data: DiscardPreview) => Promise.resolve({ status: "ok" as const, data });

  it("lists what goes before confirming, then discards from the previewed cut", async () => {
    const onConfirm = vi.fn(() => Promise.resolve(true));
    const onClose = vi.fn();
    wrap(<DiscardSheet seconds={300} sensitive={false} preview={() => ok(preview)} onConfirm={onConfirm} onClose={onClose} />);
    expect(await screen.findByText("This removes the audio and 2 transcript lines, 1 note, 2 marks.")).toBeTruthy();
    expect(screen.getByText("Discard the last 5 minutes?")).toBeTruthy();
    expect(onConfirm).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(onConfirm).toHaveBeenCalledWith(1000));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("names only the text when there is no audio, and Cancel changes nothing", async () => {
    const onConfirm = vi.fn(() => Promise.resolve(true));
    const onClose = vi.fn();
    wrap(<DiscardSheet seconds={60} sensitive preview={() => ok({ ...preview, notes: [], marks: 0 })} onConfirm={onConfirm} onClose={onClose} />);
    expect(await screen.findByText("This removes 2 transcript lines.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("closes when the preview can't be read", async () => {
    const onClose = vi.fn();
    wrap(<DiscardSheet seconds={60} sensitive={false} preview={() => Promise.resolve({ status: "error" as const, error: "not recording" })} onConfirm={vi.fn()} onClose={onClose} />);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});
