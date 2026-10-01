// SPDX-License-Identifier: Apache-2.0
// What the keyboard, the macOS menu and the command palette can do. One place,
// so a chord, a menu item and a palette row always do the same thing.
import { useNavigate } from "@tanstack/react-router";
import { useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useToast } from "@ghi/ui";
import type { RecordMode } from "../bindings";
import { ipc } from "../ipc";
import { isActive, useLive } from "../state/live";
import { useUi } from "../state/ui";
import type { ShortcutId } from "./shortcuts";

export function useAppActions() {
  const navigate = useNavigate();
  const state = useLive((s) => s.state);
  const recordMode = useUi((s) => s.recordMode);
  const setPaletteOpen = useUi((s) => s.setPaletteOpen);

  const { show } = useToast();
  const { t } = useTranslation();
  /** A failed command says so (a denied mic, nothing recording, …). */
  const report = useCallback(
    async <R extends { status: "ok" } | { status: "error"; error: string } | undefined>(p: Promise<R> | R): Promise<R> => {
      const r = await p;
      if (r && r.status === "error") show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      return r;
    },
    [show, t],
  );
  const startRecording = useCallback(
    async (mode: RecordMode = recordMode) => {
      const r = await report(ipc.commands.startRecording(mode, null, ""));
      if (r?.status === "ok") void navigate({ to: "/live" });
      return r;
    },
    [navigate, recordMode, report],
  );
  const stopRecording = useCallback(() => report(ipc.commands.stopRecording()), [report]);
  const pauseRecording = useCallback(() => report(ipc.commands.pauseRecording()), [report]);
  const resumeRecording = useCallback(() => report(ipc.commands.resumeRecording()), [report]);
  // Mid-start or mid-stop a second press does nothing (no start-then-stop).
  const toggleRecording = useCallback(() => {
    if (state === "starting" || state === "stopping") return undefined;
    return isActive(state) ? stopRecording() : startRecording();
  }, [state, startRecording, stopRecording]);
  const mark = useCallback(() => (isActive(state) ? ipc.commands.markMoment() : undefined), [state]);

  /** Runs a shortcut; false when nothing handles it yet (the key keeps its default). */
  const run = useCallback(
    (id: ShortcutId): boolean => {
      switch (id) {
        case "toggleRecording":
          void toggleRecording(); // start/stop report their own errors
          return true;
        case "mark":
          void report(mark());
          return true;
        case "commandPalette":
          setPaletteOpen(true);
          return true;
        case "settings":
          void navigate({ to: "/settings/$section", params: { section: "general" } });
          return true;
        case "find":
        case "notesTab":
        case "transcriptTab":
        case "export":
          // The library search (phase 10) and meeting detail (phase 11) handle these.
          return false;
      }
    },
    [toggleRecording, mark, setPaletteOpen, navigate, report],
  );

  return useMemo(
    () => ({ startRecording, stopRecording, pauseRecording, resumeRecording, toggleRecording, mark, run }),
    [startRecording, stopRecording, pauseRecording, resumeRecording, toggleRecording, mark, run],
  );
}
