// SPDX-License-Identifier: Apache-2.0
// D2: the native detection panel (340x150, top right). The core builds it once
// and reuses it: the first detection arrives in the route
// (`#/detect?app=zoom&name=Zoom&browser=0`), later ones as `meetingDetected`
// events sent to this window. Start records here, shows the mini recorder and
// closes the panel; Esc or no answer for ~30 s counts as "Not now".
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DetectReply, MeetingDetected } from "../../bindings";
import { ipc } from "../../ipc";
import { useLock } from "../../state/lock";
import { usePanelWindow } from "../popover/panel-window";
import { DetectionCard } from "./detection-prompt";

const AUTO_CLOSE_MS = 30_000;

export function detectedFromHash(hash: string): MeetingDetected | null {
  const q = new URLSearchParams(hash.split("?")[1] ?? "");
  const app = q.get("app");
  if (!app) return null;
  return {
    app,
    appName: q.get("name") || app,
    browser: q.get("browser") === "1",
    // A calendar meeting: its title and key.
    title: q.get("title") ?? undefined,
    event: q.get("event") ?? undefined,
  };
}

/** Outside Tauri (the browser, the gallery) the panel shows a sample prompt, as the popover shows sample data. */
const sampleDetection = (): MeetingDetected | null => (ipc.kind === "mock" ? { app: "zoom", appName: "Zoom", browser: false } : null);

export function DetectPanel({
  initial = detectedFromHash(window.location.hash) ?? sampleDetection(),
}: {
  initial?: MeetingDetected | null;
}) {
  const { t } = useTranslation();
  usePanelWindow();
  const [detected, setDetected] = useState<MeetingDetected | null>(initial);
  const [error, setError] = useState<string | null>(null);
  const locked = useLock((st) => st.locked === true);

  // Locking forgets the prompt (so the panel closes below) and leaves no event
  // title in its address.
  if (locked && detected) setDetected(null);
  useEffect(() => {
    if (locked) window.history.replaceState(null, "", "#/detect");
  }, [locked]);

  // The next detection (the panel is reused, not rebuilt).
  useEffect(() => {
    let off: (() => void) | undefined;
    let gone = false;
    void ipc
      .onMeetingDetected((d) => {
        setError(null);
        setDetected(d);
      })
      .then((u) => (gone ? u() : (off = u)));
    return () => {
      gone = true;
      off?.();
    };
  }, []);

  const reply = async (d: MeetingDetected, r: DetectReply) => {
    if (r === "start") {
      const started = await ipc.commands.startRecording("call", null, "");
      if (started.status === "error") return setError(started.error);
    }
    const res = await ipc.commands.replyMeetingDetected(d.app, r);
    if (res.status === "error") setError(res.error);
    if (r === "start") await ipc.commands.openMiniRecorder();
    void ipc.commands.closeDetect();
  };

  // One timer per prompt; Esc answers "Not now".
  useEffect(() => {
    if (!detected) return void ipc.commands.closeDetect();
    const notNow = () => void reply(detected, "notNow");
    const id = window.setTimeout(notNow, AUTO_CLOSE_MS);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && notNow();
    window.addEventListener("keydown", onKey);
    return () => {
      window.clearTimeout(id);
      window.removeEventListener("keydown", onKey);
    };
  }, [detected]);

  if (!detected) return null;
  return (
    <main className="box-border flex h-screen flex-col bg-surface">
      <DetectionCard
        detected={detected}
        onReply={(r) => void reply(detected, r)}
        className="flex-1 rounded-none border-0"
      />
      {error && (
        <p role="alert" className="text-small m-0 px-4 pb-3 text-rec-ink">
          {t("system.commandFailed", { message: error })}
        </p>
      )}
    </main>
  );
}
