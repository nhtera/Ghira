// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider, ToastProvider } from "@ghi/ui";
import type { Event, ModelDownload, ModelsStatus, Permission } from "../../bindings";
import { useLive } from "../../state/live";

// A scripted core: the tests choose what it answers and push download events.
const core = vi.hoisted(() => {
  const state = {
    status: null as unknown,
    mic: "undetermined" as string,
    listener: null as null | ((e: unknown) => void),
    systemDb: -25 as number | null,
  };
  const commands = {
    modelsStatus: vi.fn(async () => ({ status: "ok", data: state.status })),
    downloadModels: vi.fn(async () => ({ status: "ok", data: null })),
    cancelModelDownload: vi.fn(async () => undefined),
    micPermission: vi.fn(async () => state.mic),
    requestMicPermission: vi.fn(async () => ({ status: "ok", data: "granted" })),
    openPrivacySettings: vi.fn(async () => ({ status: "ok", data: null })),
    hasRecoveryKey: vi.fn(async () => ({ status: "ok", data: false })),
    createRecoveryKey: vi.fn(async () => Array.from({ length: 24 }, (_, i) => `word${i + 1}`)),
    confirmRecoveryKey: vi.fn(async (w: string[]) => ({ status: "ok", data: w.length === 24 })),
    cancelRecoveryKey: vi.fn(async () => undefined),
    voiceStatus: vi.fn(async () => ({ status: "ok", data: { modelReady: true, meProfile: null, enrolling: false } })),
    enrollVoiceCancel: vi.fn(async () => ({ status: "ok", data: null })),
    // Like the core: events into the live store, then the meeting ends.
    testCapture: vi.fn(async () => {
      const id = "test-1";
      const emit = (event: Event) => useLive.getState().apply({ seq: null, atMs: Date.now(), event });
      emit({ type: "stateChanged", meeting: id, state: "starting" });
      emit({ type: "stateChanged", meeting: id, state: "recording" });
      emit({ type: "levelMeter", meeting: id, micDbfs: -20, systemDbfs: core.state.systemDb });
      emit({ type: "transcriptPartial", meeting: id, track: 0, text: "Okay, bắt đầu nhé." });
      if (core.state.systemDb === null) emit({ type: "silentSystemTrack", meeting: id } as Event);
      setTimeout(() => emit({ type: "stateChanged", meeting: id, state: "ready" }), 50);
      return { status: "ok", data: id };
    }),
  };
  return { state, commands };
});
vi.mock("../../ipc", () => ({
  ipc: {
    kind: "mock",
    commands: core.commands,
    onCoreEvent: async () => () => {},
    onModelDownload: async (cb: (e: unknown) => void) => {
      core.state.listener = cb;
      return () => (core.state.listener = null);
    },
  },
}));

import { OnboardingFlow } from "./onboarding-flow";
import { resolveStep, type StepId } from "./steps";

// `voice`: undefined = no voice model in the list; otherwise whether it is installed.
const models = (installed: boolean, partial = 0, voice?: boolean): ModelsStatus => ({
  tier: "balanced",
  downloading: false,
  models: [
    { id: "asr-model", role: "asr", size: 1.2e9, installed, partialBytes: partial, damaged: false },
    { id: "llm-model", role: "llm", size: 2.5e9, installed, partialBytes: 0, damaged: false },
    ...(voice === undefined ? [] : [{ id: "voice-model", role: "voice", size: 3e7, installed: voice, partialBytes: 0, damaged: false }]),
  ],
});
const send = (e: ModelDownload) => act(() => core.state.listener?.(e));

function Harness({ start = "welcome", voice = false, strictOffline = false, onFinish = vi.fn() }: { start?: StepId; voice?: boolean; strictOffline?: boolean; onFinish?: () => void }) {
  const [step, setStep] = useState<StepId>(start);
  const [client] = useState(() => new QueryClient({ defaultOptions: { queries: { retry: false } } }));
  return (
    <QueryClientProvider client={client}>
    <PlatformProvider value="mac">
      <ToastProvider label="notifications">
        <OnboardingFlow
          step={resolveStep(step, voice)}
          onStep={setStep}
          onFinish={onFinish}
          voiceEnabled={voice}
          strictOffline={strictOffline}
          language="auto"
          onLanguage={() => {}}
          testSeconds={1}
        />
      </ToastProvider>
    </PlatformProvider>
    </QueryClientProvider>
  );
}

const current = () => screen.getByRole("listitem", { current: "step" }).textContent;

beforeEach(() => {
  core.state.status = models(true);
  core.state.mic = "undetermined";
  core.state.systemDb = -25;
  useLive.getState().reset();
  Object.values(core.commands).forEach((f) => f.mockClear());
});
afterEach(cleanup);

describe("flow", () => {
  it("walks every step and skips the voice step while voice profiles are off", async () => {
    const user = userEvent.setup();
    const onFinish = vi.fn();
    render(<Harness onFinish={onFinish} />);
    const seen: (string | null)[] = [];
    const go = async (name: RegExp | string) => {
      seen.push(current());
      await user.click(screen.getByRole("button", { name }));
    };
    await go("Get started");
    await go("Continue"); // languages
    await waitFor(() => expect(screen.getByRole("progressbar", { name: "Download speech models" })).toBeTruthy());
    await go("Continue"); // models
    await go("Continue"); // permissions
    await go("Skip"); // test
    await go("Set up later"); // recovery
    seen.push(current());
    expect(seen.map((x) => x?.replace(/^\d/, ""))).toEqual(["Welcome", "Languages", "Speech models", "Permissions", "Test recording", "Recovery key", "Done"]);
    await user.click(screen.getByRole("button", { name: "Open my meetings" }));
    expect(onFinish).toHaveBeenCalledOnce();
  });

  it("includes the voice step when voice profiles are on and the voice model is installed", async () => {
    core.state.status = models(true, 0, true);
    render(<Harness start="permissions" voice />);
    await waitFor(() => expect(screen.getAllByRole("listitem").some((li) => li.textContent?.includes("Your voice"))).toBe(true));
    await userEvent.setup().click(screen.getByRole("button", { name: "Continue" }));
    expect(current()).toContain("Your voice");
  });

  it("skips the voice step silently while the voice model is missing and nothing is downloading it", async () => {
    core.state.status = models(true, 0, false);
    render(<Harness start="permissions" voice strictOffline />);
    await waitFor(() => expect(core.commands.modelsStatus).toHaveBeenCalled());
    const rail = () => within(screen.getByRole("navigation")).getAllByRole("listitem");
    await waitFor(() => expect(rail()).toHaveLength(7));
    expect(rail().some((li) => li.textContent?.includes("Your voice"))).toBe(false);
    // A typed URL for the step lands on the next one.
    cleanup();
    render(<Harness start="voice" voice strictOffline />);
    await waitFor(() => expect(current()).toContain("Test recording"));
  });

  it("Enter continues, Escape does nothing, and Back goes back", async () => {
    const user = userEvent.setup();
    render(<Harness start="languages" />);
    await user.keyboard("{Escape}");
    expect(current()).toContain("Languages");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(current()).toContain("Speech models"));
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(current()).toContain("Languages");
  });

  it("the first step has no Back; Skip for now finishes", async () => {
    const onFinish = vi.fn();
    render(<Harness onFinish={onFinish} />);
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: "Skip for now" }));
    expect(onFinish).toHaveBeenCalled();
  });
});

describe("models step", () => {
  it("starts the download and shows progress, then done", async () => {
    core.state.status = models(false);
    render(<Harness start="models" />);
    await waitFor(() => expect(core.commands.downloadModels).toHaveBeenCalledOnce());
    await send({ model: "asr-model", phase: "downloading", done: 6e8, total: 1.2e9, error: null });
    expect(screen.getByRole("progressbar", { name: "Download speech models" }).getAttribute("aria-valuenow")).toBe("16");
    // Continue is never blocked while downloading.
    expect((screen.getByRole("button", { name: "Continue" }) as HTMLButtonElement).disabled).toBe(false);
    core.state.status = models(true);
    await send({ model: "asr-model", phase: "done", done: 1.2e9, total: 1.2e9, error: null });
    await send({ model: "llm-model", phase: "done", done: 2.5e9, total: 2.5e9, error: null });
    await waitFor(() => expect(screen.getAllByText("Downloaded and checked").length).toBeGreaterThan(0));
  });

  it("a missing voice model does not hold up done, and is fetched on its own", async () => {
    core.state.status = models(true, 0, false);
    render(<Harness start="models" />);
    await waitFor(() => expect(screen.getAllByText("Downloaded and checked").length).toBeGreaterThan(0));
    await waitFor(() => expect(core.commands.downloadModels).toHaveBeenCalledOnce());
  });

  it("strict offline: the voice model is not fetched", async () => {
    core.state.status = models(true, 0, false);
    render(<Harness start="models" strictOffline />);
    await waitFor(() => expect(screen.getAllByText("Downloaded and checked").length).toBeGreaterThan(0));
    expect(core.commands.downloadModels).not.toHaveBeenCalled();
  });

  it("a failed download offers Resume and Record now", async () => {
    core.state.status = models(false, 3e8);
    render(<Harness start="models" />);
    await waitFor(() => expect(core.commands.downloadModels).toHaveBeenCalledOnce());
    await send({ model: "asr-model", phase: "failed", done: 3e8, total: 1.2e9, error: "offline" });
    const resume = await screen.findByRole("button", { name: "Resume download" });
    await userEvent.setup().click(resume);
    expect(core.commands.downloadModels).toHaveBeenCalledTimes(2);
    await send({ model: "asr-model", phase: "failed", done: 3e8, total: 1.2e9, error: "offline" });
    await userEvent.setup().click(await screen.findByRole("button", { name: "Record now, process later" }));
    expect(current()).toContain("Permissions");
  });

  it("strict offline downloads nothing and says so", async () => {
    core.state.status = models(false);
    render(<Harness start="models" strictOffline />);
    await screen.findByText(/Strict offline is on/);
    expect(core.commands.downloadModels).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Record now, process later" })).toBeTruthy();
  });

  it("offline shows the record-now line", async () => {
    core.state.status = models(false);
    const online = vi.spyOn(navigator, "onLine", "get").mockReturnValue(false);
    render(<Harness start="models" />);
    await screen.findByText(/No internet\?/);
    expect(core.commands.downloadModels).not.toHaveBeenCalled();
    online.mockRestore();
  });

  it("shows the detected tier", async () => {
    render(<Harness start="models" />);
    await waitFor(() => expect(document.querySelector('[aria-current="true"]')?.textContent).toContain("Balanced"));
  });
});

describe("permissions step", () => {
  it("asks only when Allow is pressed (priming), then shows allowed", async () => {
    render(<Harness start="permissions" />);
    const allow = await screen.findByRole("button", { name: "Allow…" });
    await waitFor(() => expect((allow as HTMLButtonElement).disabled).toBe(false));
    expect(core.commands.requestMicPermission).not.toHaveBeenCalled();
    await userEvent.setup().click(allow);
    expect(core.commands.requestMicPermission).toHaveBeenCalledOnce();
    await screen.findByText("Allowed");
  });

  it.each(["denied", "restricted"] as Permission[])("%s explains where to turn it on", async (p) => {
    core.state.mic = p;
    render(<Harness start="permissions" />);
    await screen.findByText(/Turn it on in System Settings/);
    expect(screen.getByText("Off")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Allow…" })).toBeNull();
    await userEvent.setup().click(screen.getAllByRole("button", { name: "Open System Settings" })[0]!);
    expect(core.commands.openPrivacySettings).toHaveBeenCalledWith("microphone");
  });

  it("already granted shows allowed with no Allow button", async () => {
    core.state.mic = "granted";
    render(<Harness start="permissions" />);
    await screen.findByText("Allowed");
    expect(screen.queryByRole("button", { name: "Allow…" })).toBeNull();
  });

  it("macOS system audio explains that the test reveals it", async () => {
    render(<Harness start="permissions" />);
    expect(await screen.findByText(/macOS doesn’t tell/)).toBeTruthy();
    expect(screen.getByText(/Room mode only/)).toBeTruthy();
  });
});

describe("test step", () => {
  it("runs, shows levels and the first line, and finishes with Continue", async () => {
    const user = userEvent.setup();
    render(<Harness start="test" />);
    expect(screen.getAllByRole("meter")).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "Run test" }));
    expect(core.commands.testCapture).toHaveBeenCalledWith(1);
    await waitFor(() => expect(screen.getByText("Both sources are working")).toBeTruthy());
    expect(screen.getByText("Okay, bắt đầu nhé.")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(current()).toContain("Recovery key");
  });

  it("a silent system track says Room mode is what works", async () => {
    core.state.systemDb = null;
    render(<Harness start="test" />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Run test" }));
    await screen.findByText(/No sound reached the system track/);
    expect(screen.queryByText("Both sources are working")).toBeNull();
    expect(screen.getByText(/Room mode only/)).toBeTruthy();
  });

  it("an error from the core is shown and the test can be run again", async () => {
    core.commands.testCapture.mockResolvedValueOnce({ status: "error", data: "mic" } as never);
    render(<Harness start="test" />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Run test" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Run test" }).hasAttribute("disabled")).toBe(false));
    await user.click(screen.getByRole("button", { name: "Run test" }));
    await screen.findByText("Both sources are working");
  });
});

describe("recovery step", () => {
  it("is non-blocking: Set up later continues", async () => {
    render(<Harness start="recovery" />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Set up later" }));
    expect(current()).toContain("Done");
  });

  it("shows 24 words, checks the typed phrase, and forgets an unconfirmed one", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<Harness start="recovery" />);
    await user.click(screen.getByRole("button", { name: "Create recovery key" }));
    expect(await screen.findAllByText(/^word\d+$/)).toHaveLength(24);
    const box = screen.getByRole("textbox");
    await user.type(box, "word1 word2");
    await user.click(screen.getByRole("button", { name: "Check and save" }));
    await screen.findByText(/don’t match/);
    unmount();
    expect(core.commands.cancelRecoveryKey).toHaveBeenCalledOnce();
  });

  it("saves when the phrase matches and moves on", async () => {
    const user = userEvent.setup();
    render(<Harness start="recovery" />);
    await user.click(screen.getByRole("button", { name: "Create recovery key" }));
    await screen.findAllByText(/^word\d+$/);
    await user.click(screen.getByRole("textbox"));
    await user.paste(Array.from({ length: 24 }, (_, i) => `word${i + 1}`).join(" "));
    await user.click(screen.getByRole("button", { name: "Check and save" }));
    await waitFor(() => expect(current()).toContain("Done"));
    expect(core.commands.cancelRecoveryKey).not.toHaveBeenCalled();
  });
});
