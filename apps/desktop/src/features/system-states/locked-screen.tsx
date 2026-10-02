// SPDX-License-Identifier: Apache-2.0
// The app lock screen (Touch ID / Windows Hello; shell/lock-gate.tsx). Covers the
// whole window; nothing behind it is readable or focusable.
import { useTranslation } from "react-i18next";
import { Button, Icon, usePlatform } from "@ghi/ui";

export function LockedScreen({ onUnlock, onPassword, error }: { onUnlock: () => void; onPassword?: () => void; error?: string | null }) {
  const { t } = useTranslation();
  const context = usePlatform();
  return (
    <div role="alertdialog" aria-modal="true" aria-labelledby="locked-title" data-banner="locked" className="fixed inset-0 z-50 grid place-items-center bg-surface p-8">
      <div className="flex max-w-[380px] flex-col items-center gap-3 text-center">
        <Icon name="lock" size={44} className="text-accent" />
        <h1 id="locked-title" className="text-title m-0">
          {t("system.locked.title")}
        </h1>
        <p className="m-0 text-[14px] leading-relaxed text-muted">{t("system.locked.body")}</p>
        {error && (
          <p role="alert" className="m-0 text-[12.5px] text-warn">
            {error}
          </p>
        )}
        <Button variant="primary" size="lg" autoFocus onClick={onUnlock}>
          {t(`system.locked.unlock_${context}`)}
        </Button>
        {onPassword && (
          <Button variant="ghost" onClick={onPassword}>
            {t("system.locked.usePassword")}
          </Button>
        )}
      </div>
    </div>
  );
}
