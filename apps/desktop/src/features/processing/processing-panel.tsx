// SPDX-License-Identifier: Apache-2.0
// D5: the in-place stepper for a meeting that is being processed.
import { useTranslation } from "react-i18next";
import { ProcessingStepper, StatusPill, usePlatform } from "@ghi/ui";
import type { Processing } from "./processing-store";
import { stepsFor } from "./stepper-steps";

export function ProcessingPanel({ title, processing, waitingForModels }: { title?: string; processing: Processing; waitingForModels?: boolean }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  return (
    <section aria-label={t("processing.title", { context: platform })} className="mb-[18px] flex max-w-[740px] flex-col gap-2.5 rounded-panel bg-surface2 px-[18px] py-4">
      <div>
        <h2 className="text-heading m-0 text-[14px]">{t("processing.title", { context: platform })}</h2>
        {title && <p className="text-small m-0 mt-0.5 truncate text-ink">{title}</p>}
        <p className="m-0 text-[12.5px] text-muted">{t("processing.subtitle")}</p>
      </div>
      {waitingForModels ? <StatusPill status="waitingModels" className="self-start" /> : <ProcessingStepper steps={stepsFor(processing)} />}
    </section>
  );
}
