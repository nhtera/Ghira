// SPDX-License-Identifier: Apache-2.0
import { act, cleanup, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initMobileI18n } from "@ghi/i18n/mobile";
import { EnrollMeter, QUIET_AFTER_MS } from "./enroll-panel";
import type { EnrollState } from "./use-voice-enroll";

const state = (level: number): EnrollState => ({ phase: "reading", level, seconds: 3, maxSeconds: 25, error: null });
const view = (level: number) => (
  <I18nextProvider i18n={initMobileI18n("en")}>
    <EnrollMeter state={state(level)} />
  </I18nextProvider>
);
const QUIET_TEXT = "Can’t hear you. Hold the phone closer.";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("EnrollMeter quiet warning", () => {
  it("waits for a continuous stretch below the floor", () => {
    const { rerender } = render(view(0));
    act(() => void vi.advanceTimersByTime(QUIET_AFTER_MS - 100));
    expect(screen.queryByText(QUIET_TEXT)).toBeNull();
    // A word: the clock starts over.
    rerender(view(0.4));
    rerender(view(0));
    act(() => void vi.advanceTimersByTime(QUIET_AFTER_MS - 100));
    expect(screen.queryByText(QUIET_TEXT)).toBeNull();
    act(() => void vi.advanceTimersByTime(200));
    expect(screen.getByText(QUIET_TEXT)).toBeTruthy();
  });

  it("clears at once when the voice is back", () => {
    const { rerender } = render(view(0));
    act(() => void vi.advanceTimersByTime(QUIET_AFTER_MS + 10));
    expect(screen.getByText(QUIET_TEXT)).toBeTruthy();
    rerender(view(0.5));
    expect(screen.getByText("Hearing you")).toBeTruthy();
  });
});
