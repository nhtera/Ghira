// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ModelRow, type ModelRowProps } from "./model-row";

afterEach(cleanup);

const base: ModelRowProps = {
  purpose: "Live English transcript",
  name: "Parakeet TDT 0.6B v3",
  size: "0.7 GB",
  memory: "~1 GB",
  languages: "EN + 24",
  license: "CC-BY-4.0",
  status: "installed",
};

describe("ModelRow", () => {
  it("shows purpose, name, size, memory, languages and license", () => {
    render(<ModelRow {...base} />);
    expect(screen.getByText("Live English transcript")).toBeTruthy();
    expect(screen.getByText(/Parakeet TDT 0.6B v3 · Memory ~1 GB · EN \+ 24 · License CC-BY-4.0/)).toBeTruthy();
    expect(screen.getByText("0.7 GB")).toBeTruthy();
    expect(screen.getByText("Installed")).toBeTruthy();
  });

  it("installed offers Remove only", async () => {
    const onRemove = vi.fn();
    render(<ModelRow {...base} onRemove={onRemove} onDownload={() => {}} />);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    await userEvent.click(screen.getByRole("button", { name: "Remove Parakeet TDT 0.6B v3" }));
    expect(onRemove).toHaveBeenCalledOnce();
  });

  it("downloading shows a progressbar and Pause", async () => {
    const onPause = vi.fn();
    render(<ModelRow {...base} status="downloading" progress={35} onPause={onPause} />);
    expect(screen.getByRole("progressbar", { name: base.name }).getAttribute("aria-valuenow")).toBe("35");
    expect(screen.getByText("Downloading 35%")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /^Pause/ }));
    expect(onPause).toHaveBeenCalledOnce();
  });

  it("paused shows percent and Resume", async () => {
    const onResume = vi.fn();
    render(<ModelRow {...base} status="paused" progress={41} onResume={onResume} />);
    expect(screen.getByText("Paused at 41%")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /^Resume/ }));
    expect(onResume).toHaveBeenCalledOnce();
  });

  it("update available offers Update and Remove", async () => {
    const onUpdate = vi.fn();
    render(<ModelRow {...base} status="update" onUpdate={onUpdate} onRemove={() => {}} />);
    expect(screen.getByText("Update available")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /^Update/ }));
    expect(onUpdate).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: /^Remove/ })).toBeTruthy();
  });

  it("preview offers Download", async () => {
    const onDownload = vi.fn();
    render(<ModelRow {...base} status="preview" onDownload={onDownload} />);
    await userEvent.click(screen.getByRole("button", { name: /^Download/ }));
    expect(onDownload).toHaveBeenCalledOnce();
  });

  it("incompatible hardware explains why and has no actions", () => {
    render(<ModelRow {...base} status="incompatible" needsMemory="32 GB" onDownload={() => {}} onRemove={() => {}} />);
    expect(screen.getByText("Needs 32 GB memory")).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });
});
