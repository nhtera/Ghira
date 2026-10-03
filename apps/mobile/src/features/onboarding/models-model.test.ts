// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { MobileModelItem, MobileModelsStatus } from "../../bindings";
import { applyItem, modelsView } from "./models-model";

const item = (
  id: string,
  over: Partial<MobileModelItem> = {},
): MobileModelItem => ({
  id,
  role: "asr",
  sizeBytes: 100,
  receivedBytes: 0,
  state: "missing",
  ...over,
});
const status = (items: MobileModelItem[]): MobileModelsStatus => ({
  items,
  missingBytes: 200,
  wifiOnly: true,
});

describe("modelsView", () => {
  it("sums progress over all models", () => {
    const v = modelsView(
      status([
        item("a", { state: "ready", receivedBytes: 100 }),
        item("b", { state: "downloading", receivedBytes: 50 }),
      ]),
    );
    expect(v).toMatchObject({
      percent: 75,
      remainingBytes: 50,
      downloading: true,
      allReady: false,
    });
  });

  it("flags failed and waiting-for-Wi-Fi models", () => {
    expect(
      modelsView(status([item("a", { state: "failed" }), item("b")])).failed,
    ).toBe(true);
    expect(
      modelsView(status([item("a", { state: "waitingForWifi" })]))
        .waitingForWifi,
    ).toBe(true);
  });

  it("is ready only when every model is", () => {
    expect(
      modelsView(
        status([item("a", { state: "ready" }), item("b", { state: "ready" })]),
      ).allReady,
    ).toBe(true);
    expect(modelsView(status([])).allReady).toBe(false);
  });
});

describe("applyItem", () => {
  it("replaces one item and recomputes what is missing", () => {
    const next = applyItem(
      status([item("a"), item("b")]),
      item("a", { state: "ready", receivedBytes: 100 }),
    );
    expect(next.items.map((i) => i.state)).toEqual(["ready", "missing"]);
    expect(next.missingBytes).toBe(100);
  });

  it("ignores an unknown model", () => {
    const s = status([item("a")]);
    expect(applyItem(s, item("z"))).toBe(s);
  });
});
