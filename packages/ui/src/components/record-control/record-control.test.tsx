// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RecordControl } from "./record-control";

afterEach(cleanup);

describe("RecordControl", () => {
  it("idle: the main part starts the chosen mode", async () => {
    const onStart = vi.fn();
    const { rerender } = render(<RecordControl state="idle" mode="call" onStart={onStart} />);
    await userEvent.click(screen.getByRole("button", { name: "Record call" }));
    expect(onStart).toHaveBeenLastCalledWith("call");
    rerender(<RecordControl state="idle" mode="room" onStart={onStart} />);
    await userEvent.click(screen.getByRole("button", { name: "Record room" }));
    expect(onStart).toHaveBeenLastCalledWith("room");
  });

  it("idle: the caret menu changes the mode without starting", async () => {
    const onStart = vi.fn();
    const onModeChange = vi.fn();
    render(<RecordControl state="idle" mode="call" onStart={onStart} onModeChange={onModeChange} />);
    await userEvent.click(screen.getByRole("button", { name: "Choose recording mode" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Room" }));
    expect(onModeChange).toHaveBeenCalledWith("room");
    expect(onStart).not.toHaveBeenCalled();
  });

  it("starting and stopping are busy statuses", () => {
    const { rerender } = render(<RecordControl state="starting" mode="call" />);
    expect(screen.getByRole("status").textContent).toBe("Checking microphone…");
    expect(screen.getByRole("status").getAttribute("aria-busy")).toBe("true");
    rerender(<RecordControl state="stopping" mode="call" />);
    expect(screen.getByRole("status").textContent).toBe("Saving…");
  });

  it("recording: shows elapsed time, pauses and stops", async () => {
    const onPause = vi.fn();
    const onStop = vi.fn();
    render(<RecordControl state="recording" mode="call" elapsedMs={724_000} onPause={onPause} onStop={onStop} />);
    await userEvent.click(screen.getByRole("button", { name: "Pause" }));
    await userEvent.click(screen.getByRole("button", { name: /^Stop/ }));
    expect(screen.getByRole("button", { name: /^Stop/ }).textContent).toContain("12:04");
    expect(onPause).toHaveBeenCalledOnce();
    expect(onStop).toHaveBeenCalledOnce();
  });

  it("paused: resumes", async () => {
    const onResume = vi.fn();
    render(<RecordControl state="paused" mode="call" elapsedMs={5000} onResume={onResume} />);
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(onResume).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
  });

  it("error is an alert with a Fix action", async () => {
    const onFix = vi.fn();
    render(<RecordControl state="error" mode="call" onFix={onFix} />);
    expect(screen.getByRole("alert").textContent).toContain("Microphone unavailable");
    await userEvent.click(screen.getByRole("button", { name: "Fix" }));
    expect(onFix).toHaveBeenCalledOnce();
  });

  it("the record shape morphs: circle idle, rounded square recording", () => {
    const { container, rerender } = render(<RecordControl state="idle" mode="call" />);
    expect(container.querySelector("span[aria-hidden]")?.className).toContain("rounded-full");
    rerender(<RecordControl state="recording" mode="call" />);
    expect(container.querySelector("span[aria-hidden]")?.className).toContain("rounded-[3px]");
  });
});
