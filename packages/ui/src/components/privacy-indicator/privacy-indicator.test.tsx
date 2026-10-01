// SPDX-License-Identifier: Apache-2.0
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { PrivacyIndicator, type PrivacyState } from "./privacy-indicator";

const TEXT: Record<PrivacyState, string> = {
  local: "Local only",
  recording: "Recording · Local only",
  paused: "Paused · Local only",
  cloudMeeting: "Cloud used for this meeting",
  cloudToday: "Cloud used today",
};

afterEach(cleanup);

describe("PrivacyIndicator", () => {
  it.each(Object.entries(TEXT))("%s is a status with an icon and text", (state, text) => {
    const { container } = render(<PrivacyIndicator state={state as PrivacyState} />);
    expect(screen.getByRole("status").textContent).toBe(text);
    expect(container.querySelector("svg[data-icon]")).toBeTruthy();
  });

  it("only the recording state has the red dot", () => {
    const { container, rerender } = render(<PrivacyIndicator state="recording" />);
    expect(container.querySelector("span[aria-hidden]")).toBeTruthy();
    rerender(<PrivacyIndicator state="local" />);
    expect(container.querySelector("span[aria-hidden]")).toBeNull();
  });
});
