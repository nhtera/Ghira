// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CitationChip } from "./citation-chip";

afterEach(cleanup);

describe("CitationChip", () => {
  it("is a button named after the moment and fires onClick", async () => {
    const onClick = vi.fn();
    render(<CitationChip timeMs={572_000} onClick={onClick} />);
    const b = screen.getByRole("button", { name: "Show in transcript 09:32" });
    expect(b.textContent).toBe("09:32");
    await userEvent.click(b);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("falls back to the source index", () => {
    render(<CitationChip index={3} />);
    expect(screen.getByRole("button", { name: "Show source 3 in transcript" }).textContent).toBe("3");
  });

  it("marks visited and broken sources in text/icon, not color alone", () => {
    const { container, rerender } = render(<CitationChip timeMs={1000} visited />);
    expect(screen.getByRole("button").dataset.state).toBe("visited");
    expect(container.querySelector('svg[data-icon="check"]')).toBeTruthy();
    rerender(<CitationChip timeMs={1000} broken text="Tue" />);
    const b = screen.getByRole("button", { name: /audio was deleted/ });
    expect(b.dataset.state).toBe("broken");
    expect(b.textContent).toBe("Tue");
    expect(b.className).toContain("border-dashed");
  });

  it("keeps a 24 px minimum target", () => {
    render(<CitationChip index={1} />);
    expect(screen.getByRole("button").className).toContain("h-6");
    expect(screen.getByRole("button").className).toContain("min-w-6");
  });
});
