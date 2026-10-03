// SPDX-License-Identifier: Apache-2.0
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Kbd, kbdKeys } from "./kbd";

afterEach(cleanup);

describe("Kbd", () => {
  it("splits mac symbols and Windows keys", () => {
    expect(kbdKeys("⌘⇧R")).toEqual(["⌘", "⇧", "R"]);
    expect(kbdKeys("Ctrl+Shift+R")).toEqual(["Ctrl", "Shift", "R"]);
  });

  it("renders the chord in one kbd element", () => {
    const { container } = render(<Kbd shortcut="⌘⇧R" size="lg" />);
    const el = container.querySelector("kbd");
    expect(el?.textContent).toBe("⌘⇧R");
    expect(el?.getAttribute("data-size")).toBe("lg");
  });
});
