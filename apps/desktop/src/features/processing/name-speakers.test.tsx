// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import en from "../../../../../packages/i18n/locales/en.json";
import { ipc } from "../../ipc";
import { NameSpeakers } from "./name-speakers";
import type { UnnamedSpeaker } from "./speakers-adapter";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const speaker = (score: number | null): UnnamedSpeaker => ({
  gid: "s1",
  number: 2,
  colorSlot: 2,
  t0Ms: null,
  t1Ms: null,
  suggestion: { personGid: "me", name: "", isMe: true, score },
});

function renderCard(s: UnnamedSpeaker, onDone = vi.fn()) {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <PlatformProvider value="mac">
        <ToastProvider label="toasts">
          <NameSpeakers meeting="m1" speakers={[s]} onDone={onDone} onSkipAll={vi.fn()} />
        </ToastProvider>
      </PlatformProvider>
    </QueryClientProvider>,
  );
  return onDone;
}

describe("Name your speakers: voice suggestion pill", () => {
  it("shows 'Me? · 87%' and accepting names the speaker and removes the card", async () => {
    const accept = vi.spyOn(ipc.commands, "acceptVoiceSuggestion").mockResolvedValue({ status: "ok", data: null });
    const onDone = renderCard(speaker(0.87));
    const pill = screen.getByRole("button", { name: "Accept Me as this speaker" });
    expect(pill.textContent).toBe("Me? · 87%");
    await act(async () => fireEvent.click(pill));
    expect(accept).toHaveBeenCalledWith("m1", "s1");
    expect(onDone).toHaveBeenCalledOnce();
  });

  it("dismissing hides the pill and leaves the card", async () => {
    const dismiss = vi.spyOn(ipc.commands, "dismissVoiceSuggestion").mockResolvedValue({ status: "ok", data: null });
    const onDone = renderCard(speaker(0.87));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Dismiss the suggestion for Speaker 2" })));
    expect(dismiss).toHaveBeenCalledWith("m1", "s1");
    expect(screen.queryByTestId("voice-suggestion")).toBeNull();
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox")).toBeTruthy();
  });

  it("a second click while the request runs does nothing", async () => {
    let release!: () => void;
    const accept = vi.spyOn(ipc.commands, "acceptVoiceSuggestion").mockImplementation(() => new Promise((r) => (release = () => r({ status: "ok", data: null }))));
    const onDone = renderCard(speaker(0.87));
    const pill = screen.getByRole("button", { name: "Accept Me as this speaker" });
    await act(async () => {
      fireEvent.click(pill);
      fireEvent.click(pill);
    });
    expect(accept).toHaveBeenCalledOnce();
    expect((pill as HTMLButtonElement).disabled).toBe(true);
    await act(async () => release());
    expect(onDone).toHaveBeenCalledOnce();
  });

  it("a rename refused with a code says a sentence, not the code", async () => {
    vi.spyOn(ipc.commands, "renameMeetingSpeaker").mockResolvedValue({ status: "error", error: "liveMeeting" });
    const onDone = renderCard({ ...speaker(null), suggestion: null });
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "Linh" } });
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.queryByText(/liveMeeting/)).toBeNull();
    expect(screen.getByText(en.people.errors.liveMeeting)).toBeTruthy();
  });

  it("a null score shows just 'Me?'", () => {
    renderCard(speaker(null));
    expect(screen.getByRole("button", { name: "Accept Me as this speaker" }).textContent).toBe("Me?");
  });

  it("a failed accept keeps the card", async () => {
    vi.spyOn(ipc.commands, "acceptVoiceSuggestion").mockResolvedValue({ status: "error", error: "thirdPartyOff" });
    const onDone = renderCard(speaker(0.8));
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Accept Me as this speaker" })));
    expect(onDone).not.toHaveBeenCalled();
  });
});
