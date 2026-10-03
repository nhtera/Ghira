// SPDX-License-Identifier: Apache-2.0
// The app lock screen (Touch ID / Windows Hello; shell/lock-gate.tsx). Covers the
// whole window; nothing behind it is readable or focusable.
import { useTranslation } from "react-i18next";
import { APP_NAME } from "@ghi/i18n";
import { Button, usePlatform } from "@ghi/ui";

export function LockedScreen({ onUnlock, onPassword, error }: { onUnlock: () => void; onPassword?: () => void; error?: string | null }) {
  const { t } = useTranslation();
  const context = usePlatform();
  return (
    <div role="alertdialog" aria-modal="true" aria-labelledby="locked-title" data-banner="locked" className="fixed inset-0 z-50 grid place-items-center bg-surface2 p-8">
      <div className="flex max-w-[380px] flex-col items-center gap-3 text-center">
        <span aria-hidden className="grid size-14 place-items-center rounded-[14px] bg-accent font-serif text-[32px] font-semibold text-on-accent">
          {APP_NAME.charAt(0).toLowerCase()}
        </span>
        <h1 id="locked-title" className="m-0 text-[20px] font-semibold">
          {t("system.locked.title")}
        </h1>
        <p className="m-0 text-[13.5px] leading-relaxed text-muted">{t("system.locked.body")}</p>
        {error && (
          <p role="alert" className="m-0 text-[12.5px] text-warn">
            {error}
          </p>
        )}
        <Button variant="primary" size="lg" autoFocus icon={context === "win" ? "face" : "fingerprint"} className="mt-2.5 h-11 rounded-[10px] px-5 text-[14px]" onClick={onUnlock}>
          {t(`system.locked.unlock_${context}`)}
        </Button>
        {onPassword && (
          <Button variant="ghost" className="text-muted" onClick={onPassword}>
            {t("system.locked.usePassword")}
          </Button>
        )}
      </div>
    </div>
  );
}
