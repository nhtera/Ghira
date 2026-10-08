// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PlatformProvider } from "../platform/platform";
import { Dialog } from "./dialog";

afterEach(cleanup);

const shape = (platform: "mac" | "win", placement?: "sheet" | "center") => {
  render(
    <PlatformProvider value={platform}>
      <Dialog open onOpenChange={() => {}} title="T" placement={placement} />
    </PlatformProvider>,
  );
  return screen.getByRole("dialog").getAttribute("data-shape");
};

describe("Dialog placement", () => {
  it("is a sheet on mac by default and centered on Windows", () => {
    expect(shape("mac")).toBe("sheet");
    cleanup();
    expect(shape("win")).toBe("dialog");
  });

  it("center keeps mac centered", () => {
    expect(shape("mac", "center")).toBe("dialog");
  });
});

describe("Dialog close button", () => {
  it("closes a dismissible dialog from its header", () => {
    const onOpenChange = vi.fn();
    render(
      <PlatformProvider value="mac">
        <Dialog open onOpenChange={onOpenChange} title="T" />
      </PlatformProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("is not there when the choice must be explicit", () => {
    render(
      <PlatformProvider value="mac">
        <Dialog open onOpenChange={() => {}} title="T" dismissible={false} />
      </PlatformProvider>,
    );
    expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
  });
});
