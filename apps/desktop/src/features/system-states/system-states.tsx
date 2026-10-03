// SPDX-License-Identifier: Apache-2.0
// App-level system states (D12) as full-width banners under the title bar.
// Mount once in the shell. The update banner and the locked screen are
// separate components: the updater and the app lock are phase 12.
import { CrashReportBanner } from "./crash-report-banner";
import { CoreErrorBanners } from "./core-errors";
import { ModelDamagedBanner } from "./model-damaged-banner";
import { RecoveredDialog } from "./recovered-dialog";

export function SystemStates() {
  return (
    <div data-testid="system-states" className="flex flex-col empty:hidden">
      <CrashReportBanner />
      <RecoveredDialog />
      <ModelDamagedBanner />
      <CoreErrorBanners />
    </div>
  );
}
