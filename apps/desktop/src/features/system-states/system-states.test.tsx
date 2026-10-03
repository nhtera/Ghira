// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import type { CoreEvent, Event } from "../../bindings";

const core = vi.hoisted(() => {
  const state = {
    recovered: [] as unknown[],
    damaged: false,
    crashed: false,
    coreListeners: new Set<(e: unknown) => void>(),
    downloadListener: null as null | ((e: unknown) => void),
  };
  const commands = {
    takeRecoveredMeetings: vi.fn(async () => ({ status: "ok", data: state.recovered })),
    deleteMeeting: vi.fn(async () => ({ status: "ok", data: null })),
    modelsStatus: vi.fn(async () => ({
      status: "ok",
      data: { tier: "balanced", downloading: false, models: [{ id: "llm", role: "llm", size: 2.5e9, installed: true, partialBytes: 0, damaged: state.damaged }] },
    })),
    downloadModels: vi.fn(async () => ({ status: "ok", data: null })),
    cancelModelDownload: vi.fn(async () => undefined),
    openPrivacySettings: vi.fn(async () => ({ status: "ok", data: null })),
    retryCapture: vi.fn(async () => ({ status: "ok", data: null })),
    diagnosticsStatus: vi.fn(async () => ({ crashedLastRun: state.crashed, reports: 1 })),
    revealDiagnostics: vi.fn(async () => ({ status: "ok", data: null })),
    acknowledgeCrash: vi.fn(async () => undefined),
  };
  return { state, commands };
});
const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));
vi.mock("../../ipc", () => ({
  ipc: {
    kind: "mock",
    commands: core.commands,
    onCoreEvent: async (cb: (e: unknown) => void) => (core.state.coreListeners.add(cb), () => core.state.coreListeners.delete(cb)),
    onModelDownload: async (cb: (e: unknown) => void) => ((core.state.downloadListener = cb), () => (core.state.downloadListener = null)),
  },
}));

import { LockedScreen, SystemStates, UpdateBanner, updateDeferred } from "./index";

const fire = (event: Event) => act(() => core.state.coreListeners.forEach((l) => l({ seq: null, atMs: 0, event } satisfies CoreEvent)));
const renderStates = () =>
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <PlatformProvider value="mac">
        <ToastProvider label="n">
          <SystemStates />
        </ToastProvider>
      </PlatformProvider>
    </QueryClientProvider>,
  );

beforeEach(() => {
  core.state.recovered = [];
  core.state.damaged = false;
  core.state.crashed = false;
  Object.values(core.commands).forEach((f) => f.mockClear());
  navigate.mockClear();
});
afterEach(cleanup);

describe("recovered meetings", () => {
  beforeEach(() => {
    core.state.recovered = [{ gid: "m1", title: "Weekly sync", durationMs: 1_520_000 }];
  });

  it("says what was saved and opens the meeting", async () => {
    renderStates();
    expect(await screen.findByText(/Weekly sync/)).toBeTruthy();
    expect(screen.getByText(/25:20/)).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Recover and write notes" }));
    expect(navigate).toHaveBeenCalledWith({ to: "/meetings/$id/$tab", params: { id: "m1", tab: "notes" } });
  });

  it("discarding asks first, then deletes", async () => {
    const user = userEvent.setup();
    renderStates();
    await user.click(await screen.findByRole("button", { name: "Discard recording" }));
    expect(core.commands.deleteMeeting).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Discard recording" }));
    expect(core.commands.deleteMeeting).toHaveBeenCalledWith("m1");
    await waitFor(() => expect(screen.queryByText(/Weekly sync/)).toBeNull());
  });

  it("can be dismissed with Escape", async () => {
    renderStates();
    await screen.findByText(/Weekly sync/);
    await userEvent.setup().keyboard("{Escape}");
    expect(screen.queryByText(/Weekly sync/)).toBeNull();
  });
});
import { within } from "@testing-library/react";

describe("crash report notice", () => {
  const banner = () => document.querySelector('[data-banner="crash-report"]');

  it("shows nothing after a clean run", async () => {
    renderStates();
    await waitFor(() => expect(core.commands.diagnosticsStatus).toHaveBeenCalled());
    expect(banner()).toBeNull();
  });

  it("opens the report folder and acknowledges", async () => {
    core.state.crashed = true;
    renderStates();
    await waitFor(() => expect(banner()).not.toBeNull());
    await userEvent.setup().click(within(banner() as HTMLElement).getAllByRole("button")[0]);
    expect(core.commands.revealDiagnostics).toHaveBeenCalled();
    expect(core.commands.acknowledgeCrash).toHaveBeenCalled();
    await waitFor(() => expect(banner()).toBeNull());
  });

  it("dismiss acknowledges without opening the folder", async () => {
    core.state.crashed = true;
    renderStates();
    await waitFor(() => expect(banner()).not.toBeNull());
    await userEvent.setup().click(screen.getByRole("button", { name: "Dismiss" }));
    expect(core.commands.acknowledgeCrash).toHaveBeenCalled();
    expect(core.commands.revealDiagnostics).not.toHaveBeenCalled();
    expect(banner()).toBeNull();
  });
});

describe("damaged model", () => {
  it("shows nothing when the models are fine", async () => {
    renderStates();
    await waitFor(() => expect(core.commands.modelsStatus).toHaveBeenCalled());
    expect(screen.queryByText(/model file is damaged/)).toBeNull();
  });

  it("offers a new download and shows its progress", async () => {
    core.state.damaged = true;
    renderStates();
    const again = await screen.findByRole("button", { name: /Download again/ });
    expect(again.textContent).toContain("2.5 GB");
    await userEvent.setup().click(again);
    expect(core.commands.downloadModels).toHaveBeenCalled();
    await act(() => core.state.downloadListener?.({ model: "llm", phase: "downloading", done: 1e9, total: 2.5e9, error: null }));
    expect(await screen.findByText("Downloading 40%")).toBeTruthy();
  });

  it("appears when a core engine error makes the status re-read", async () => {
    renderStates();
    await waitFor(() => expect(core.commands.modelsStatus).toHaveBeenCalledOnce());
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    core.state.damaged = true;
    await fire({ type: "error", meeting: null, kind: "engine", message: "bad model" });
    expect(await screen.findByText(/model file is damaged/)).toBeTruthy();
  });
});

describe("core errors", () => {
  it("a taken mic uses our copy; repeats collapse; dismiss sticks", async () => {
    const user = userEvent.setup();
    renderStates();
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    const e: Event = { type: "error", meeting: null, kind: "capture", message: "device in exclusive mode" };
    await fire(e);
    await fire(e);
    expect(await screen.findAllByText(/Another app is using the microphone/)).toHaveLength(1);
    expect(document.querySelector("[data-banner=capture]")?.getAttribute("role")).toBe("alert");
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    await fire(e);
    expect(screen.queryByText(/Another app is using the microphone/)).toBeNull();
  });

  it("Try again waits for the core: busy, then cleared on recovery", async () => {
    renderStates();
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    await fire({ type: "error", meeting: null, kind: "capture", message: "device in exclusive mode" });
    await userEvent.setup().click(await screen.findByRole("button", { name: "Try again" }));
    expect(core.commands.retryCapture).toHaveBeenCalledOnce();
    // The command returning is not the answer yet.
    expect(screen.getByText(/Another app is using the microphone/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" }).hasAttribute("disabled")).toBe(true);
    await fire({ type: "captureRecovered", meeting: "m1" });
    await waitFor(() => expect(screen.queryByText(/Another app is using the microphone/)).toBeNull());
  });

  it("Try again that fails keeps the notice with the reason and can be pressed again", async () => {
    renderStates();
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    await fire({ type: "error", meeting: null, kind: "capture", message: "device in exclusive mode" });
    await userEvent.setup().click(await screen.findByRole("button", { name: "Try again" }));
    await fire({ type: "captureRetryFailed", meeting: "m1", message: "still busy" });
    expect(await screen.findByText("still busy")).toBeTruthy();
    expect(screen.getByText(/Another app is using the microphone/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" }).hasAttribute("disabled")).toBe(false);
  });

  it("an unknown capture error shows the core's message", async () => {
    renderStates();
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    await fire({ type: "error", meeting: null, kind: "capture", message: "stream stalled" });
    expect(await screen.findByText("stream stalled")).toBeTruthy();
  });

  it("a lost permission opens the pane", async () => {
    renderStates();
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    await fire({ type: "error", meeting: null, kind: "permission", message: "tcc" });
    await userEvent.setup().click(await screen.findByRole("button", { name: "Open System Settings" }));
    expect(core.commands.openPrivacySettings).toHaveBeenCalledWith("systemAudio");
  });

  it("a storage error blocks with a retry; engine errors are not shown", async () => {
    renderStates();
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    await fire({ type: "error", meeting: null, kind: "job", message: "x" });
    expect(screen.queryByRole("alert")).toBeNull();
    await fire({ type: "error", meeting: null, kind: "storage", message: "disk I/O" });
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByText("disk I/O")).toBeTruthy();
    expect(within(alert).getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});

describe("storage errors during a recording", () => {
  it("a write failure with a meeting is a dismissible banner, not the blocking screen", async () => {
    const user = userEvent.setup();
    renderStates();
    await waitFor(() => expect(core.state.coreListeners.size).toBe(2));
    await fire({ type: "error", meeting: "m1", kind: "storage", message: "write failed" });
    const banner = await screen.findByText(/write failed/);
    expect(document.querySelector("[data-banner=storage]")).toBeNull();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(banner.isConnected).toBe(false);
  });
});

describe("update banner", () => {
  it("installs on Restart", async () => {
    const onInstall = vi.fn();
    const onDismiss = vi.fn();
    render(<UpdateBanner version="1.2.0" onInstall={onInstall} onDismiss={onDismiss} />);
    expect(screen.getByText(/1\.2\.0/)).toBeTruthy();
    await userEvent.setup().click(screen.getByRole("button", { name: "Later" }));
    expect(onDismiss).toHaveBeenCalled();
    await userEvent.setup().click(screen.getByRole("button", { name: "Restart" }));
    expect(onInstall).toHaveBeenCalled();
  });

  it("waits while recording or processing", () => {
    render(<UpdateBanner version="1.2.0" onInstall={() => {}} deferred />);
    expect((screen.getByRole("button", { name: "Restart" }) as HTMLButtonElement).disabled).toBe(true);
    expect(updateDeferred("recording", false)).toBe(true);
    expect(updateDeferred("paused", false)).toBe(true);
    expect(updateDeferred("idle", true)).toBe(true);
    expect(updateDeferred("idle", false)).toBe(false);
    expect(updateDeferred("ready", false)).toBe(false);
  });
});

describe("locked screen", () => {
  it("unlocks with the platform's name for it, and offers the password", async () => {
    const user = userEvent.setup();
    const onUnlock = vi.fn();
    const onPassword = vi.fn();
    render(
      <PlatformProvider value="win">
        <LockedScreen onUnlock={onUnlock} onPassword={onPassword} error="cancelled" />
      </PlatformProvider>,
    );
    expect(screen.getByRole("alertdialog", { name: /is locked/ })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Unlock with Windows Hello" }));
    await user.click(screen.getByRole("button", { name: "Use password" }));
    expect(onUnlock).toHaveBeenCalled();
    expect(onPassword).toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("cancelled");
  });
});
