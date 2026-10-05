// SPDX-License-Identifier: Apache-2.0
// M1 step 5: where new recordings are processed by default. This phone; My
// computer once one is paired (the step before); cloud is chosen per meeting.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { MobileSettings, ProcessingTarget } from "../../bindings";
import { ipc } from "../../ipc";
import { PhoneButton } from "@ghi/ui";
import { TargetPicker } from "../record/target-picker";
import { NeedPair, useDesktopTarget } from "../sync";
import { StepLayout } from "./step-layout";

export function ProcessingStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<MobileSettings | null>(null);
  const [target, setTarget] = useState<ProcessingTarget>("phone");
  const [recordOnly, setRecordOnly] = useState(false);
  const desktop = useDesktopTarget();

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [s, tier] = await Promise.all([
        ipc.commands.mobileSettings(),
        ipc.commands.deviceTier(),
      ]);
      if (!alive) return;
      if (s.status === "ok") {
        setSettings(s.data);
        setTarget(s.data.defaultTarget);
      }
      if (tier.status === "ok") setRecordOnly(tier.data.tier === "recordOnly");
    })();
    return () => {
      alive = false;
    };
  }, []);

  const next = async () => {
    if (settings)
      await ipc.commands.setMobileSettings({
        ...settings,
        defaultTarget: target,
      });
    onNext();
  };

  return (
    <StepLayout
      icon="hard_drive"
      title={t("mobile.onboarding.processing.title")}
      subtitle={t("mobile.onboarding.processing.footer")}
      footer={
        <PhoneButton onClick={() => void next()}>
          {t("mobile.common.continue")}
        </PhoneButton>
      }
    >
      <TargetPicker
        value={target}
        onChange={setTarget}
        disabled={desktop.disabled(recordOnly)}
      />
      {desktop.available && !desktop.paired && !recordOnly && (
        <NeedPair link={false} className="text-ios-footnote m-0 mt-2 text-muted" />
      )}
    </StepLayout>
  );
}
