// SPDX-License-Identifier: Apache-2.0
import {
  act,
  cleanup,
  fireEvent,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CloudPreview } from "../../bindings";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const commands = vi.hoisted(() => ({
  cloudKeys: vi.fn(),
  cloudModels: vi.fn(),
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  cloudPreview: vi.fn(),
  cloudSend: vi.fn(),
}));
vi.mock("../../ipc", () => ({ ipc: { commands } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

import { renderLive } from "../live/test-utils";
import { CloudSheet, type CloudSheetProps } from "./cloud-sheet";
import { PREVIEW_DEBOUNCE_MS } from "./use-cloud-preview";

// Keys not yet in the locale files render as the key itself; match either.
const L = (key: string, en: string) =>
  new RegExp(`^(${key.replace(/\./g, "\\.")}|${en})`);

let n = 0;
let payloadOverride: string | undefined;
const preview = (over: Partial<CloudPreview> = {}): CloudPreview => ({
  id: `plan-${++n}`,
  provider: "openai",
  model: "gpt-4.1-mini",
  host: "api.openai.com",
  payload: payloadOverride ?? '{"messages":[{"content":"hello [PERSON_1]"}]}',
  sha256: "ab".repeat(32),
  tokensEst: 1200,
  costEstUsd: 0.04,
  costMaxUsd: 0.04,
  retentionNote: "Kept 30 days.",
  warnings: [],
  redactions: [{ kind: "person", count: 1 }],
  excerptBefore: null,
  excerptAfter: null,
  ...over,
});

const setup = (props: Partial<CloudSheetProps> = {}, stored = ["openai"]) => {
  commands.cloudKeys.mockReturnValue(
    ok(
      ["openai", "anthropic"].map((provider) => ({
        provider,
        stored: stored.includes(provider),
      })),
    ),
  );
  commands.cloudModels.mockResolvedValue([
    { provider: "openai", model: "gpt-4.1-mini" },
    { provider: "openai", model: "gpt-4.1" },
    { provider: "anthropic", model: "claude-haiku-4-5" },
  ]);
  commands.getSettings.mockReturnValue(
    ok({
      cloudProvider: "openai",
      cloudModel: "gpt-4.1-mini",
      cloudRedact: true,
      strictOffline: false,
    }),
  );
  commands.updateSettings.mockReturnValue(ok({}));
  commands.cloudPreview.mockImplementation(() =>
    ok({ kind: "preview", ...preview() }),
  );
  return renderLive(
    <CloudSheet
      open
      onOpenChange={vi.fn()}
      meeting="m1"
      locked={false}
      task={{ kind: "notes" }}
      {...props}
    />,
  );
};

const settle = () =>
  act(() => vi.advanceTimersByTimeAsync(PREVIEW_DEBOUNCE_MS + 20));
const sendButton = () =>
  screen.getByRole("button", { name: L("cloud.send", "Send and improve") });

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  Object.values(commands).forEach((c) => c.mockReset());
  payloadOverride = undefined;
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("CloudSheet", () => {
  it("previews the exact payload as text, with host, tokens and retention", async () => {
    setup();
    await settle();
    const pre = await screen.findByTestId("cloud-payload");
    expect(pre.textContent).toBe(
      '{"messages":[{"content":"hello [PERSON_1]"}]}',
    );
    expect(pre.querySelector("*")).toBeNull();
    // The destination is a visible row, and also named by the exact-data disclosure.
    expect(screen.getAllByText(/api\.openai\.com|cloud\.sheet\.host/).length).toBeGreaterThan(1);
    // The provider by its name, in the app's language (not the core's English note).
    expect(screen.getByText(/OpenAI handles this text on its servers/)).toBeTruthy();
    expect(commands.cloudPreview).toHaveBeenCalledWith(
      "m1",
      expect.objectContaining({
        provider: "openai",
        model: "gpt-4.1-mini",
        redact: true,
        task: { kind: "notes", template: null, language: "meeting" },
      }),
    );
  });

  it("counts and excerpts only the user message, never a longer system prompt", async () => {
    const system = "Write careful meeting notes. ".repeat(60);
    payloadOverride = JSON.stringify({ messages: [{ role: "system", content: system }, { role: "user", content: "one two three" }] });
    setup();
    await settle();
    expect((await screen.findByTestId("cloud-excerpt")).textContent).toBe("one two three");
    expect(screen.getByText(/3 words/)).toBeTruthy();
    expect(screen.queryByText(/Write careful/, { selector: "[data-testid=cloud-excerpt]" })).toBeNull();
  });

  it("shows the user's text before and after redaction side by side when the core sends both", async () => {
    payloadOverride = JSON.stringify({ messages: [{ role: "user", content: "[PERSON_1]: call me\nlater" }] });
    setup();
    commands.cloudPreview.mockImplementation(() => ok({ kind: "preview", ...preview({ excerptBefore: "Linh: call me", excerptAfter: "[PERSON_1]: call me" }) }));
    await settle();
    expect((await screen.findByTestId("cloud-excerpt-before")).textContent).toBe("Linh: call me");
    expect(screen.getByTestId("cloud-excerpt").textContent).toBe("[PERSON_1]: call me");
  });

  it("ignores an excerpt that is not part of the request body", async () => {
    payloadOverride = JSON.stringify({ messages: [{ role: "user", content: "the real text" }] });
    setup();
    commands.cloudPreview.mockImplementation(() => ok({ kind: "preview", ...preview({ excerptBefore: "x", excerptAfter: "something else" }) }));
    await settle();
    expect((await screen.findByTestId("cloud-excerpt")).textContent).toBe("the real text");
    expect(screen.queryByTestId("cloud-excerpt-before")).toBeNull();
  });

  it("an unknown request shape claims no word count and opens the exact data", async () => {
    payloadOverride = JSON.stringify({ prompt: "mystery body" });
    setup();
    await settle();
    await screen.findByTestId("cloud-payload");
    expect(screen.queryByTestId("cloud-excerpt")).toBeNull();
    expect(screen.queryByText(/words/)).toBeNull();
    expect((screen.getByTestId("cloud-payload").closest("details") as HTMLDetailsElement).open).toBe(true);
  });

  it("re-previews when redact or the model changes, and Send uses the latest id", async () => {
    setup();
    await settle();
    await screen.findByTestId("cloud-payload");
    fireEvent.click(screen.getByRole("switch"));
    expect(sendButton().hasAttribute("disabled")).toBe(true); // the old preview is not sendable
    await settle();
    await waitFor(() => expect(commands.cloudPreview).toHaveBeenCalledTimes(2));
    expect(commands.cloudPreview).toHaveBeenLastCalledWith(
      "m1",
      expect.objectContaining({ redact: false }),
    );
    fireEvent.change(screen.getByRole("combobox"), {
      target: { value: "gpt-4.1" },
    });
    await settle();
    await waitFor(() => expect(commands.cloudPreview).toHaveBeenCalledTimes(3));
    const lastId = (await commands.cloudPreview.mock.results[2].value).data.id;
    commands.cloudSend.mockReturnValue(ok({ kind: "notes" }));
    await waitFor(() =>
      expect(sendButton().hasAttribute("disabled")).toBe(false),
    );
    fireEvent.click(sendButton());
    await waitFor(() =>
      expect(commands.cloudSend).toHaveBeenCalledWith(lastId),
    );
    expect(commands.updateSettings).toHaveBeenCalledWith({
      cloudProvider: "openai",
      cloudModel: "gpt-4.1",
    });
  });

  it("without a key for the provider there is a link to Settings and no Send", async () => {
    setup({}, []);
    await settle();
    expect(
      await screen.findByRole("button", {
        name: L("cloud.sheet.addKey", "Add a key"),
      }),
    ).toBeTruthy();
    expect(sendButton().hasAttribute("disabled")).toBe(true);
    expect(commands.cloudPreview).not.toHaveBeenCalled();
  });

  it("a locked meeting or strict offline shows why and never previews", async () => {
    setup({ locked: true });
    await settle();
    expect(
      await screen.findByText(
        L("cloud.sheet.locked", "Cloud AI is turned off"),
      ),
    ).toBeTruthy();
    expect(sendButton().hasAttribute("disabled")).toBe(true);
    expect(commands.cloudPreview).not.toHaveBeenCalled();
  });

  it("strict offline shows why and never previews", async () => {
    setup();
    commands.getSettings.mockReturnValue(
      ok({
        cloudProvider: "openai",
        cloudModel: "gpt-4.1-mini",
        cloudRedact: true,
        strictOffline: true,
      }),
    );
    cleanup();
    renderLive(
      <CloudSheet
        open
        onOpenChange={vi.fn()}
        meeting="m1"
        locked={false}
        task={{ kind: "notes" }}
      />,
    );
    await settle();
    expect(
      await screen.findByText(L("cloud.sheet.strictOffline", "Strict offline")),
    ).toBeTruthy();
    expect(commands.cloudPreview).not.toHaveBeenCalled();
  });

  it("a failed notes send says the notes are written on this device", async () => {
    setup();
    await settle();
    await waitFor(() =>
      expect(sendButton().hasAttribute("disabled")).toBe(false),
    );
    commands.cloudSend.mockReturnValue(
      ok({ kind: "failed", reason: "503 overloaded", leftDevice: true }),
    );
    fireEvent.click(sendButton());
    expect(await screen.findByText("503 overloaded")).toBeTruthy();
    expect(
      screen.getByText(
        L("cloud.sheet.failedLocal", "Writing them on this device"),
      ),
    ).toBeTruthy();
    expect(
      screen.queryByRole("button", {
        name: L("cloud.send", "Send and improve"),
      }),
    ).toBeNull();
  });

  it("an Ask nothing matched answers without a request", async () => {
    const onAnswer = vi.fn();
    const onOpenChange = vi.fn();
    setup({
      task: { kind: "ask", question: "kubernetes?" },
      onAnswer,
      onOpenChange,
    });
    commands.cloudPreview.mockReturnValue(
      ok({
        kind: "answer",
        answered: false,
        text: "",
        citations: [],
        searched: ["kubernetes"],
        engine: "local",
      }),
    );
    await settle();
    await waitFor(() => expect(onAnswer).toHaveBeenCalledTimes(1));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(commands.cloudSend).not.toHaveBeenCalled();
  });
});
