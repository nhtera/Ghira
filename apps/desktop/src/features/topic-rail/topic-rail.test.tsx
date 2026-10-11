// SPDX-License-Identifier: Apache-2.0
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TopicView } from "../../bindings";
import { usePlayer } from "../../state/player";
import { TOPIC_RAIL_WIDTH, usePrefs } from "../../state/prefs";
import { TopicOutline, TopicRail, currentTopic, showTopicRail } from "./topic-rail";

afterEach(() => {
  cleanup();
  usePlayer.setState({ src: null, currentMs: 0 });
  usePrefs.setState({ topicRailWidth: TOPIC_RAIL_WIDTH.default, topicRailCollapsed: false });
});

const topics = (n: number): TopicView[] => Array.from({ length: n }, (_, i) => ({ title: `Topic ${i}`, tMs: i * 60_000 }));
const MIN = 60_000;

describe("showTopicRail", () => {
  it("shows for an hour, four topics, or two topics in 20 minutes", () => {
    expect(showTopicRail(61 * MIN, topics(1))).toBe(true);
    expect(showTopicRail(10 * MIN, topics(4))).toBe(true);
    expect(showTopicRail(20 * MIN, topics(2))).toBe(true);
    expect(showTopicRail(25 * MIN, topics(3))).toBe(true);
  });

  it("stays hidden below the thresholds", () => {
    expect(showTopicRail(19 * MIN, topics(3))).toBe(false);
    expect(showTopicRail(30 * MIN, topics(1))).toBe(false);
    expect(showTopicRail(61 * MIN, [])).toBe(false);
    expect(showTopicRail(null, topics(2))).toBe(false);
  });
});

describe("currentTopic", () => {
  it("tracks the topic being discussed", () => {
    const t = [{ tMs: 0 }, { tMs: 5000 }, { tMs: 9000 }];
    expect([-1, 0, 4999, 5000, 20_000].map((ms) => currentTopic(t, ms))).toEqual([-1, 0, 0, 1, 2]);
  });
});

describe("TopicOutline (compact row below 1100 px)", () => {
  it("is collapsed until opened, then lists the topics and jumps", () => {
    const onJump = vi.fn();
    render(<TopicOutline topics={topics(3)} onJump={onJump} />);
    const toggle = screen.getByRole("button", { name: "Outline (3)" });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("Topic 1")).toBeNull();
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: /Topic 1/ }));
    expect(onJump).toHaveBeenCalledWith(60_000);
  });

  it("marks the topic being played", () => {
    usePlayer.setState({ src: "x", currentMs: 70_000 });
    render(<TopicOutline topics={topics(3)} onJump={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Outline (3)" }));
    expect(screen.getByRole("button", { name: /Topic 1/ }).getAttribute("aria-current")).toBe("true");
    expect(screen.getByRole("button", { name: /Topic 0/ }).getAttribute("aria-current")).toBeNull();
  });
});

describe("TopicRail", () => {
  it("lists every topic with its time", () => {
    render(<TopicRail topics={topics(2)} onJump={vi.fn()} />);
    expect(screen.getByTestId("topic-rail").textContent).toContain("01:00");
  });

  it("widens when its left edge is dragged left, and keeps the width", () => {
    render(<TopicRail topics={topics(2)} onJump={vi.fn()} />);
    const rail = screen.getByTestId("topic-rail");
    const edge = screen.getByRole("separator", { name: "Resize topics" });
    expect(rail.style.width).toBe(`${TOPIC_RAIL_WIDTH.default}px`);
    fireEvent.pointerDown(edge, { button: 0, clientX: 800, pointerId: 1 });
    fireEvent.pointerMove(edge, { clientX: 700, pointerId: 1 });
    expect(rail.style.width).toBe(`${TOPIC_RAIL_WIDTH.default + 100}px`);
    expect(usePrefs.getState().topicRailWidth).toBe(TOPIC_RAIL_WIDTH.default);
    fireEvent.pointerUp(edge, { pointerId: 1 });
    expect(usePrefs.getState().topicRailWidth).toBe(TOPIC_RAIL_WIDTH.default + 100);
  });

  it("stays within its limits, resizes from the keyboard and resets on double-click", () => {
    render(<TopicRail topics={topics(2)} onJump={vi.fn()} />);
    const edge = screen.getByRole("separator", { name: "Resize topics" });
    fireEvent.pointerDown(edge, { button: 0, clientX: 800, pointerId: 1 });
    fireEvent.pointerMove(edge, { clientX: 0, pointerId: 1 });
    fireEvent.pointerUp(edge, { pointerId: 1 });
    expect(usePrefs.getState().topicRailWidth).toBe(TOPIC_RAIL_WIDTH.max);
    fireEvent.keyDown(edge, { key: "End" });
    expect(usePrefs.getState().topicRailWidth).toBe(TOPIC_RAIL_WIDTH.min);
    fireEvent.keyDown(edge, { key: "ArrowLeft" });
    expect(edge.getAttribute("aria-valuenow")).toBe(String(TOPIC_RAIL_WIDTH.min + 16));
    fireEvent.doubleClick(edge);
    expect(usePrefs.getState().topicRailWidth).toBe(TOPIC_RAIL_WIDTH.default);
  });

  it("folds to a strip with only its expand button, and opens again", () => {
    render(<TopicRail topics={topics(2)} onJump={vi.fn()} />);
    const hide = screen.getByRole("button", { name: "Hide topics" });
    expect(hide.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(hide);
    expect(usePrefs.getState().topicRailCollapsed).toBe(true);
    expect(screen.getByTestId("topic-rail").style.width).toBe("");
    expect(screen.queryByText("Topic 1")).toBeNull();
    expect(screen.queryByRole("separator")).toBeNull();
    const show = screen.getByRole("button", { name: "Show topics" });
    expect(show).toBe(hide); // the same element, so focus stays on it
    expect(show.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(show);
    expect(usePrefs.getState().topicRailCollapsed).toBe(false);
    expect(screen.getByText("Topic 1")).toBeTruthy();
  });
});
