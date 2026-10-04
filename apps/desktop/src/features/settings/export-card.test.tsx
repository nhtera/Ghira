// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const ok = <T,>(data: T) => Promise.resolve({ status: "ok" as const, data });
const err = (error: string) => Promise.resolve({ status: "error" as const, error });
const commands = vi.hoisted(() => ({ exportDestination: vi.fn(), chooseExportFolder: vi.fn(), obsidianVault: vi.fn(), chooseObsidianVault: vi.fn() }));
vi.mock("../../ipc", () => ({ ipc: { commands } }));

import { renderSettings } from "./test-utils";
import { ExportCard } from "./export-card";

beforeEach(() => {
  Object.values(commands).forEach((c) => c.mockReset());
  commands.exportDestination.mockReturnValue(ok("Documents"));
  commands.obsidianVault.mockReturnValue(ok(null));
});
afterEach(cleanup);

describe("Settings → Export", () => {
  it("shows each folder by name in its own row; none yet says so", async () => {
    renderSettings(<ExportCard />);
    await waitFor(() => expect(screen.getByTestId("export-folder-name").textContent).toBe("Documents"));
    expect(screen.getByTestId("obsidian-vault-name").textContent).toBe("Not chosen yet");
  });

  it("a failed read shows an error with Try again, not 'Not chosen yet'", async () => {
    commands.obsidianVault.mockReturnValueOnce(err("storage")).mockReturnValue(ok("Vault"));
    renderSettings(<ExportCard />);
    expect(await screen.findByTestId("obsidian-vault-error")).toBeTruthy();
    expect(screen.queryByTestId("obsidian-vault-name")).toBeNull();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.getByTestId("obsidian-vault-name").textContent).toBe("Vault"));
  });

  it("a folder that could not be remembered is an error and is not shown", async () => {
    commands.chooseObsidianVault.mockReturnValue(err("disk full"));
    renderSettings(<ExportCard />);
    await waitFor(() => expect(screen.getByTestId("obsidian-vault-name").textContent).toBe("Not chosen yet"));
    await userEvent.setup().click(screen.getByRole("button", { name: "Obsidian vault folder: Change…" }));
    expect(await screen.findByText(/disk full/)).toBeTruthy();
    expect(screen.getByTestId("obsidian-vault-name").textContent).toBe("Not chosen yet");
  });

  it("a chosen folder replaces the name", async () => {
    commands.chooseExportFolder.mockReturnValue(ok("Notes"));
    renderSettings(<ExportCard />);
    await waitFor(() => expect(screen.getByTestId("export-folder-name").textContent).toBe("Documents"));
    await userEvent.setup().click(screen.getByRole("button", { name: "Export folder: Change…" }));
    await waitFor(() => expect(screen.getByTestId("export-folder-name").textContent).toBe("Notes"));
  });
});
