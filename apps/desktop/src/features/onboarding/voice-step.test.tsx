// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "@ghi/ui";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const commands = vi.hoisted(() => ({
  voiceStatus: vi.fn(),
  enrollVoiceStart: vi.fn(),
  enrollVoiceLevel: vi.fn(),
  enrollVoiceFinish: vi.fn(),
  enrollVoiceCancel: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderLive } from "../live/test-utils";
import { VoiceStep } from "./voice-step";

const nav = { next: vi.fn(), back: vi.fn(), finish: vi.fn(), canGoBack: false };
const show = (props: { voiceDownloading?: boolean; strictOffline?: boolean } = {}) =>
  renderLive(
    <PlatformProvider value="mac">
      <VoiceStep nav={nav} {...props} />
    </PlatformProvider>,
  );
const start = () => screen.getByRole("button", { name: /Start reading|Try again/ }) as HTMLButtonElement;

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  nav.next.mockReset();
  commands.voiceStatus.mockReturnValue(ok({ modelReady: true, meProfile: null, enrolling: false }));
  commands.enrollVoiceStart.mockReturnValue(ok(null));
  commands.enrollVoiceLevel.mockReturnValue(ok({ level: 0.3, seconds: 12, maxSeconds: 25, done: false }));
  commands.enrollVoiceFinish.mockReturnValue(ok(null));
  commands.enrollVoiceCancel.mockReturnValue(ok(null));
});
afterEach(cleanup);

describe("onboarding voice step", () => {
  it("opens the mic only after consent, reads, and stores with the consent text key", async () => {
    show();
    expect(start().disabled).toBe(true);
    fireEvent.click(screen.getByRole("checkbox"));
    expect(start().disabled).toBe(false);
    fireEvent.click(start());
    await waitFor(() => expect(commands.enrollVoiceStart).toHaveBeenCalledOnce());
    expect(commands.enrollVoiceFinish).not.toHaveBeenCalled();
    await waitFor(() => expect(commands.enrollVoiceLevel).toHaveBeenCalled());
    expect(await screen.findByText(/Reading… 12 s/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop and save" }));
    await waitFor(() => expect(commands.enrollVoiceFinish).toHaveBeenCalledWith("onboarding.voice.consent_mac"));
    expect(await screen.findByText("Your voice is saved")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(nav.next).toHaveBeenCalled();
  });

  it("saves by itself when the core says the mic closed", async () => {
    commands.enrollVoiceLevel.mockReturnValue(ok({ level: 0.1, seconds: 25, maxSeconds: 25, done: true }));
    show();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(start());
    await waitFor(() => expect(commands.enrollVoiceFinish).toHaveBeenCalledOnce());
  });

  it.each([
    ["micPermission", "start", /Microphone access is off/],
    ["noMic", "start", /No microphone was found/],
    ["noModel", "start", /voice model isn.t installed/],
    ["tooShort", "finish", /too short/],
    ["tooQuiet", "finish", /barely hear you/],
  ])("%s is explained and can be retried", async (code, where, text) => {
    (where === "start" ? commands.enrollVoiceStart : commands.enrollVoiceFinish).mockReturnValue(err(code));
    show();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(start());
    if (where === "finish") fireEvent.click(await screen.findByRole("button", { name: "Stop and save" }));
    expect((await screen.findByRole("alert")).textContent).toMatch(text);
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("releases the mic when the step is left mid-read, and Skip is always there", async () => {
    const { unmount } = show();
    expect(screen.getByRole("button", { name: "Skip for now" })).toBeTruthy();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(start());
    await screen.findByRole("button", { name: "Stop and save" });
    expect(screen.getByRole("button", { name: "Skip for now" })).toBeTruthy();
    unmount();
    expect(commands.enrollVoiceCancel).toHaveBeenCalledOnce();
    expect(commands.enrollVoiceFinish).not.toHaveBeenCalled();
  });

  it("an enrollment the core dropped (app lock) says to start again", async () => {
    commands.enrollVoiceLevel.mockReturnValue(err("notEnrolling"));
    show();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(start());
    expect((await screen.findByRole("alert")).textContent).toMatch(/Start again/);
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    expect(commands.enrollVoiceCancel).not.toHaveBeenCalled();
  });

  it("does not open the mic when it was never started", () => {
    const { unmount } = show();
    unmount();
    expect(commands.enrollVoiceCancel).not.toHaveBeenCalled();
  });

  it.each<[{ voiceDownloading?: boolean; strictOffline?: boolean }, RegExp]>([
    [{ voiceDownloading: true }, /voice model is still downloading/],
    [{ strictOffline: true }, /strict offline is on/],
    [{}, /voice model isn.t installed yet/],
  ])("a voice model that is not there says why (%o), blocks only Start, and Skip still works", async (props, text) => {
    commands.voiceStatus.mockReturnValue(ok({ modelReady: false, meProfile: null, enrolling: false }));
    show(props);
    expect(await screen.findByText(text)).toBeTruthy();
    // "Still downloading" is only said while a download is really running.
    if (!props.voiceDownloading) expect(screen.queryByText(/still downloading/)).toBeNull();
    fireEvent.click(screen.getByRole("checkbox"));
    expect(start().disabled).toBe(true);
    await act(async () => fireEvent.click(screen.getByRole("button", { name: "Skip for now" })));
    expect(nav.next).toHaveBeenCalled();
    expect(commands.enrollVoiceStart).not.toHaveBeenCalled();
  });

  it("leaving while the mic is still opening closes it again", async () => {
    let resolve!: (v: unknown) => void;
    commands.enrollVoiceStart.mockReturnValue(new Promise((r) => (resolve = r)));
    const { unmount } = show();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(start());
    await waitFor(() => expect(commands.enrollVoiceStart).toHaveBeenCalledOnce());
    unmount();
    expect(commands.enrollVoiceCancel).not.toHaveBeenCalled();
    await act(async () => resolve({ status: "ok", data: null }));
    expect(commands.enrollVoiceCancel).toHaveBeenCalledOnce();
  });

  it("leaving while it saves sends no cancel", async () => {
    let resolve!: (v: unknown) => void;
    commands.enrollVoiceFinish.mockReturnValue(new Promise((r) => (resolve = r)));
    const { unmount } = show();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(start());
    fireEvent.click(await screen.findByRole("button", { name: "Stop and save" }));
    await waitFor(() => expect(commands.enrollVoiceFinish).toHaveBeenCalledOnce());
    unmount();
    await act(async () => resolve({ status: "ok", data: null }));
    expect(commands.enrollVoiceCancel).not.toHaveBeenCalled();
  });
});
