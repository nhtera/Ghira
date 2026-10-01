// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Avatar, initialOf } from "./avatar";

afterEach(cleanup);

describe("initialOf", () => {
  it("takes the first letter of the first word and keeps diacritics", () => {
    expect(initialOf("Linh")).toBe("L");
    expect(initialOf("  đặng Thu Hà")).toBe("Đ");
    expect(initialOf("Ông Văn")).toBe("Ô");
    expect(initialOf("")).toBe("");
  });
});

describe("Avatar", () => {
  it("shows the derived initial on the speaker color and is decorative", () => {
    const { container } = render(<Avatar name="Linh" colorSlot={2} />);
    const el = container.firstElementChild as HTMLElement;
    expect(el.textContent).toBe("L");
    expect(el.style.background).toContain("--s2");
    expect(el.getAttribute("aria-hidden")).toBe("true");
  });

  it("is a named image when given a label", () => {
    render(<Avatar name="Linh" label="Linh" />);
    expect(screen.getByRole("img", { name: "Linh" })).toBeTruthy();
  });

  it("renders Me from copy", () => {
    const { container } = render(<Avatar kind="me" />);
    expect(container.firstElementChild?.getAttribute("data-kind")).toBe("me");
    expect(container.textContent).toBe("Me");
  });

  it("renders a group count and an unknown voice", () => {
    const { container, rerender } = render(<Avatar kind="group" count={3} />);
    expect(container.textContent).toBe("+3");
    rerender(<Avatar kind="unknown" />);
    expect(container.firstElementChild?.getAttribute("data-kind")).toBe("unknown");
  });

  it("uses a neutral fill for slot 0", () => {
    const { container } = render(<Avatar name="Others" colorSlot={0} />);
    expect((container.firstElementChild as HTMLElement).style.background).toBe("");
  });
});
