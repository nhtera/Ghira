// SPDX-License-Identifier: Apache-2.0
// Shared pieces of the Settings sections: layout, the switch, the settings
// query + patch helper.
import { useCallback, useId, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { cn, useToast, Icon, type IconName } from "@ghi/ui";
import type { AppSettings, SettingsPatch } from "../../bindings";
import { ipc } from "../../ipc";
import { settingsQuery } from "../../shell/root-view";

export function useSettings() {
  const { t } = useTranslation();
  const { show } = useToast();
  const queryClient = useQueryClient();
  const { data } = useQuery(settingsQuery);
  const patch = useCallback(
    async (p: SettingsPatch): Promise<AppSettings | null> => {
      const r = await ipc.commands.updateSettings(p);
      if (r.status === "ok") {
        queryClient.setQueryData(settingsQuery.queryKey, r.data);
        return r.data;
      }
      show({ tone: "warning", title: t("system.commandFailed", { message: r.error }) });
      return null;
    },
    [queryClient, show, t],
  );
  return { settings: data, patch };
}

/** A command failure as a warning toast. */
export function useFail() {
  const { t } = useTranslation();
  const { show } = useToast();
  return useCallback((message: string) => show({ tone: "warning", title: t("system.commandFailed", { message }) }), [show, t]);
}

export function Card({ title, hint, children, className, danger }: { title?: string; hint?: ReactNode; children?: ReactNode; className?: string; danger?: boolean }) {
  return (
    <section className={cn("flex max-w-2xl flex-col gap-3 rounded-xl border px-4 py-4", danger ? "border-rec bg-rec-soft" : "border-line bg-surface", className)}>
      {title && <h3 className="text-body m-0 font-semibold">{title}</h3>}
      {hint && <p className="text-small m-0 text-muted">{hint}</p>}
      {children}
    </section>
  );
}

/** A label + hint on the left, the control on the right. */
export function Row({ label, hint, children, id }: { label: string; hint?: ReactNode; children?: ReactNode; id?: string }) {
  return (
    <div className="flex min-h-9 items-center justify-between gap-6">
      <div className="flex min-w-0 flex-col gap-0.5">
        <span id={id} className="text-body">
          {label}
        </span>
        {hint && <span className="text-small text-muted">{hint}</span>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

export function Switch({ checked, onChange, labelledBy, label, disabled }: { checked: boolean; onChange: (v: boolean) => void; labelledBy?: string; label?: string; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-labelledby={labelledBy}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative inline-flex h-6 w-10 shrink-0 items-center rounded-full border border-line2 transition-colors disabled:opacity-50",
        checked ? "bg-accent" : "bg-sunk",
      )}
    >
      <i className={cn("block size-[18px] rounded-full bg-on-accent shadow transition-transform", checked ? "translate-x-[18px]" : "translate-x-[3px]")} />
    </button>
  );
}

/** A switch row: the label names the switch. */
export function SwitchRow({ label, hint, checked, onChange, disabled, testId }: { label: string; hint?: ReactNode; checked: boolean; onChange: (v: boolean) => void; disabled?: boolean; testId?: string }) {
  const id = useId();
  return (
    <div data-testid={testId}>
      <Row label={label} hint={hint} id={id}>
        <Switch checked={checked} onChange={onChange} labelledBy={id} disabled={disabled} />
      </Row>
    </div>
  );
}

export function Note({ icon = "info", children, tone }: { icon?: IconName; children: ReactNode; tone?: "warn" }) {
  return (
    <p className={cn("text-small m-0 flex items-start gap-2", tone === "warn" ? "text-warn" : "text-muted")}>
      <Icon name={icon} size={16} className="mt-px shrink-0" />
      <span>{children}</span>
    </p>
  );
}

export const inputCls =
  "h-8 min-w-0 rounded-ctl border border-line2 bg-surface px-2.5 text-[13.5px] text-ink placeholder:text-faint focus-visible:outline-2 focus-visible:outline-accent";
