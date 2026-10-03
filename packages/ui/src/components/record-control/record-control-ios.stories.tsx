// SPDX-License-Identifier: Apache-2.0
import type { Story, StoryMeta } from "../../story";
import { RecordControl, type RecordState } from "./record-control";

export default { title: "Record control (iOS)", platform: "ios" } satisfies StoryMeta;

const at = (state: RecordState): Story => ({
  render: () => (
    <div className="flex justify-center py-2">
      <RecordControl state={state} mode="room" elapsedMs={754_000} onStart={() => {}} onStop={() => {}} onPause={() => {}} onResume={() => {}} onFix={() => {}} />
    </div>
  ),
});

export const Idle = at("idle");
export const Starting = at("starting");
export const Recording = at("recording");
export const Paused = at("paused");
export const Stopping = at("stopping");
export const ErrorState = at("error");
