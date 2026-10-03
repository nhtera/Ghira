// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
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
