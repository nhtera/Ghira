// SPDX-License-Identifier: Apache-2.0
import { useState } from "react";
import type { Story, StoryMeta } from "../../story";
import { RecordControl, type RecordMode, type RecordState } from "./record-control";

export default { title: "Record control", width: 420 } satisfies StoryMeta;

const MS = 12 * 60_000 + 4_000; // 12:04

export const Idle: Story = { render: () => <RecordControl state="idle" mode="call" /> };
export const IdleRoom: Story = { render: () => <RecordControl state="idle" mode="room" />, note: "Room mode chosen from the caret menu." };
export const StartingPermissionCheck: Story = { render: () => <RecordControl state="starting" mode="call" /> };
export const Recording: Story = { render: () => <RecordControl state="recording" mode="call" elapsedMs={MS} /> };
export const Paused: Story = { render: () => <RecordControl state="paused" mode="call" elapsedMs={MS} /> };
export const Stopping: Story = { render: () => <RecordControl state="stopping" mode="call" /> };
export const Error: Story = { render: () => <RecordControl state="error" mode="call" onFix={() => {}} /> };

function Flow() {
  const [state, setState] = useState<RecordState>("idle");
  const [mode, setMode] = useState<RecordMode>("call");
  return (
    <RecordControl
      state={state}
      mode={mode}
      elapsedMs={MS}
      onModeChange={setMode}
      onStart={() => setState("recording")}
      onPause={() => setState("paused")}
      onResume={() => setState("recording")}
      onStop={() => setState("idle")}
    />
  );
}
export const Interactive: Story = { render: () => <Flow />, note: "Start, pause, resume, stop; the circle morphs into a rounded square in 200 ms." };
