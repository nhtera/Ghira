// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderLive } from "../live/test-utils";
import { ConsentDialog } from "./consent-dialog";

afterEach(cleanup);

const setup = () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  renderLive(<ConsentDialog open name="Linh" onConfirm={onConfirm} onCancel={onCancel} />);
  return { onConfirm, onCancel };
};

describe("ConsentDialog", () => {
  it("has no default choice and Save is off until one is picked", () => {
    setup();
    const radios = screen.getAllByRole("radio") as HTMLInputElement[];
    expect(radios).toHaveLength(2);
    expect(radios.every((r) => !r.checked)).toBe(true);
    expect((screen.getByRole("button", { name: "Save voice profile" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("Choose one to continue.")).toBeTruthy();
  });

  it("yes saves the voice profile", async () => {
    const { onConfirm } = setup();
    await userEvent.click(screen.getByRole("radio", { name: /Yes, Linh agreed/ }));
    await userEvent.click(screen.getByRole("button", { name: "Save voice profile" }));
    expect(onConfirm).toHaveBeenCalledWith(true);
  });

  it("not yet saves the name only", async () => {
    const { onConfirm } = setup();
    await userEvent.click(screen.getByRole("radio", { name: /Not yet/ }));
    await userEvent.click(screen.getByRole("button", { name: "Save name only" }));
    expect(onConfirm).toHaveBeenCalledWith(false);
  });

  it("Escape and Cancel save nothing", async () => {
    const { onConfirm, onCancel } = setup();
    await userEvent.keyboard("{Escape}");
    expect(onCancel).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledTimes(2);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("clicking outside does not dismiss", async () => {
    const { onCancel } = setup();
    fireEvent.pointerDown(document.body);
    expect(onCancel).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });
});
