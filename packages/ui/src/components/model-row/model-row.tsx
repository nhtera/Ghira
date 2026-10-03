// SPDX-License-Identifier: Apache-2.0
// One row of Settings → Models. Status is icon + text; actions depend on it.
import { useTranslation } from "react-i18next";
import { Icon, type IconName } from "../../icons/icon";
import { Button } from "../../primitives/button";
import { cn } from "../../utils/cn";

export type ModelStatus = "installed" | "downloading" | "paused" | "update" | "preview" | "incompatible";

export type ModelRowProps = {
  /** What it does, e.g. "Live English transcript". */
  purpose: string;
  /** Model name, e.g. "Parakeet TDT 0.6B v3". */
  name: string;
  size: string;
  memory?: string;
  languages?: string;
  license?: string;
  status: ModelStatus;
  /** 0..100 while downloading or paused. */
  progress?: number;
  /** Shown for "incompatible", e.g. "32 GB". */
  needsMemory?: string;
  onDownload?: () => void;
  onPause?: () => void;
  onResume?: () => void;
  onUpdate?: () => void;
  onRemove?: () => void;
  className?: string;
};

/** Purpose, size, memory, languages, status: the header row uses the same columns. */
const MODEL_GRID = "sm:grid-cols-[minmax(0,1fr)_64px_72px_96px_minmax(150px,auto)]";

/** The column titles above a list of rows. */
export function ModelRowHeader({ className }: { className?: string }) {
  const { t } = useTranslation();
  return (
    <div className={cn("hidden grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2.5 bg-surface2 px-3 py-2 text-[11.5px] font-semibold text-faint sm:grid", MODEL_GRID, className)}>
      <span>{t("settings.models.columns.purpose")}</span>
      <span>{t("settings.models.columns.size")}</span>
      <span>{t("settings.models.columns.memory")}</span>
      <span>{t("settings.models.columns.languages")}</span>
      <span>{t("settings.models.columns.status")}</span>
    </div>
  );
}

type ModelAction = "download" | "pause" | "resume" | "update" | "remove";

const LOOK: Record<ModelStatus, { icon: IconName; cls: string }> = {
  installed: { icon: "check_circle", cls: "bg-transparent text-accent" },
  downloading: { icon: "download", cls: "bg-accent-soft text-accent" },
  paused: { icon: "pause_circle", cls: "bg-sunk text-muted" },
  update: { icon: "update", cls: "bg-accent-soft text-accent" },
  preview: { icon: "science", cls: "bg-warn-soft text-warn" },
  incompatible: { icon: "block", cls: "bg-sunk text-muted" },
};

function ActionButton({ model, action, onClick, variant }: { model: string; action: ModelAction; onClick: () => void; variant?: "primary" | "ghost" }) {
  const { t } = useTranslation();
  const text = String(t(`settings.models.actions.${action}`));
  return (
    <Button size="sm" variant={variant} onClick={onClick} aria-label={String(t("settings.models.actionFor", { action: text, name: model }))}>
      {text}
    </Button>
  );
}

export function ModelRow(p: ModelRowProps) {
  const { t } = useTranslation();
  const percent = Math.round(p.progress ?? 0);
  const label: string = {
    installed: () => String(t("settings.models.status.installed")),
    downloading: () => String(t("settings.models.status.downloading", { percent })),
    paused: () => String(t("settings.models.status.paused", { percent })),
    update: () => String(t("settings.models.status.update")),
    preview: () => String(t("settings.models.status.preview")),
    incompatible: () => String(t("settings.models.status.incompatible", { ram: p.needsMemory ?? "" })),
  }[p.status]();
  const look = LOOK[p.status];
  const meta = [p.name, p.license && t("settings.models.license", { license: p.license })].filter(Boolean);

  return (
    <div data-status={p.status} className={cn("grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-1.5 bg-surface px-3 py-[9px] text-[12.5px]", MODEL_GRID, p.className)}>
      <span className="min-w-0">
        <span className="block font-medium text-ink">{p.purpose}</span>
        <span className="block text-[11.5px] text-faint">{meta.join(" · ")}</span>
        {(p.memory || p.languages) && <span className="block text-[11.5px] text-faint sm:hidden">{[p.memory && `${t("settings.models.columns.memory")} ${p.memory}`, p.languages].filter(Boolean).join(" · ")}</span>}
        {(p.status === "downloading" || p.status === "paused") && (
          <span role="progressbar" aria-label={p.name} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} className="mt-1.5 block h-1.5 overflow-hidden rounded-[3px] bg-sunk">
            <i className={cn("block h-full", p.status === "paused" ? "bg-faint" : "bg-accent")} style={{ width: `${percent}%` }} />
          </span>
        )}
      </span>
      <span className="text-mono text-right text-[11.5px] text-ink sm:text-left">{p.size}</span>
      <span className="hidden text-mono text-[11.5px] sm:block">{p.memory}</span>
      <span className="hidden text-[12px] text-muted sm:block">{p.languages}</span>
      <span className="col-span-2 flex flex-wrap items-center gap-2 sm:col-span-1">
        <span className={cn("inline-flex h-6 items-center gap-1 rounded-full px-2 text-[11.5px] font-semibold whitespace-nowrap", look.cls)}>
          <Icon name={look.icon} size={14} />
          {label}
        </span>
        {p.status === "downloading" && p.onPause && <ActionButton model={p.name} action="pause" onClick={p.onPause} />}
        {p.status === "paused" && p.onResume && <ActionButton model={p.name} action="resume" onClick={p.onResume} />}
        {p.status === "preview" && p.onDownload && <ActionButton model={p.name} action="download" variant="primary" onClick={p.onDownload} />}
        {p.status === "update" && p.onUpdate && <ActionButton model={p.name} action="update" variant="primary" onClick={p.onUpdate} />}
        {(p.status === "installed" || p.status === "update") && p.onRemove && <ActionButton model={p.name} action="remove" variant="ghost" onClick={p.onRemove} />}
      </span>
    </div>
  );
}
