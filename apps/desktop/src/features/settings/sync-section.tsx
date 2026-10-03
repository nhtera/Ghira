// SPDX-License-Identifier: Apache-2.0
// Settings → Sync: the design's section with every control disabled until
// phone sync (phase 15) is built.
import { useTranslation } from "react-i18next";
import { Note, Row, Switch } from "./parts";

export function SyncSection() {
  const { t } = useTranslation();
  const toggles = [
    { key: "phoneRecordings", on: false },
    { key: "notes", on: false },
    { key: "voiceProfiles", on: false, hint: true },
    { key: "localOnly", on: false },
  ] as const;
  return (
    <div className="flex flex-col gap-5">
      <Note icon="schedule">{t("settings.sync.laterNote")}</Note>
      <section className="flex items-start gap-5 border-b border-line pb-5" aria-disabled>
        <div
          aria-hidden
          className="grid size-[104px] shrink-0 place-items-center rounded-xl border border-line2 bg-[repeating-linear-gradient(135deg,var(--color-surface2)_0_8px,var(--color-sunk)_8px_16px)]"
        />
        <div className="min-w-0">
          <h3 className="m-0 text-[14px] font-semibold">{t("settings.sync.pairTitle")}</h3>
        </div>
      </section>
      <section className="flex flex-col">
        <h3 className="m-0 text-[13px] font-semibold">{t("settings.sync.whatSyncs")}</h3>
        {toggles.map((x) => (
          <Row key={x.key} label={t(`settings.sync.${x.key}`)} hint={"hint" in x ? t("settings.sync.voiceProfilesHint") : undefined}>
            <Switch checked={x.on} onChange={() => {}} label={t(`settings.sync.${x.key}`)} disabled />
          </Row>
        ))}
      </section>
    </div>
  );
}
