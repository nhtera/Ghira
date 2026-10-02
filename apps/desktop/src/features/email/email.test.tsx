// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import { ipc } from "../../ipc";
import { emailText } from "./email-text";
import { FollowupEmailDialog } from "./followup-email-dialog";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

const view = () =>
  render(
    <PlatformProvider value="mac">
      <ToastProvider label="toasts">
        <FollowupEmailDialog open onOpenChange={() => {}} meeting="m1" />
      </ToastProvider>
    </PlatformProvider>,
  );
const ok = { status: "ok", data: { subject: "Sync notes", body: "Hi all,\n\nThanks." } } as const;

describe("emailText", () => {
  it("is the subject, a blank line, then the body", () => expect(emailText(" S ", "B\n")).toBe("S\n\nB"));
});

describe("FollowupEmailDialog", () => {
  it("writes with the chosen language and tone, then shows editable subject and body", async () => {
    const draft = vi.spyOn(ipc.commands, "draftFollowupEmail").mockResolvedValue(ok);
    view();
    fireEvent.click(screen.getByRole("radio", { name: "Formal" }));
    fireEvent.click(screen.getByRole("radio", { name: "Tiếng Việt" }));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Write draft" })));
    expect(draft).toHaveBeenCalledWith("m1", "vi", "formal");
    const subject = screen.getByRole("textbox", { name: "Subject" }) as HTMLInputElement;
    expect(subject.value).toBe("Sync notes");
    fireEvent.change(subject, { target: { value: "Edited" } });
    expect(screen.getByRole("button", { name: "Rewrite" })).toBeTruthy();
    expect(screen.getByText(/Nothing is sent from/)).toBeTruthy();
  });

  it("counts seconds while writing", async () => {
    vi.useFakeTimers();
    vi.spyOn(ipc.commands, "draftFollowupEmail").mockReturnValue(new Promise(() => {}));
    view();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Write draft" })));
    expect(screen.getByRole("status").textContent).toBe("Writing… 0 s");
    await act(() => vi.advanceTimersByTimeAsync(3000));
    expect(screen.getByRole("status").textContent).toBe("Writing… 3 s");
  });

  it("copies subject + blank line + body (edits included) and says Copied", async () => {
    vi.spyOn(ipc.commands, "draftFollowupEmail").mockResolvedValue(ok);
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    view();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Write draft" })));
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), { target: { value: "New body" } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Copy email" })));
    expect(write).toHaveBeenCalledWith("Sync notes\n\nNew body");
    expect(await screen.findByText("Email copied")).toBeTruthy();
  });

  it("shows the error (no notes yet, model missing) and keeps the form", async () => {
    vi.spyOn(ipc.commands, "draftFollowupEmail").mockResolvedValue({ status: "error", error: "this meeting has no notes yet" });
    view();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Write draft" })));
    expect(screen.getByRole("alert").textContent).toMatch(/no notes yet/);
    expect(screen.getByRole("button", { name: "Write draft" })).toBeTruthy();
    expect(screen.queryByRole("textbox", { name: "Subject" })).toBeNull();
  });

  it("a failed Rewrite keeps the earlier draft", async () => {
    const spy = vi.spyOn(ipc.commands, "draftFollowupEmail").mockResolvedValueOnce(ok).mockResolvedValueOnce({ status: "error", error: "busy" });
    view();
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Write draft" })));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Rewrite" })));
    expect(spy).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("alert").textContent).toMatch(/busy/);
    expect((screen.getByRole("textbox", { name: "Subject" }) as HTMLInputElement).value).toBe("Sync notes");
  });
});
