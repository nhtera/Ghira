// SPDX-License-Identifier: Apache-2.0
// D1 step 6: a 10 s test. Both levels move and a transcript line shows up; a
// silent system track means system audio is off, so the hint offers Room mode.
import { useTranslation } from "react-i18next";
import { Button, Icon, LevelMeter } from "@ghi/ui";
import { Notice, StepActions, StepFrame, type StepNav } from "./step-frame";
import { useTestCapture } from "./use-test-capture";

const QUIET = -70;

export function TestStep({ nav, seconds }: { nav: StepNav; seconds?: number }) {
  const { t } = useTranslation();
  const test = useTestCapture(seconds);
  const idle = test.phase === "idle";
  const running = test.phase === "running";
  const done = test.phase === "done";
  // Until the first level arrives both are null: not "no microphone" yet.
  const heard = test.levels.mic !== null || test.levels.system !== null;

  return (
    <StepFrame title={t("onboarding.test.title")} body={t("onboarding.test.body")}>
      <div className="flex flex-col gap-3 rounded-xl bg-surface2 px-5 py-4">
        {/* Level meters are not announced; the status line below is. */}
        <LevelMeter source="mic" db={running && heard ? test.levels.mic : QUIET} />
        <LevelMeter source="system" db={running && heard ? test.levels.system : QUIET} />
        {test.line && (
          <div className="grid grid-cols-[24px_minmax(0,1fr)] gap-2.5 border-t border-line pt-2.5">
            <span aria-hidden className="grid size-6 place-items-center rounded-full bg-accent-soft text-[11px] font-bold text-accent">
              {Array.from(t("speakers.me"))[0]}
            </span>
            <p className="m-0 font-serif text-[16px] leading-relaxed">{test.line}</p>
          </div>
        )}
      </div>

      {/* Announced once per state; the ticking number stays outside it. */}
      <div className="flex min-h-6 flex-wrap items-center gap-2 text-[14px] font-semibold">
        <span role="status" className="flex items-center gap-1.5 text-rec-ink empty:hidden">
          {running && (
            <>
              <Icon name="graphic_eq" size={19} />
              {t("onboarding.test.running")}
            </>
          )}
        </span>
        {running && (
          <span aria-hidden className="text-mono text-muted">
            {test.secondsLeft}
          </span>
        )}
        {done && !test.systemSilent && (
          <span className="flex items-center gap-1.5 text-accent">
            <Icon name="check_circle" size={19} />
            {t("onboarding.test.ok")}
          </span>
        )}
      </div>
      {test.phase === "failed" && (
        <Notice tone="warn" icon="warning">
          {test.error || t("record.micUnavailable")}
        </Notice>
      )}
      {done && test.systemSilent && (
        <Notice tone="warn" icon="warning">
          {t("onboarding.test.systemSilent")} {t("onboarding.permissions.roomOnly")}
        </Notice>
      )}

      <StepActions
        nav={nav}
        start={
          !done && (
            <Button size="lg" variant="danger" disabled={running} onClick={test.start} icon="graphic_eq" data-onboarding-primary={idle || test.phase === "failed" ? "" : undefined}>
              {running ? t("onboarding.test.running") : t("onboarding.test.run")}
            </Button>
          )
        }
        primary={done ? t("common.continue") : t("common.skip")}
        primaryProps={{ variant: done ? "primary" : "ghost", className: done ? undefined : "text-muted" }}
        enter={done}
      />
    </StepFrame>
  );
}
