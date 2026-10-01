// SPDX-License-Identifier: Apache-2.0
// App-level system states (D12) as non-modal banners above the routed screen.
// Mount once in the shell. The update banner and the locked screen are
// separate components: the updater and the app lock are phase 12.
import { CoreErrorBanners } from "./core-errors";
import { ModelDamagedBanner } from "./model-damaged-banner";
import { RecoveredBanners } from "./recovered-banners";

export function SystemStates() {
  return (
    <div data-testid="system-states" className="flex flex-col gap-2 empty:hidden [&:has(>*)]:px-4 [&:has(>*)]:pt-3">
      <RecoveredBanners />
      <ModelDamagedBanner />
      <CoreErrorBanners />
    </div>
  );
}
