// SPDX-License-Identifier: Apache-2.0
// App-level banner for the two urgent update states. Mount once in the shell:
// `<UpdateBanner />` above the page. Renders nothing otherwise.
import { useTranslation } from "react-i18next";
import { Button, Icon } from "@ghi/ui";
import { ipc } from "../../ipc";
import { useFail } from "./parts";
import { useUpdateStatus } from "./use-update-status";

/** `onOpenAbout` defaults to the hash route (the app uses hash history). */
export function UpdateBanner({ onOpenAbout = () => void (window.location.hash = "#/settings/about") }: { onOpenAbout?: () => void }) {
  const { t } = useTranslation();
  const status = useUpdateStatus();
  const fail = useFail();
  if (!status || (!status.runningPulled && !status.reinstallNeeded)) return null;

  const ready = status.ready && !!status.available;
  const text = status.reinstallNeeded ? t("settings.updates.banner.reinstall") : ready ? t("settings.updates.banner.pulledReady") : t("settings.updates.banner.pulledNotReady");
  const action = status.reinstallNeeded ? null : ready ? (
    <Button
      size="sm"
      variant="primary"
      onClick={async () => {
        const r = await ipc.commands.installUpdate();
        if (r.status === "error") fail(r.error);
      }}
    >
      {t("settings.updates.banner.restart")}
    </Button>
  ) : (
    <Button size="sm" onClick={onOpenAbout}>
      {t("settings.updates.banner.open")}
    </Button>
  );
  return (
    <div role="alert" data-testid="update-banner" className="flex items-center gap-3 border-b border-line bg-warn-soft px-4 py-2">
      <Icon name="warning" size={18} className="text-warn" />
      <span className="text-small flex-1">{text}</span>
      {action}
    </div>
  );
}
