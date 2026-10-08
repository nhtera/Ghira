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
      show({
        tone: "warning",
        title: t("system.commandFailed", { message: r.error }),
      });
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

/** A flat group: an optional bold heading + hint, then its rows (no box). */
export function Card({ title, hint, children, className, danger }: { title?: string; hint?: ReactNode; children?: ReactNode; className?: string; danger?: boolean }) {
  return (
    <section className={cn("flex flex-col", title && "mt-[26px]", danger && "text-rec-ink", className)}>
      {title && <h3 className="m-0 text-[13px] font-semibold">{title}</h3>}
      {hint && <p className="m-0 text-[12.5px] text-muted">{hint}</p>}
      {children && <div className={cn("flex flex-col [&>:not([data-row])]:mt-3 [&>:first-child]:mt-0", title && "mt-2")}>{children}</div>}
    </section>
  );
}

/** A label + hint on the left, the control on the right, a rule below. */
export function Row({ label, hint, children, id }: { label: string; hint?: ReactNode; children?: ReactNode; id?: string }) {
  return (
    <div data-row className="flex items-center gap-4 border-b border-line py-3.5">
      <div className="min-w-0 flex-1">
        <div id={id} className="text-[14px] font-medium">
          {label}
        </div>
        {hint && <div className="text-[12.5px] leading-normal text-muted">{hint}</div>}
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
      // px, not rem: the knob stays centred at any text size.
      // A locked switch fades its track only: the knob stays white.
      className={cn("inline-flex h-[20px] w-[36px] shrink-0 items-center rounded-full border-0 p-[2px] transition-colors disabled:cursor-default", checked ? (disabled ? "bg-accent/45" : "bg-accent") : disabled ? "bg-line2/50" : "bg-line2")}
    >
      <i className={cn("block size-[16px] rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/0.25)] transition-transform", checked && "translate-x-[16px]")} />
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

/** Segmented controls at the size the design draws them in Settings. */
export const bigSegCls = "[&>button]:h-8 [&>button]:px-3.5 [&>button]:text-[13.5px]";

export const inputCls = "h-8 min-w-0 rounded-ctl border border-line2 bg-surface px-2.5 text-[13.5px] text-ink placeholder:text-faint focus-visible:outline-2 focus-visible:outline-accent";
