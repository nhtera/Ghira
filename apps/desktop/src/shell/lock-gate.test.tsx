// SPDX-License-Identifier: Apache-2.0
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "@ghi/ui";
import type { LockChanged } from "../bindings";
import { ipc } from "../ipc";
import { useLock } from "../state/lock";
import { LockGate } from "./lock-gate";

function setup(locked: boolean, mode: "full" | "controls" = "full") {
  let emit: (e: LockChanged) => void = () => {};
  vi.spyOn(ipc.commands, "lockState").mockResolvedValue({ status: "ok", data: locked });
  vi.spyOn(ipc, "onLockChanged").mockImplementation((cb) => {
    emit = cb;
    return Promise.resolve(() => {});
  });
  const client = new QueryClient();
  client.setQueryData(["meeting", "m1", "notes"], { secret: true });
  render(
    <QueryClientProvider client={client}>
      <PlatformProvider value="mac">
        <LockGate mode={mode}>
          <p>meeting content</p>
        </LockGate>
      </PlatformProvider>
    </QueryClientProvider>,
  );
  return { client, emit: (e: LockChanged) => act(() => emit(e)) };
}

describe("LockGate", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    useLock.setState({ locked: null });
  });

  it("keeps a panel's controls while locked and tells the page", async () => {
    setup(true, "controls");
    expect(await screen.findByText("meeting content")).toBeTruthy();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(useLock.getState().locked).toBe(true);
  });

  it("shows nothing until it knows, then the content when unlocked", async () => {
    setup(false);
    expect(screen.queryByText("meeting content")).toBeNull();
    expect(await screen.findByText("meeting content")).toBeTruthy();
  });

  it("while locked renders only the lock screen and drops cached data", async () => {
    const { client } = setup(true);
    expect(await screen.findByRole("alertdialog")).toBeTruthy();
    expect(screen.queryByText("meeting content")).toBeNull();
    expect(client.getQueryData(["meeting", "m1", "notes"])).toBeUndefined();
  });

  it("unlocks through the core and follows lock events", async () => {
    const { emit } = setup(true);
    const unlock = vi.spyOn(ipc.commands, "unlock").mockResolvedValue({ status: "ok", data: true });
    fireEvent.click(await screen.findByRole("button", { name: /Unlock with Touch ID/ }));
    expect(unlock).toHaveBeenCalledWith("unlock your meetings");
    emit({ locked: false });
    expect(await screen.findByText("meeting content")).toBeTruthy();
    emit({ locked: true });
    expect(await screen.findByRole("alertdialog")).toBeTruthy();
  });

  it("Use password opens the same system prompt", async () => {
    setup(true);
    const unlock = vi.spyOn(ipc.commands, "unlock").mockResolvedValue({ status: "ok", data: true });
    fireEvent.click(await screen.findByRole("button", { name: "Use password" }));
    expect(unlock).toHaveBeenCalledWith("unlock your meetings");
  });
});
