// SPDX-License-Identifier: Apache-2.0
// The strip under the live screen: where the data stays, the health pill and
// the level meters.
import { useTranslation } from "react-i18next";
import { Icon, usePlatform } from "@ghi/ui";
import { Health } from "./health";
import { Levels } from "./levels";

export function LiveFooter({ room }: { room: boolean }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  return (
    <footer className="relative flex h-11 flex-none items-center gap-3.5 border-t border-line px-5 text-[12px]">
      <span className="flex items-center gap-1.5 text-muted">
        <Icon name="lock" size={15} className="text-accent" />
        {t("live.localLine", { context: platform })}
      </span>
      <Health />
      <span className="flex-1" />
      <Levels room={room} />
    </footer>
  );
}
