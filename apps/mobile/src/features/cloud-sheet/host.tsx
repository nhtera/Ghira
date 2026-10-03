// SPDX-License-Identifier: Apache-2.0
// Opens the cloud send sheet from anywhere: `openCloudSheet(meetingId)` (the
// meeting view's "Improve with cloud"). <CloudSheetHost /> sits above the
// routes (shell/root-view.tsx via <GlobalOverlays />) and shows the sheet.
import { useEffect, useState } from "react";
import { LOCKED_EVENT } from "../app-lock/events";
import { CloudSheet, type CloudSheetProps } from "./cloud-sheet";

const OPEN_EVENT = "ghi:open-cloud-sheet";

type Opening = Pick<
  CloudSheetProps,
  "meetingId" | "task" | "cloudLocked" | "onSent"
>;

export function openCloudSheet(opening: Opening | string): void {
  const detail: Opening =
    typeof opening === "string" ? { meetingId: opening } : opening;
  window.dispatchEvent(new CustomEvent<Opening>(OPEN_EVENT, { detail }));
}

export function CloudSheetHost() {
  const [opening, setOpening] = useState<Opening | null>(null);
  useEffect(() => {
    const onOpen = (e: Event) => setOpening((e as CustomEvent<Opening>).detail);
    const onLocked = () => setOpening(null);
    window.addEventListener(OPEN_EVENT, onOpen);
    window.addEventListener(LOCKED_EVENT, onLocked);
    return () => {
      window.removeEventListener(OPEN_EVENT, onOpen);
      window.removeEventListener(LOCKED_EVENT, onLocked);
    };
  }, []);
  if (!opening) return null;
  return (
    <CloudSheet
      {...opening}
      open
      onOpenChange={(o) => !o && setOpening(null)}
    />
  );
}
