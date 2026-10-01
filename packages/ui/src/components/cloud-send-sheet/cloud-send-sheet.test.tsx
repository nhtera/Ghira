// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "../../platform/platform";
import { CloudSendSheet, type CloudSendSheetProps } from "./cloud-send-sheet";

afterEach(cleanup);

const RAW = "Linh: gửi qua linh.tran@studio.vn";
const RED = "[PERSON 2]: gửi qua [EMAIL]";

const props = (over: Partial<CloudSendSheetProps> = {}): CloudSendSheetProps => ({
  open: true,
  onOpenChange: () => {},
  state: "default",
  providers: [
    { id: "a", name: "Claude" },
    { id: "o", name: "OpenAI" },
  ],
  providerId: "a",
  onProviderChange: () => {},
  words: 9840,
  tokens: 13100,
  redact: true,
  onRedactChange: () => {},
  rawText: RAW,
  redactedText: RED,
  onSend: () => {},
  onKeepLocal: () => {},
  ...over,
});

describe("CloudSendSheet", () => {
  it("states what leaves the device: text only, never audio", () => {
    render(<CloudSendSheet {...props()} />);
    expect(screen.getByRole("dialog", { name: "Improve these notes with a cloud model?" })).toBeTruthy();
    expect(screen.getByText(/Transcript text only · 9,840 words · about 13,100 tokens/)).toBeTruthy();
    expect(screen.getByText("Audio never leaves this device")).toBeTruthy();
  });

  it("renders the before/after preview as text; redaction on sends the redacted text", () => {
    render(<CloudSendSheet {...props()} />);
    expect(screen.getByText(RAW)).toBeTruthy();
    expect(screen.getByText(RED)).toBeTruthy();
    expect(screen.queryByText(/will be sent as written/)).toBeNull();
  });

  it("redaction off previews the raw text and warns", () => {
    render(<CloudSendSheet {...props({ redact: false })} />);
    expect(screen.getAllByText(RAW)).toHaveLength(2);
    expect(screen.queryByText(RED)).toBeNull();
    expect(screen.getByText(/will be sent as written/)).toBeTruthy();
  });

  it("toggles redaction and provider, sends or keeps local", async () => {
    const user = userEvent.setup();
    const onRedactChange = vi.fn();
    const onProviderChange = vi.fn();
    const onSend = vi.fn();
    const onKeepLocal = vi.fn();
    render(<CloudSendSheet {...props({ onRedactChange, onProviderChange, onSend, onKeepLocal })} />);
    await user.click(screen.getByRole("switch", { name: "Redact names, emails and phone numbers" }));
    expect(onRedactChange).toHaveBeenCalledWith(false);
    await user.click(screen.getByRole("radio", { name: "OpenAI" }));
    expect(onProviderChange).toHaveBeenCalledWith("o");
    await user.click(screen.getByRole("button", { name: "Send and improve" }));
    expect(onSend).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "Keep local" }));
    expect(onKeepLocal).toHaveBeenCalledOnce();
  });

  it("sending disables actions, announces status and ignores Escape", async () => {
    const onOpenChange = vi.fn();
    render(<CloudSendSheet {...props({ state: "sending", onOpenChange })} />);
    expect((screen.getByRole("button", { name: "Sending…" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Keep local" }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(screen.getByRole("dialog")).getByRole("status").textContent).toContain("Sending transcript…");
    await userEvent.keyboard("{Escape}");
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("sent confirms and offers Done", async () => {
    const onOpenChange = vi.fn();
    render(<CloudSendSheet {...props({ state: "sent", onOpenChange })} />);
    expect(screen.getByRole("status").textContent).toContain("logged in Settings");
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("failed alerts that local notes are kept and can retry", async () => {
    const onRetry = vi.fn();
    render(<CloudSendSheet {...props({ state: "failed", onRetry })} />);
    expect(screen.getByRole("alert").textContent).toBe("Couldn’t reach Claude. Local notes are kept.");
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("is a sheet on mac and a centered dialog on Windows", () => {
    const { unmount } = render(<CloudSendSheet {...props()} />);
    expect(screen.getByRole("dialog").getAttribute("data-shape")).toBe("sheet");
    unmount();
    render(
      <PlatformProvider value="win">
        <CloudSendSheet {...props()} />
      </PlatformProvider>,
    );
    expect(screen.getByRole("dialog").getAttribute("data-shape")).toBe("dialog");
    expect(screen.getByText("What leaves this PC")).toBeTruthy();
  });
});
