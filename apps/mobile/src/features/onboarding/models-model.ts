// SPDX-License-Identifier: Apache-2.0
// The model download as the models step shows it: the status plus
// `modelDownload` events folded into a few flags.
import type { MobileModelItem, MobileModelsStatus } from "../../bindings";

/** Replaces the item with the same id (an event for an unknown id is ignored). */
export function applyItem(
  status: MobileModelsStatus,
  item: MobileModelItem,
): MobileModelsStatus {
  if (!status.items.some((i) => i.id === item.id)) return status;
  const items = status.items.map((i) => (i.id === item.id ? item : i));
  return { ...status, items, missingBytes: missingBytes(items) };
}

function missingBytes(items: MobileModelItem[]): number {
  return items
    .filter((i) => i.state !== "ready")
    .reduce(
      (sum, i) =>
        sum + Math.max(0, (i.sizeBytes ?? 0) - (i.receivedBytes ?? 0)),
      0,
    );
}

export type ModelsView = {
  allReady: boolean;
  downloading: boolean;
  failed: boolean;
  waitingForWifi: boolean;
  /** 0..100 over all bytes to download. */
  percent: number;
  /** Bytes still to download. */
  remainingBytes: number;
};

export function modelsView(status: MobileModelsStatus): ModelsView {
  const items = status.items;
  const total = items.reduce((sum, i) => sum + (i.sizeBytes ?? 0), 0);
  const received = items.reduce(
    (sum, i) =>
      sum +
      (i.state === "ready"
        ? (i.sizeBytes ?? 0)
        : Math.min(i.receivedBytes ?? 0, i.sizeBytes ?? 0)),
    0,
  );
  return {
    allReady: items.length > 0 && items.every((i) => i.state === "ready"),
    downloading: items.some((i) => i.state === "downloading"),
    failed: items.some((i) => i.state === "failed"),
    waitingForWifi: items.some((i) => i.state === "waitingForWifi"),
    percent: total > 0 ? Math.round((received / total) * 100) : 0,
    remainingBytes: Math.max(0, total - received),
  };
}
