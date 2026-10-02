// SPDX-License-Identifier: Apache-2.0
// One person: meetings together, open action items, voice samples with the
// voice actions, and the name in notes. The two destructive actions are
// separate sections with separate confirms (design rationale #1).
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { APP_NAME, formatClock, formatDate, type Locale } from "@ghi/i18n";
import { useQueryClient } from "@tanstack/react-query";
import { Avatar, Button, ConfirmArea, Icon, InlineConfirm, Menu, useToast } from "@ghi/ui";
import type { PersonRow, VoiceSample } from "../../bindings";
import { ipc } from "../../ipc";
import { EnrollDialog } from "./enroll-dialog";
import { errorText } from "./error-text";
import { personName } from "./person-label";
import { personKey, useInvalidatePeople, usePersonDetail, useVoiceStatus } from "./queries";
import { VoiceBadge } from "./voice-badge";

function Samples({ samples, name }: { samples: VoiceSample[]; name: string }) {
  const { t } = useTranslation();
  // `n` counts plays, so playing the same sample again restarts it.
  const [playing, setPlaying] = useState<{ key: string; src: string; n: number } | null>(null);
  const [plays, setPlays] = useState(0);
  const play = async (s: VoiceSample, key: string) => {
    const r = await ipc.commands.issueAudioSample(s.meetingGid, s.t0Ms, s.t1Ms, s.track);
    // No audio left (retention, the mock core): the span stays listed.
    if (r.status === "ok") {
      setPlays((n) => n + 1);
      setPlaying({ key, src: ipc.audioUrl(r.data), n: plays + 1 });
    }
  };
  return (
    <>
      <span className="text-small text-muted">{t("people.samples.subtitle", { app: APP_NAME, name })}</span>
      <ul className="m-0 list-none rounded-row border border-line p-0">
        {samples.map((s, i) => {
          const key = `${s.meetingGid}:${s.t0Ms}:${i}`;
          const on = playing?.key === key;
          return (
            <li key={key} className="flex items-center gap-2.5 border-t border-line px-2.5 py-2 first:border-t-0">
              <Button
                size="sm"
                icon={on ? "stop" : "play_arrow"}
                aria-label={on ? t("people.stopSample") : t("speakers.playSample")}
                onClick={() => (on ? setPlaying(null) : void play(s, key))}
              />
              <span className="min-w-0 flex-1 truncate text-[13px]">{s.meetingTitle}</span>
              {s.t0Ms != null && <span className="text-mono text-muted">{formatClock(s.t0Ms)}</span>}
            </li>
          );
        })}
      </ul>
      {playing && <audio key={playing.n} src={playing.src} autoPlay onEnded={() => setPlaying(null)} aria-label={t("speakers.playing")} />}
    </>
  );
}

export function PersonDetailView({ gid, people, thirdParty, onSelect }: { gid: string; people: PersonRow[]; thirdParty: boolean; onSelect: (gid: string | null) => void }) {
  const { t, i18n } = useTranslation();
  const locale: Locale = i18n.language === "vi" ? "vi" : "en";
  const { show } = useToast();
  const navigate = useNavigate();
  const client = useQueryClient();
  const invalidate = useInvalidatePeople();
  const detail = usePersonDetail(gid);
  const voice = useVoiceStatus();
  const [enrolling, setEnrolling] = useState(false);
  const [mergeInto, setMergeInto] = useState<PersonRow | null>(null);

  if (detail.isError) return <p className="text-body text-muted">{errorText(t, detail.error.message)}</p>;
  if (!detail.data) return null;
  const { person, meetings, openActions, samples } = detail.data;
  const name = personName(person, t);
  const hasVoice = person.voice.kind !== "none";
  const sampleCount = person.isMe ? (voice.data?.meProfile?.samples ?? samples.length) : samples.length;

  const fail = (error: string) => show({ tone: "warning", title: errorText(t, error) });
  const openMeeting = (id: string) => void navigate({ to: "/meetings/$id/$tab", params: { id, tab: "notes" } });

  const merge = async (into: PersonRow) => {
    setMergeInto(null);
    const r = await ipc.commands.mergePeople(gid, into.gid);
    if (r.status === "error") return fail(r.error);
    show({ tone: "success", title: t("people.merge.done", { from: name, into: personName(into, t) }) });
    invalidate();
    onSelect(into.gid);
  };
  const deleteVoice = async () => {
    const r = await ipc.commands.deleteVoiceData(gid);
    if (r.status === "error") return fail(r.error);
    show({ tone: "success", title: t("people.deleteVoice.done", { name }) });
    invalidate();
  };
  const removeName = async () => {
    const r = await ipc.commands.removePersonName(gid);
    if (r.status === "error") return fail(r.error);
    show({ tone: "success", title: t("people.removeName.done", { count: r.data }) });
    // Without a voice profile the person is gone: don't ask for them again, show the first row.
    if (!hasVoice) {
      client.removeQueries({ queryKey: personKey(gid) });
      onSelect(null);
    }
    invalidate();
  };

  const mergeTargets = people.filter((p) => !p.isMe && p.gid !== gid);
  // A voice profile can't be merged without other people's voice profiles being on (the core refuses with thirdPartyOff).
  const hasProfile = (p: PersonRow) => p.voice.kind !== "none" && !thirdParty;
  const mergeBlocked = hasProfile(person) || mergeTargets.some(hasProfile);

  return (
    <div className="flex max-w-[680px] flex-col gap-6">
      <div className="flex items-center gap-3.5">
        <Avatar kind={person.isMe ? "me" : "person"} name={name} colorSlot={person.colorSlot} size="xl" className="size-14 text-[20px]" />
        <div className="min-w-0 flex-1">
          <h2 className="text-title m-0 truncate">{name}</h2>
          <p className="text-small m-0 text-muted">
            {t("people.meetingsCount", { count: person.meetings })}
            {person.lastMetMs != null && ` · ${t("people.lastMet", { date: formatDate(person.lastMetMs, locale) })}`}
          </p>
        </div>
        {!person.isMe && mergeTargets.length > 0 && (
          <Menu
            label={t("people.merge.action")}
            trigger={
              <Button icon="call_merge" disabled={hasProfile(person)} aria-label={t("people.merge.action")}>
                {t("people.merge.action")}
              </Button>
            }
            items={mergeTargets.map((p) => ({ label: personName(p, t), disabled: hasProfile(p), onSelect: () => setMergeInto(p) }))}
          />
        )}
      </div>
      {!person.isMe && (
        <div className="-mt-4 flex flex-col gap-1.5">
          <p className="text-small m-0 text-muted">{t("people.merge.hint")}</p>
          {mergeBlocked && <p className="text-small m-0 text-muted">{t("people.merge.blocked")}</p>}
          {mergeInto && (
            <InlineConfirm
              icon="call_merge"
              question={t("people.merge.question", { from: name, into: personName(mergeInto, t) })}
              confirmLabel={t("people.merge.confirm")}
              onConfirm={() => void merge(mergeInto)}
              onCancel={() => setMergeInto(null)}
            />
          )}
        </div>
      )}

      <section aria-label={t("people.samples.title")} className="flex flex-col gap-2.5">
        <div className="flex items-center gap-2.5">
          <h3 className="text-body m-0 font-semibold">{t("people.samples.title")}</h3>
          <VoiceBadge voice={person.voice} />
        </div>
        {samples.length > 0 ? <Samples samples={samples} name={name} /> : <span className="text-small text-muted">{t("people.samples.none", { app: APP_NAME, name })}</span>}
        <div className="flex flex-wrap gap-2">
          {hasVoice && (
            <ConfirmArea
              icon="delete_forever"
              question={person.isMe ? t("settings.privacy.myVoice.question", { count: sampleCount, app: APP_NAME }) : t("people.deleteVoice.question", { name, samples: sampleCount, count: person.meetings, app: APP_NAME })}
              confirmLabel={t("people.deleteVoice.confirm")}
              onConfirm={() => void deleteVoice()}
              trigger={({ onClick, ref }) => (
                <Button ref={ref} icon="delete" className="border-rec text-rec" onClick={onClick}>
                  {t("people.deleteVoice.action")}
                </Button>
              )}
            />
          )}
          {person.isMe && (
            <Button icon="mic" onClick={() => setEnrolling(true)}>
              {t("people.reRecordVoice")}
            </Button>
          )}
        </div>
      </section>

      <section aria-label={t("people.openActions")} className="flex flex-col gap-1.5">
        <h3 className="text-body m-0 font-semibold">{t("people.openActions")}</h3>
        {openActions.length === 0 && <span className="text-small text-muted">{t("people.noOpenActions")}</span>}
        {openActions.map((a) => (
          <div key={a.gid} className="grid grid-cols-[22px_minmax(0,1fr)] items-start gap-2 border-b border-line py-1.5">
            <Icon name="check_box_outline_blank" size={20} className="text-muted" />
            <div>
              <div className="font-serif text-[15.5px] leading-normal">{a.text}</div>
              <button type="button" onClick={() => openMeeting(a.meetingGid)} className="text-small text-muted hover:text-accent">
                {a.meetingTitle}
                {(a.dueText ?? "") && ` · ${a.dueText}`}
              </button>
            </div>
          </div>
        ))}
      </section>

      <section aria-label={t("people.meetingsTogether")} className="flex flex-col gap-0.5">
        <h3 className="text-body m-0 mb-1 font-semibold">{t("people.meetingsTogether")}</h3>
        {meetings.length === 0 && <span className="text-small text-muted">{t("people.noMeetings")}</span>}
        {meetings.map((m) => (
          <button key={m.gid} type="button" onClick={() => openMeeting(m.gid)} className="grid h-10 grid-cols-[22px_minmax(0,1fr)_auto] items-center gap-2.5 rounded-seg px-2 text-left hover:bg-surface2">
            <Icon name="graphic_eq" size={18} className="text-muted" />
            <span className="truncate text-[13.5px] font-medium">{m.title}</span>
            {m.startedAt != null && <span className="text-small text-faint">{formatDate(m.startedAt, locale)}</span>}
          </button>
        ))}
      </section>

      {!person.isMe && (
        <section aria-label={t("people.nameInNotes.title")} className="flex flex-col gap-2 border-t border-line pt-4">
          <h3 className="text-body m-0 font-semibold">{t("people.nameInNotes.title")}</h3>
          <p className="text-small m-0 text-muted">{t("people.nameInNotes.hint", { count: person.meetings })}</p>
          <div>
            <ConfirmArea
              tone="warn"
              icon="person_remove"
              question={t("people.removeName.question", { name, count: person.meetings })}
              confirmLabel={t("people.removeName.confirm")}
              onConfirm={() => void removeName()}
              trigger={({ onClick, ref }) => (
                <Button ref={ref} onClick={onClick}>
                  {t("people.removeName.action")}
                </Button>
              )}
            />
          </div>
        </section>
      )}
      <EnrollDialog open={enrolling} onClose={() => setEnrolling(false)} />
    </div>
  );
}
