// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

const commands = vi.hoisted(() => ({ transcriptionEngine: vi.fn(), setTranscriptionEngine: vi.fn() }));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderLive } from "../live/test-utils";
import { EngineCard } from "./engine-card";

const state = (engine: "nemo" | "whisper", whisperAvailable = true, whisperInstalled = false) => ({
  status: "ok" as const,
  data: { engine, whisperAvailable, whisperInstalled, whisperBytes: 574_926_293 },
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("EngineCard", () => {
  it("chooses Whisper, lists its download and says what happens meanwhile", async () => {
    commands.transcriptionEngine.mockResolvedValue(state("nemo"));
    commands.setTranscriptionEngine.mockResolvedValue(state("whisper"));
    const onChanged = vi.fn();
    renderLive(<EngineCard onChanged={onChanged} />);
    const standard = await screen.findByRole("radio", { name: /^Standard/ });
    expect(standard.getAttribute("aria-checked")).toBe("true");
    const whisper = screen.getByRole("radio", { name: /High accuracy \(Whisper\)/ });
    expect(whisper.textContent).toMatch(/575 MB download/);
    await userEvent.click(whisper);
    expect(commands.setTranscriptionEngine).toHaveBeenCalledWith("whisper");
    await waitFor(() => expect(whisper.getAttribute("aria-checked")).toBe("true"));
    expect(onChanged).toHaveBeenCalled();
    expect(screen.getByText("Until Whisper is downloaded, transcripts use Standard.")).toBeTruthy();
    // Choosing the current one again does nothing.
    await userEvent.click(whisper);
    expect(commands.setTranscriptionEngine).toHaveBeenCalledTimes(1);
  });

  it("is hidden in a build without Whisper", async () => {
    commands.transcriptionEngine.mockResolvedValue(state("nemo", false));
    const { container } = renderLive(<EngineCard onChanged={() => {}} />);
    await waitFor(() => expect(commands.transcriptionEngine).toHaveBeenCalled());
    expect(container.textContent).toBe("");
  });
});
