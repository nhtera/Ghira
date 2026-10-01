// SPDX-License-Identifier: Apache-2.0
// The 40 px title bar: a drag region with room for the mac traffic lights
// (the window uses an overlay title bar) and the privacy indicator, which is
// always visible (brief §3). Windows keeps its native caption in phase 9.
import { APP_NAME } from "@ghi/i18n";
import { PrivacyIndicator, usePlatform } from "@ghi/ui";
import { useLive } from "../state/live";

export function TitleBar() {
  const mac = usePlatform() === "mac";
  const state = useLive((s) => s.state);
  // Cloud states arrive with the cloud send flow (phase 11).
  const privacy = state === "paused" ? "paused" : state === "recording" || state === "starting" || state === "stopping" ? "recording" : "local";
  return (
    <div data-tauri-drag-region className="flex h-10 flex-none items-center gap-3 border-b border-line bg-surface2 pr-3.5" style={{ paddingLeft: mac ? 84 : 14 }}>
      <span data-tauri-drag-region className="flex-1 text-[13px] font-semibold text-muted">
        {APP_NAME}
      </span>
      <PrivacyIndicator state={privacy} />
    </div>
  );
}
