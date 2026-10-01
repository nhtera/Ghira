// SPDX-License-Identifier: Apache-2.0
// What you can do with one speaker, from their chip: name them (type or pick,
// Enter), merge into someone else, split some of their lines off, or mark
// them as not a person. Saving the voice is offered only when third-party
// voice profiles are on, and always goes through the consent dialog.
import { useQuery } from "@tanstack/react-query";
import { useMemo, useState, type ReactElement } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { APP_NAME } from "@ghi/i18n";
import { Button, Icon, Popover, SpeakerChip, cn, usePlatform } from "@ghi/ui";
import type { SpeakerInfo } from "../../bindings";
import { ipc } from "../../ipc";
import { settingsQuery } from "../../shell/root-view";
import { useLive } from "../../state/live";
import { speakerNumber, useSpeakerLabel } from "../../state/speaker-label";
import { ConsentDialog } from "./consent-dialog";
import { NameField } from "./name-field";
import { useSpeakerActions } from "./use-speaker-actions";

type View = "main" | "merge" | "split";

const knownNamesQuery = {
  queryKey: ["knownSpeakerNames"] as const,
  queryFn: async () => {
    const r = await ipc.commands.knownSpeakerNames();
    if (r.status === "error") throw new Error(r.error);
    return r.data;
  },
  staleTime: 5 * 60_000,
};

function Row({ icon, children, onClick, danger }: { icon: Parameters<typeof Icon>[0]["name"]; children: string; onClick: () => void; danger?: boolean }) {
  return (
    <button type="button" onClick={onClick} className={cn("text-body flex h-8 w-full items-center gap-2 rounded-seg px-2 text-left hover:bg-sunk", danger && "text-rec")}>
      <Icon name={icon} size={16} />
      {children}
    </button>
  );
}

function MergeView({ from, onDone }: { from: SpeakerInfo; onDone: () => void }) {
  const actions = useSpeakerActions();
  const labelOf = useSpeakerLabel();
  const others = useLive(useShallow((s) => Object.values(s.speakers).filter((x) => x.id !== from.id && !x.notPerson && !x.provisional).map((x) => x.id)));
  const speakers = useLive.getState().speakers;
  return (
    <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
      {others.map((id) => {
        const s = speakers[id];
        return (
          <li key={id}>
            <button
              type="button"
              onClick={() => void actions.merge(from.id, id).then((ok) => ok && onDone())}
              className="flex h-9 w-full items-center rounded-seg px-1 hover:bg-sunk"
            >
              <SpeakerChip state={speakerNumber(s) ? "numbered" : "named"} name={labelOf(s)} colorSlot={s.colorSlot} isMe={s.isMe} className="border-transparent" />
            </button>
          </li>
        );
      })}
    </ul>
  );
}

function SplitView({ from, onDone }: { from: SpeakerInfo; onDone: () => void }) {
  const { t } = useTranslation();
  const actions = useSpeakerActions();
  const all = useLive((s) => s.lines);
  const lines = useMemo(() => all.filter((l) => l.speaker === from.id && l.gid), [all, from.id]);
  const [picked, setPicked] = useState<string[]>([]);
  const [name, setName] = useState("");
  const toggle = (gid: string) => setPicked((p) => (p.includes(gid) ? p.filter((x) => x !== gid) : [...p, gid]));
  return (
    <div className="flex flex-col gap-2.5">
      <p className="text-small m-0 text-muted">{t("speakers.split.body", { app: APP_NAME })}</p>
      <ul aria-label={t("speakers.split.action")} className="m-0 flex max-h-48 list-none flex-col gap-0.5 overflow-auto p-0">
        {lines.map((l) => (
          <li key={l.gid}>
            <label className="text-small flex cursor-pointer items-start gap-2 rounded-seg px-1.5 py-1 hover:bg-sunk">
              <input type="checkbox" checked={picked.includes(l.gid)} onChange={() => toggle(l.gid)} className="mt-0.5" />
              <span className="min-w-0 flex-1">{l.text}</span>
            </label>
          </li>
        ))}
      </ul>
      <input value={name} onChange={(e) => setName(e.target.value)} aria-label={t("speakers.split.otherName")} placeholder={t("speakers.split.otherName")} className="text-body h-9 rounded-ctl border border-ctl bg-surface px-3" />
      <Button variant="primary" disabled={!picked.length} onClick={() => void actions.split(from.id, picked, name).then((id) => id != null && onDone())}>
        {t("speakers.split.go", { count: picked.length })}
      </Button>
    </div>
  );
}

function Content({ speaker, onClose, onNamed }: { speaker: SpeakerInfo; onClose: () => void; onNamed: (name: string, saveVoice: boolean) => void }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const actions = useSpeakerActions();
  const labelOf = useSpeakerLabel();
  const [view, setView] = useState<View>("main");
  const [saveVoice, setSaveVoice] = useState(false);
  const names = useQuery(knownNamesQuery).data ?? [];
  // Voice profiles of other people are off in this phase; the option only shows when they are on.
  const canSaveVoice = useQuery(settingsQuery).data?.voiceProfilesThirdParty === true;
  const title = speaker.provisional || speakerNumber(speaker) ? t("speakers.whoIs") : t("speakers.rename");

  if (view !== "main") {
    return (
      <div className="flex w-72 flex-col gap-2">
        <button type="button" onClick={() => setView("main")} className="text-small inline-flex h-7 items-center gap-1 self-start rounded-seg px-1.5 text-muted hover:bg-sunk">
          <Icon name="arrow_back" size={14} />
          {t("common.back")}
        </button>
        <b className="text-body font-semibold">{view === "merge" ? t("speakers.mergeInto") : t("speakers.split.title", { name: labelOf(speaker) })}</b>
        {view === "merge" ? <MergeView from={speaker} onDone={onClose} /> : <SplitView from={speaker} onDone={onClose} />}
      </div>
    );
  }
  return (
    <div className="flex w-72 flex-col gap-2.5">
      <b className="text-body font-semibold">{title}</b>
      <NameField
        names={names}
        label={t("speakers.rename")}
        placeholder={t("speakers.typeName")}
        onSubmit={(name) => {
          if (saveVoice) return onNamed(name, true);
          void actions.rename(speaker.id, name).then((ok) => ok && onClose());
        }}
      />
      {canSaveVoice && (
        <div className="flex flex-col gap-1">
          <label className="text-body flex cursor-pointer items-center gap-2">
            <input type="checkbox" checked={saveVoice} onChange={(e) => setSaveVoice(e.target.checked)} />
            {t("speakers.saveVoice")}
          </label>
          <p className="text-small m-0 text-muted">{saveVoice ? t("speakers.saveVoiceNext") : t("speakers.saveVoiceHint", { context: platform, app: APP_NAME })}</p>
        </div>
      )}
      <div className="-mx-1 border-t border-line pt-1.5">
        <Row icon="call_merge" onClick={() => setView("merge")}>
          {t("speakers.mergeInto")}
        </Row>
        <Row icon="call_split" onClick={() => setView("split")}>
          {t("speakers.split.action")}
        </Row>
        <Row icon="block" danger onClick={() => void actions.notPerson(speaker.id).then((ok) => ok && onClose())}>
          {t("speakers.notPerson.action")}
        </Row>
      </div>
    </div>
  );
}

/** The chip, opening the speaker's popover. The chip's own look is kept; the button only adds the opening. */
export function SpeakerPopover({ speaker, chip }: { speaker: SpeakerInfo; chip: ReactElement }) {
  const { t } = useTranslation();
  const actions = useSpeakerActions();
  const [open, setOpen] = useState(false);
  const [consentFor, setConsentFor] = useState<string | null>(null);
  return (
    <>
      <Popover
        open={open}
        onOpenChange={setOpen}
        label={t("speakers.panel")}
        trigger={
          <button type="button" className="rounded-full">
            {chip}
          </button>
        }
      >
        <Content
          speaker={speaker}
          onClose={() => setOpen(false)}
          onNamed={(name) => {
            setOpen(false);
            setConsentFor(name);
          }}
        />
      </Popover>
      <ConsentDialog
        open={consentFor != null}
        name={consentFor ?? ""}
        onCancel={() => setConsentFor(null)}
        onConfirm={() => {
          // "Yes" will also save the voice profile (save_voice_profile, phase 14); for now it names the speaker.
          const name = consentFor;
          setConsentFor(null);
          if (name) void actions.rename(speaker.id, name);
        }}
      />
    </>
  );
}
