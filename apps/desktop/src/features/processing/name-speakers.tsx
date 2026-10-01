// SPDX-License-Identifier: Apache-2.0
// D5 "Name your speakers": one card per unnamed voice with a 3 s sample.
// Naming applies to notes and transcript; Skip leaves the voice as "Speaker N".
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Avatar, Button, Icon, useToast } from "@ghi/ui";
import { ipc } from "../../ipc";
import { adapter, type UnnamedSpeaker } from "./speakers-adapter";

function SpeakerCard({ meeting, speaker, onDone }: { meeting: string; speaker: UnnamedSpeaker; onDone: () => void }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const [name, setName] = useState("");
  const [sample, setSample] = useState<{ src: string } | null>(null);
  const number = String(speaker.number);
  const label = t("speakers.numbered", { number });

  const play = async () => {
    const r = await ipc.commands.issueAudioSample(meeting, speaker.t0Ms, speaker.t1Ms, null);
    // No audio (the mock core, a deleted bundle): the card still lets you type a name.
    if (r.status === "ok") setSample({ src: convertFileSrc(r.data, "ghi-audio") });
  };
  const save = async () => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const r = await adapter.rename(meeting, speaker.gid, trimmed);
    if (!r.ok) return show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
    show({ tone: "success", title: t("speakers.renamed", { name: trimmed }) });
    onDone();
  };

  return (
    <li className="flex flex-wrap items-center gap-2.5 border-t border-line pt-2.5">
      <Avatar name={label} initial={number} colorSlot={speaker.colorSlot} label={label} size="xl" />
      <span className="w-24 text-[13px] font-semibold">{label}</span>
      {speaker.t0Ms != null && speaker.t1Ms != null && (
        <Button size="sm" icon="play_arrow" onClick={() => void play()}>
          {t("speakers.play3")}
        </Button>
      )}
      {sample && <audio key={sample.src} src={sample.src} autoPlay aria-label={t("speakers.playing")} />}
      <input
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && !e.nativeEvent.isComposing && void save()}
        placeholder={t("speakers.typeName")}
        aria-label={t("speakers.whoIs")}
        className="h-8 min-w-32 flex-1 rounded-ctl border border-ctl bg-surface px-2.5 text-[13px] text-ink"
      />
      <Button size="sm" variant="primary" disabled={!name.trim()} onClick={() => void save()}>
        {t("common.save")}
      </Button>
    </li>
  );
}

/** `speakers` is what is still unnamed; `onDone(gid)` removes one card, `onSkipAll` dismisses all. */
export function NameSpeakers({ meeting, speakers, onDone, onSkipAll }: { meeting: string; speakers: UnnamedSpeaker[]; onDone: (gid: string) => void; onSkipAll: () => void }) {
  const { t } = useTranslation();
  if (speakers.length === 0) return null;
  return (
    <section aria-label={t("speakers.nameTitle")} className="mb-5 flex flex-col gap-3 rounded-panel border border-line2 px-[18px] py-4">
      <div className="flex items-start gap-2.5">
        <Icon name="record_voice_over" size={20} className="text-warn" />
        <div className="flex-1">
          <h2 className="text-heading m-0 text-[14px]">{t("speakers.nameTitle")}</h2>
          <p className="text-small m-0 text-muted">{t("speakers.nameSubtitle", { count: speakers.length })}</p>
        </div>
        <Button size="sm" variant="ghost" onClick={onSkipAll}>
          {t("common.skip")}
        </Button>
      </div>
      <ul className="m-0 flex list-none flex-col gap-2.5 p-0">
        {speakers.map((s) => (
          <SpeakerCard key={s.gid} meeting={meeting} speaker={s} onDone={() => onDone(s.gid)} />
        ))}
      </ul>
    </section>
  );
}
