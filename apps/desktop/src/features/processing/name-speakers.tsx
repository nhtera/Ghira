// SPDX-License-Identifier: Apache-2.0
// D5 "Name your speakers": one card per unnamed voice with a 3 s sample.
// Naming applies to notes and transcript; Skip leaves the voice as "Speaker N".
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar, Button, Icon, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { errorText } from "../people/error-text";
import { useStoredSpeakerActions } from "../speakers/use-speaker-actions";
import { useMeetingAttendees } from "../calendar/use-calendar";
import { adapter, type UnnamedSpeaker } from "./speakers-adapter";

function SpeakerCard({ meeting, speaker, invited, onDone }: { meeting: string; speaker: UnnamedSpeaker; invited: string[]; onDone: () => void }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const [name, setName] = useState("");
  const [sample, setSample] = useState<{ src: string } | null>(null);
  const actions = useStoredSpeakerActions(meeting);
  const [dismissed, setDismissed] = useState(false);
  const suggestion = dismissed ? null : speaker.suggestion;
  const number = String(speaker.number);
  const label = t("speakers.numbered", { number });

  const play = async () => {
    const r = await ipc.commands.issueAudioSample(meeting, speaker.t0Ms, speaker.t1Ms, null);
    // No audio (the mock core, a deleted bundle): the card still lets you type a name.
    if (r.status === "ok") setSample({ src: ipc.audioUrl(r.data) });
  };
  const save = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const r = await adapter.rename(meeting, speaker.gid, trimmed);
    if (!r.ok) return show({ tone: "warning", title: errorText(t, r.error) });
    show({ tone: "success", title: t("speakers.renamed", { name: trimmed }) });
    onDone();
  };

  const suggested = suggestion ? (suggestion.isMe ? t("speakers.me") : suggestion.name) : "";
  // One suggestion request at a time: a second click while it runs does nothing.
  const [pending, setPending] = useState(false);
  const inFlight = useRef(false);
  const once = async (run: () => Promise<void>) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setPending(true);
    try {
      await run();
    } finally {
      inFlight.current = false;
      setPending(false);
    }
  };
  const accept = () =>
    once(async () => {
      if (suggestion && (await actions.accept(speaker.gid, suggestion.isMe, suggested))) onDone();
    });
  const dismiss = () =>
    once(async () => {
      if (await actions.dismiss(speaker.gid)) setDismissed(true);
    });

  return (
    <li className="flex flex-wrap items-center gap-2.5 border-t border-line pt-2.5">
      <Avatar name={label} initial={number} colorSlot={speaker.colorSlot} label={label} size="lg" className="size-[30px]" />
      <span className="w-[84px] text-[13px] font-semibold">{label}</span>
      {speaker.t0Ms != null && speaker.t1Ms != null && (
        <Button size="sm" className="h-[30px]" icon="play_arrow" onClick={() => void play()}>
          {t("speakers.play3")}
        </Button>
      )}
      {suggestion && (
        <span data-testid="voice-suggestion" className="inline-flex items-center">
          <button
            type="button"
            disabled={pending}
            onClick={() => void accept()}
            aria-label={t("speakers.acceptSuggestion", { name: suggested })}
            className="inline-flex h-[30px] items-center gap-0.5 rounded-full bg-warn-soft px-2.5 text-[12.5px] font-semibold text-warn"
          >
            {suggestion.score != null
              ? t("speakers.suggestionPill", { name: suggested, percent: Math.round(suggestion.score * 100) })
              : t("speakers.suggestionPillPlain", { name: suggested })}
            <Icon name="check" size={15} />
          </button>
          <button
            type="button"
            disabled={pending}
            onClick={() => void dismiss()}
            aria-label={t("speakers.dismissSuggestionFor", { speaker: label })}
            className="ml-0.5 inline-flex size-7 items-center justify-center rounded-full text-muted hover:bg-sunk"
          >
            <Icon name="close" size={14} />
          </button>
        </span>
      )}
      {sample && <audio key={sample.src} src={sample.src} autoPlay aria-label={t("speakers.playing")} />}
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && !e.nativeEvent.isComposing && void save()}
        placeholder={t("speakers.typeName")}
        aria-label={t("speakers.whoIs")}
        className="h-[30px] min-w-32 flex-1 rounded-ctl border border-ctl bg-surface px-2.5 text-[13px] text-ink"
      />
      <Button size="sm" variant="primary" className="h-[30px] px-3" disabled={!name.trim()} onClick={() => void save()}>
        {t("common.save")}
      </Button>
      {invited.length > 0 && (
        <div role="group" aria-label={t("calendar.inInvite")} className="flex basis-full flex-wrap items-center gap-1.5">
          <span className="text-small text-muted">{t("calendar.inInvite")}</span>
          {invited.map((n) => (
            <Button key={n} size="sm" variant="ghost" onClick={() => setName(n)}>
              {n}
            </Button>
          ))}
        </div>
      )}
    </li>
  );
}

/** `speakers` is what is still unnamed; `onDone(gid)` removes one card, `onSkipAll` dismisses all. */
export function NameSpeakers({ meeting, speakers, onDone, onSkipAll }: { meeting: string; speakers: UnnamedSpeaker[]; onDone: (gid: string) => void; onSkipAll: () => void }) {
  const { t } = useTranslation();
  const attendees = useMeetingAttendees(meeting);
  if (speakers.length === 0) return null;
  // By label number, so voices past the 8th (the Others lane) come after the first eight.
  const ordered = [...speakers].sort((a, b) => a.number - b.number);
  return (
    <section aria-label={t("speakers.nameTitle")} className="mb-[22px] flex max-w-[740px] flex-col gap-3 rounded-panel border border-line2 px-[18px] py-4">
      <div className="flex items-start gap-2.5">
        <Icon name="record_voice_over" size={20} className="text-warn" />
        <div className="flex-1">
          <h2 className="text-heading m-0 text-[14px]">{t("speakers.nameTitle")}</h2>
          <p className="m-0 text-[12.5px] text-muted">{t("speakers.nameSubtitle", { count: speakers.length })}</p>
        </div>
        <Button size="sm" variant="ghost" className="font-medium text-muted" onClick={onSkipAll}>
          {t("common.skip")}
        </Button>
      </div>
      <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
        {ordered.map((s) => (
          <SpeakerCard key={s.gid} meeting={meeting} speaker={s} invited={attendees} onDone={() => onDone(s.gid)} />
        ))}
      </ul>
    </section>
  );
}
