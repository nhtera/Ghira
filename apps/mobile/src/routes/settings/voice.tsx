// SPDX-License-Identifier: Apache-2.0
// Settings → Voice profile: enroll "Me" (consent first, then read the passage)
// or delete the stored profile. The profile never leaves the phone.
import { ListRow, ListSection } from "@ghi/ui";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { useGo } from "../../features/settings/go";
import { unwrap, useAction, useResource } from "../../features/settings/api";
import { Btn, ErrorLine } from "../../features/settings/controls";
import { Page } from "../../features/settings/page";
import { EnrollError, EnrollMeter } from "../../features/voice/enroll-panel";
import { useVoiceEnroll } from "../../features/voice/use-voice-enroll";

const loadVoice = async () => unwrap(await ipc.commands.voiceStatus());

export function VoiceScreen() {
  const { t, i18n } = useTranslation();
  const go = useGo();
  const voice = useResource(loadVoice);
  const action = useAction();
  const [consent, setConsent] = useState(false);
  const [note, setNote] = useState<"saved" | "deleted" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const status = voice.data;
  // The hook ends an enrollment when the screen goes: the mic must not stay open behind another screen.
  const enroll = useVoiceEnroll(() => {
    setConsent(false);
    setNote("saved");
    voice.reload();
  });
  const phase = enroll.state.phase;
  const enrolling = phase !== "idle" && phase !== "done";

  const start = () =>
    void action.run(async () => {
      setNote(null);
      unwrap(await ipc.commands.voiceSetConsent(true));
      await enroll.start();
    });
  // Leaving without a saved profile takes the consent back (the hook ends the enrollment).
  const given = useRef(false);
  useEffect(() => {
    given.current = phase !== "idle" && phase !== "done";
  }, [phase]);
  useEffect(
    () => () => {
      if (given.current) void ipc.commands.voiceSetConsent(false);
    },
    [],
  );
  // A failed try ends the enrollment; the consent goes with it.
  const failed = enroll.state.error;
  useEffect(() => {
    if (failed) void ipc.commands.voiceSetConsent(false);
  }, [failed]);
  const cancel = () =>
    void action.run(async () => {
      enroll.cancel();
      unwrap(await ipc.commands.voiceEnrollCancel());
      await ipc.commands.voiceSetConsent(false);
      setConsent(false);
      voice.reload();
    });
  const remove = () =>
    void action.run(async () => {
      unwrap(await ipc.commands.voiceDeleteMe());
      setConfirmDelete(false);
      setNote("deleted");
      voice.reload();
    });

  return (
    <Page title={t("mobile.settings.voice.title")} back="settings" error={voice.error} onRetry={voice.reload}>
      {status && (
        <>
          <ErrorLine code={action.error} />
          {enroll.state.error && (
            <div className="mx-4 my-2">
              <EnrollError code={enroll.state.error} />
            </div>
          )}
          <ListSection header={t("mobile.settings.voice.headerMe")} footer={t("mobile.settings.voice.footer")}>
            <ListRow
              title={status.meProfile ? t("mobile.settings.voice.statusSet", { count: status.meProfile.samples }) : t("mobile.settings.voice.statusNone")}
              icon="record_voice_over"
            />
          </ListSection>
          {note && (
            <p role="status" className="text-ios-footnote mx-4 my-2 text-accent">
              {t(`mobile.settings.voice.${note}`)}
            </p>
          )}
          {!status.modelReady ? (
            <div className="mx-4 flex flex-col items-start gap-3">
              <p className="text-ios-subhead m-0 text-muted">{t("mobile.settings.voice.noModel")}</p>
              <Btn onClick={() => go("/settings/models")}>{t("mobile.settings.voice.openModels")}</Btn>
            </div>
          ) : enrolling ? (
            <div className="mx-4 flex flex-col gap-3">
              <p role="status" className="text-ios-subhead m-0 text-muted">{t("mobile.voice.listening")}</p>
              <blockquote lang={i18n.language} className="text-ios-body m-0 rounded-(--ios-radius-group) bg-surface p-4 select-text">
                {t("mobile.voice.passage")}
              </blockquote>
              <EnrollMeter state={enroll.state} />
              <Btn tone="primary" onClick={() => void enroll.finish()} disabled={!enroll.canFinish}>
                {t("mobile.settings.voice.finish")}
              </Btn>
              {phase !== "saving" && <Btn onClick={cancel}>{t("mobile.common.cancel")}</Btn>}
            </div>
          ) : (
            <div className="mx-4 flex flex-col gap-3">
              <p className="text-ios-subhead m-0 text-muted">{t("mobile.voice.body")}</p>
              <label className="text-ios-subhead flex min-h-ios-target items-start gap-3">
                <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} className="mt-0.5 size-[1.375rem] shrink-0 accent-[var(--accent)]" />
                <span>{t("mobile.voice.consent")}</span>
              </label>
              <Btn tone="primary" onClick={start} disabled={!consent || action.busy}>
                {status.meProfile ? t("mobile.settings.voice.redo") : t("mobile.settings.voice.enroll")}
              </Btn>
              {status.meProfile && !confirmDelete && (
                <Btn tone="danger" onClick={() => setConfirmDelete(true)} disabled={action.busy}>
                  {t("mobile.settings.voice.delete")}
                </Btn>
              )}
              {status.meProfile && confirmDelete && (
                <div role="alertdialog" aria-labelledby="voice-delete-q" className="flex flex-col gap-2 rounded-(--ios-radius-group) border-[1.5px] border-rec bg-rec-soft p-3">
                  <p id="voice-delete-q" className="text-ios-subhead m-0 font-semibold text-rec-ink">
                    {t("mobile.settings.voice.deleteConfirm")}
                  </p>
                  <Btn tone="danger" onClick={remove} disabled={action.busy}>
                    {t("mobile.settings.voice.deleteConfirmAction")}
                  </Btn>
                  <Btn autoFocus onClick={() => setConfirmDelete(false)}>
                    {t("mobile.common.cancel")}
                  </Btn>
                </div>
              )}
            </div>
          )}
        </>
      )}
    </Page>
  );
}
