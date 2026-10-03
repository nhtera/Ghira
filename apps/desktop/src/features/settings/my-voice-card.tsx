// SPDX-License-Identifier: Apache-2.0
// Settings → Privacy → Your voice: the real state of the Me profile, record
// it again, delete it. Other people's voices stay off in this build.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { APP_NAME, formatDate, type Locale } from "@ghi/i18n";
import { Button, ConfirmArea, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { EnrollDialog } from "../people/enroll-dialog";
import { errorText } from "../people/error-text";
import { useInvalidatePeople, usePeople, useVoiceStatus } from "../people/queries";
import { Card } from "./parts";

export function MyVoiceCard() {
  const { t, i18n } = useTranslation();
  const locale: Locale = i18n.language === "vi" ? "vi" : "en";
  const { show } = useToast();
  const voice = useVoiceStatus();
  const people = usePeople();
  const invalidate = useInvalidatePeople();
  const [enrolling, setEnrolling] = useState(false);
  if (voice.isError)
    return (
      <Card title={t("settings.privacy.myVoice.title")}>
        <p role="alert" className="text-small m-0 text-rec-ink">
          {errorText(t, voice.error.message)}
        </p>
      </Card>
    );
  if (!voice.data) return null;
  const { modelReady, meProfile } = voice.data;
  const me = people.data?.people.find((p) => p.isMe);

  const remove = async () => {
    if (!me) return;
    const r = await ipc.commands.deleteVoiceData(me.gid);
    if (r.status === "error") return show({ tone: "warning", title: errorText(t, r.error) });
    show({ tone: "success", title: t("settings.privacy.myVoice.deleted") });
    invalidate();
  };

  return (
    <Card
      title={t("settings.privacy.myVoice.title")}
      hint={
        meProfile
          ? t("settings.privacy.myVoice.saved", {
              count: meProfile.samples,
              date: meProfile.atMs != null ? formatDate(meProfile.atMs, locale) : "",
            })
          : t("settings.privacy.myVoice.none", { app: APP_NAME })
      }
    >
      {!modelReady && <p className="text-small m-0 text-muted">{t("settings.privacy.myVoice.modelMissing")}</p>}
      <div className="flex flex-wrap gap-2">
        <Button icon="mic" disabled={!modelReady} onClick={() => setEnrolling(true)}>
          {t("people.reRecordVoice")}
        </Button>
        {meProfile && me && (
          <ConfirmArea
            icon="delete_forever"
            question={t("settings.privacy.myVoice.question", {
              count: meProfile.samples,
              app: APP_NAME,
            })}
            confirmLabel={t("people.deleteVoice.confirm")}
            onConfirm={() => void remove()}
            trigger={({ onClick, ref }) => (
              <Button ref={ref} icon="delete" className="border-rec text-rec" onClick={onClick}>
                {t("settings.privacy.myVoice.delete")}
              </Button>
            )}
          />
        )}
      </div>
      <EnrollDialog open={enrolling} onClose={() => setEnrolling(false)} />
    </Card>
  );
}
