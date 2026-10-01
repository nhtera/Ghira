// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SpeakerChip } from "./speaker-chip";

afterEach(cleanup);

describe("SpeakerChip", () => {
  it("identifying shows the copy and a dashed unknown avatar", () => {
    const { container } = render(<SpeakerChip state="identifying" onClick={() => {}} />);
    expect(screen.getByRole("button", { name: "Identifying speaker…" })).toBeTruthy();
    expect(container.querySelector('[data-kind="unknown"]')).toBeTruthy();
  });

  it("numbered speakers use their number as the initial", () => {
    const { container } = render(<SpeakerChip state="numbered" name="Speaker 2" colorSlot={2} />);
    expect(container.querySelector('[data-kind="person"]')?.textContent).toBe("2");
  });

  it("named speakers use the initial of the name", () => {
    const { container } = render(<SpeakerChip state="named" name="Sarah" colorSlot={8} />);
    expect(container.querySelector('[data-kind="person"]')?.textContent).toBe("S");
  });

  it("suggested has a separate accept button", async () => {
    const onAccept = vi.fn();
    const onClick = vi.fn();
    render(<SpeakerChip state="suggested" name="Speaker 2" suggestion="Linh" onAcceptSuggestion={onAccept} onClick={onClick} />);
    await userEvent.click(screen.getByRole("button", { name: "Accept Linh as this speaker" }));
    expect(onAccept).toHaveBeenCalledOnce();
    expect(onClick).not.toHaveBeenCalled();
  });

  it("auto-named shows the voice-match mark", () => {
    render(<SpeakerChip state="auto" name="Minh" colorSlot={4} />);
    expect(screen.getByRole("img", { name: "voice match" })).toBeTruthy();
  });

  it("merged shows from → into", () => {
    render(<SpeakerChip state="merged" name="Linh" mergedFrom="Speaker 5" onClick={() => {}} />);
    expect(screen.getByRole("button").textContent).toBe("LSpeaker 5 → Linh");
  });

  it("without onClick it is plain text, not a button", () => {
    render(<SpeakerChip state="named" name="Linh" />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByText("Linh")).toBeTruthy();
  });

  it("selected sets aria-pressed and clicking calls onClick", async () => {
    const onClick = vi.fn();
    const { rerender } = render(<SpeakerChip state="named" name="Linh" onClick={onClick} />);
    expect(screen.getByRole("button", { name: "Linh" }).getAttribute("aria-pressed")).toBe("false");
    await userEvent.click(screen.getByRole("button", { name: "Linh" }));
    expect(onClick).toHaveBeenCalledOnce();
    rerender(<SpeakerChip state="named" name="Linh" selected onClick={onClick} />);
    expect(screen.getByRole("button", { name: "Linh" }).getAttribute("aria-pressed")).toBe("true");
  });
});
