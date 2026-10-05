// SPDX-License-Identifier: Apache-2.0
// "Final pass on my computer" as a processing target: pickable once a
// computer is paired, otherwise disabled with "Pair a computer first".
import { useTranslation } from "react-i18next";
import type { ProcessingTarget } from "../../bindings";
import { useGo } from "../settings/go";
import { useSync, useSyncAvailable } from "./use-sync";

export function useDesktopTarget() {
  const available = useSyncAvailable();
  const sync = useSync();
  const paired = sync.device !== null;
  return {
    /** Pairing is offered at all. */
    available,
    /** The status has been read (until then `paired` is not known). */
    ready: sync.status !== undefined,
    paired,
    device: sync.device,
    /** What the picker cannot offer: cloud is chosen per meeting; the computer needs a pairing. */
    disabled: (recordOnly = false): ProcessingTarget[] => (recordOnly ? ["phone", "desktop", "cloud"] : paired ? ["cloud"] : ["desktop", "cloud"]),
  };
}

/** The note under a picker whose computer option is off because nothing is paired. */
export function NeedPair({ link = true, className }: { link?: boolean; className?: string }) {
  const { t } = useTranslation();
  const go = useGo();
  return (
    <p data-testid="need-pair" className={className ?? "text-ios-footnote m-0 text-muted"}>
      {t("mobile.settings.desktopLater")}
      {link && (
        <>
          {" · "}
          <button type="button" onClick={() => go("/settings/sync")} className="text-ios-footnote -my-3 min-h-ios-target px-1 align-baseline font-semibold text-accent underline underline-offset-2">
            {t("mobile.sync.pairNow")}
          </button>
        </>
      )}
    </p>
  );
}
