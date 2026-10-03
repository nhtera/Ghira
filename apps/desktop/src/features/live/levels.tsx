// SPDX-License-Identifier: Apache-2.0
// Level meters on their own: levels arrive about 10 times a second, so only
// this component subscribes to them (not the whole screen). Room mode has no
// system track, so only the mic shows.
import { LevelMeter } from "@ghi/ui";
import { useLive } from "../../state/live";

export function Levels({ room }: { room?: boolean }) {
  const mic = useLive((s) => s.levels.mic);
  const system = useLive((s) => s.levels.system);
  return (
    <div className="flex items-center gap-3.5">
      <LevelMeter source="mic" db={mic} compact />
      {!room && <LevelMeter source="system" db={system} compact />}
    </div>
  );
}
