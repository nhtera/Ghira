// SPDX-License-Identifier: Apache-2.0
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ipc } from "../../ipc";
import { LiveModeCard } from "./recording-section";
import { renderSettings } from "./test-utils";


afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const tier = async (tier: string) => {
  const r = await ipc.commands.modelsStatus();
  if (r.status !== "ok") throw new Error("no models");
  vi.spyOn(ipc.commands, "modelsStatus").mockResolvedValue({ status: "ok", data: { ...r.data, tier } });
};

describe("live transcript mode", () => {
  it("passes the choice on and describes the selected option", async () => {
    const user = userEvent.setup();
    await tier("balanced");
    const change = vi.fn();
    renderSettings(<LiveModeCard mode="auto" onChange={change} />);
    const group = await screen.findByRole("radiogroup", { name: "Live transcript" });
    expect(screen.getByText("Chosen for this computer’s hardware.")).toBeTruthy();
    expect(screen.getByText("Applies from the next recording.")).toBeTruthy();
    await user.click(within(group).getByRole("radio", { name: "Accurate" }));
    expect(change).toHaveBeenCalledWith("accurate");
  });

  it("is fixed to Fast and disabled on an 8 GB computer", async () => {
    await tier("light");
    renderSettings(<LiveModeCard mode="accurate" onChange={vi.fn()} />);
    expect(await screen.findByText("This Mac uses Fast mode (8 GB memory).")).toBeTruthy();
    // A disabled fieldset disables every control inside it.
    await waitFor(() => expect((screen.getByRole("group") as HTMLFieldSetElement).disabled).toBe(true));
    expect(screen.getByRole("radio", { name: "Fast" }).getAttribute("aria-checked")).toBe("true");
  });
});
