// SPDX-License-Identifier: Apache-2.0
// Why the local model can't answer right now. The core refuses with a code
// ("busyRecording", "busyNotes", "noModel"): those are waiting states, shown
// as a neutral note. Any other message is a failure and keeps the alert.
import { useTranslation } from "react-i18next";
import { Icon } from "@ghi/ui";

const CODES = ["busyRecording", "busyNotes", "noModel"] as const;
export type BusyCode = (typeof CODES)[number];

export const busyCode = (error: string): BusyCode | null => (CODES as readonly string[]).includes(error) ? (error as BusyCode) : null;

/**
 * `status`: give the neutral note its own live region (outside a list that is
 * already one).
 */
export function AskError({ error, status }: { error: string; status?: boolean }) {
  const { t } = useTranslation();
  const code = busyCode(error);
  if (code)
    return (
      <p role={status ? "status" : undefined} data-testid="ask-busy" className="text-small m-0 flex items-start gap-2 rounded-panel border border-line2 bg-surface p-3 text-muted">
        <Icon name="schedule" size={16} className="shrink-0" />
        {t(`ask.busy.${code}`)}
      </p>
    );
  return (
    <p role="alert" className="text-small m-0 flex items-start gap-2 rounded-panel border border-line2 bg-surface p-3 text-rec-ink">
      <Icon name="error" size={16} className="shrink-0" />
      {t("system.commandFailed", { message: error })}
    </p>
  );
}
