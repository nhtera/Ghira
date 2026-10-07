// SPDX-License-Identifier: Apache-2.0
// Settings → Models: which engine writes the transcript after a meeting.
// Standard (Nemotron) is the default; High accuracy (Whisper) is an extra
// download, listed with the other models once chosen. Hidden in a build
// without Whisper.
import { cn, useToast } from "@ghi/ui";
import { formatBytes } from "@ghi/i18n";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type { AsrEngine } from "../../bindings";
import { ipc } from "../../ipc";
import { Card, Note } from "./parts";

const KEY = ["transcription-engine"];

export function EngineCard({ onChanged }: { onChanged: () => void }) {
  const { t, i18n } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  const { data } = useQuery({
    queryKey: KEY,
    queryFn: async () => {
      const r = await ipc.commands.transcriptionEngine();
      if (r.status === "error") throw new Error(r.error);
      return r.data;
    },
  });
  if (!data?.whisperAvailable) return null;

  const choose = async (engine: AsrEngine) => {
    if (engine === data.engine) return;
    const r = await ipc.commands.setTranscriptionEngine(engine);
    if (r.status === "error") {
      show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      return;
    }
    client.setQueryData(KEY, r.data);
    // Whisper's models join (or leave) the list below.
    onChanged();
  };
  const options: { id: AsrEngine; label: string; hint: string }[] = [
    { id: "nemo", label: t("settings.models.engineNemo"), hint: t("settings.models.engineNemoHint") },
    {
      id: "whisper",
      label: t("settings.models.engineWhisper"),
      hint: t("settings.models.engineWhisperHint", { size: formatBytes(data.whisperBytes, i18n.language) }),
    },
  ];

  return (
    <Card title={t("settings.models.engineTitle")} hint={t("settings.models.engineHint")}>
      <div role="radiogroup" aria-label={t("settings.models.engineTitle")} className="grid grid-cols-2 gap-2">
        {options.map((o) => {
          const on = o.id === data.engine;
          return (
            <button
              key={o.id}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => void choose(o.id)}
              className={cn("flex min-w-0 flex-col gap-1 rounded-[10px] border-[1.5px] px-3.5 py-3 text-left", on ? "border-accent bg-accent-soft" : "border-ctl bg-surface hover:bg-surface2")}
            >
              <b className="text-[14px]">{o.label}</b>
              <span className="text-[12px] text-muted">{o.hint}</span>
            </button>
          );
        })}
      </div>
      {data.engine === "whisper" && !data.whisperInstalled && <Note icon="download">{t("settings.models.engineWaiting")}</Note>}
    </Card>
  );
}
