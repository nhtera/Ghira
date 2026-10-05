// SPDX-License-Identifier: Apache-2.0
// M1 step 4 (only when pairing is available): pair with a computer by scanning
// the code its Settings → Sync shows. Optional: "Not now" moves on, and
// Settings → Sync with computer pairs later. Same Wi-Fi, nothing through the
// internet, said before the camera opens.
import { Icon, PhoneButton } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { PairScanPanel } from "../sync/pair-scan";
import { ScrollFocus } from "../sync/scroll-focus";
import { usePairScan } from "../sync/use-pair-scan";
import { StepLayout } from "./step-layout";

export function PairStep({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation();
  const scan = usePairScan(true);
  const paired = scan.phase === "paired";
  return (
    <StepLayout
      title={t("mobile.pair.title")}
      subtitle={t("mobile.pair.body")}
      footer={
        paired ? (
          <PhoneButton onClick={onNext}>{t("mobile.common.continue")}</PhoneButton>
        ) : (
          <PhoneButton variant="ghost" onClick={onNext}>
            {t("mobile.common.notNow")}
          </PhoneButton>
        )
      }
    >
      <ScrollFocus className="flex flex-col gap-4">
        <PairScanPanel scan={scan} />
        <p className="text-ios-footnote m-0 flex items-start gap-2.5 text-muted">
          <Icon name="wifi" size={19} className="mt-0.5 size-[1.1875rem] shrink-0 text-accent" />
          {t("mobile.pair.wifiNote")}
        </p>
      </ScrollFocus>
    </StepLayout>
  );
}
