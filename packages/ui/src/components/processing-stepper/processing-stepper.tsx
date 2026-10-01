// SPDX-License-Identifier: Apache-2.0
// The four final-pass stages as an ordered list. The running step carries a
// progressbar; a failed step offers retry. Status is icon + text, not color.
import { useTranslation } from "react-i18next";
import { Icon, type IconName } from "../../icons/icon";
import { Button } from "../../primitives/button";
import { cn } from "../../utils/cn";

export type StageId = "decoding" | "refiningSpeakers" | "matchingVoices" | "improvingTranscript" | "writingNotes";
export type StepStatus = "pending" | "running" | "done" | "failed" | "skipped";

export type ProcessingStep = {
  id: StageId;
  status: StepStatus;
  /** 0..100 for the running step; omitted = indeterminate. */
  progress?: number;
  /** Seconds left for the running step. */
  estimateSeconds?: number;
};

export type ProcessingStepperProps = {
  steps: ProcessingStep[];
  onRetry?: (id: StageId) => void;
  className?: string;
};

const LOOK: Record<StepStatus, { icon: IconName; icon_cls: string; text: string }> = {
  done: { icon: "check_circle", icon_cls: "text-accent", text: "text-accent" },
  skipped: { icon: "do_not_disturb_on", icon_cls: "text-muted", text: "text-muted" },
  running: { icon: "progress_activity", icon_cls: "text-ink", text: "text-muted" },
  pending: { icon: "radio_button_unchecked", icon_cls: "text-muted", text: "text-muted" },
  failed: { icon: "error", icon_cls: "text-rec-ink", text: "text-rec-ink" },
};

export function ProcessingStepper({ steps, onRetry, className }: ProcessingStepperProps) {
  const { t } = useTranslation();
  const estimate = (s: number) => (s < 90 ? t("processing.estimateSeconds", { count: Math.round(s) }) : t("processing.estimateMinutes", { count: Math.round(s / 60) }));
  return (
    <ol aria-label={t("processing.stepsLabel")} className={cn("m-0 flex list-none flex-col gap-2.5 p-0", className)}>
      {steps.map((s) => {
        const look = LOOK[s.status];
        const label = t(`processing.steps.${s.id}`);
        const running = s.status === "running";
        return (
          <li key={s.id} data-status={s.status} aria-current={running ? "step" : undefined} className="flex flex-col gap-1.5">
            <div className="flex items-center gap-2.5 text-[13px]">
              <Icon name={look.icon} size={18} className={cn(look.icon_cls, running && "animate-spin motion-reduce:animate-none")} />
              <span className={cn("flex-1", s.status === "pending" || s.status === "skipped" ? "text-muted" : "text-ink")}>{label}</span>
              <span className={cn("text-[11.5px] font-semibold", look.text)}>
                {t(`processing.state.${s.status}`)}
                {running && s.estimateSeconds !== undefined && ` · ${estimate(s.estimateSeconds)}`}
              </span>
              {s.status === "failed" && onRetry && (
                <Button size="sm" onClick={() => onRetry(s.id)} aria-label={`${t("common.tryAgain")}: ${label}`}>
                  {t("common.tryAgain")}
                </Button>
              )}
            </div>
            {running && (
              <div
                role="progressbar"
                aria-label={label}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={s.progress === undefined ? undefined : Math.round(s.progress)}
                className="ml-[28px] h-1.5 overflow-hidden rounded-[3px] bg-sunk"
              >
                <i
                  className={cn("block h-full bg-accent", s.progress === undefined && "w-1/3 animate-pulse motion-reduce:animate-none")}
                  style={s.progress === undefined ? undefined : { width: `${s.progress}%` }}
                />
              </div>
            )}
          </li>
        );
      })}
    </ol>
  );
}
