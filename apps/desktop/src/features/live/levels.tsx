// SPDX-License-Identifier: Apache-2.0
// Level meters on their own: levels arrive about 10 times a second, so only
// this component subscribes to them (not the whole screen).
import { LevelMeter } from "@ghi/ui";
import { useLive } from "../../state/live";

export function Levels() {
  const mic = useLive((s) => s.levels.mic);
  const system = useLive((s) => s.levels.system);
  return (
    <div className="grid w-full max-w-md grid-cols-2 gap-3">
      <LevelMeter source="mic" db={mic} />
      <LevelMeter source="system" db={system} />
    </div>
  );
}
