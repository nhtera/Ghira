// SPDX-License-Identifier: Apache-2.0
// The record control with its clock. The one-second tick lives here so the
// rest of the screen doesn't re-render with it.
import { useShallow } from "zustand/react/shallow";
import { RecordControl, type RecordState } from "@ghi/ui";
import type { RecordMode, SessionState } from "../../bindings";
import { useAppActions } from "../../shell/actions";
import { elapsedMs, useLive } from "../../state/live";
import { useUi } from "../../state/ui";
import { SensitiveStartToggle } from "../sensitive";
import { useNow } from "./clock";

function recordState(s: SessionState): RecordState {
  switch (s) {
    case "starting":
    case "recording":
    case "paused":
    case "stopping":
      return s;
    case "failed":
      return "error";
    default:
      return "idle";
  }
}

export function HeaderRecord() {
  const state = useLive((s) => s.state);
  const clock = useLive(useShallow((s) => ({ startedAtMs: s.startedAtMs, pausedAtMs: s.pausedAtMs, pausedTotalMs: s.pausedTotalMs })));
  const now = useNow(state === "recording");
  const mode: RecordMode = useUi((s) => s.recordMode);
  const setMode = useUi((s) => s.setRecordMode);
  const { startRecording, stopRecording, pauseRecording, resumeRecording } = useAppActions();
  const idle = recordState(state) === "idle";
  return (
    <div className="flex items-center gap-2">
      {idle && <SensitiveStartToggle />}
      <RecordControl
        state={recordState(state)}
        mode={mode}
        elapsedMs={elapsedMs(clock, now)}
        onStart={(m) => void startRecording(m)}
        onModeChange={setMode}
        onPause={() => void pauseRecording()}
        onResume={() => void resumeRecording()}
        onStop={() => void stopRecording()}
      />
    </div>
  );
}
