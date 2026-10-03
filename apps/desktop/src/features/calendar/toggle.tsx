// SPDX-License-Identifier: Apache-2.0
// The "ask to record when it starts" switch of one event (Up next strip and popover).
import { useTranslation } from "react-i18next";
import type { EventView } from "../../bindings";
import { Switch } from "../settings/parts";
import { useSetEventAsk } from "./use-calendar";

/** `compact`: the switch alone (the label is its tooltip), for the popover row. */
export function AskToggle({ event, compact = false }: { event: EventView; compact?: boolean }) {
  const { t } = useTranslation();
  const setAsk = useSetEventAsk();
  const label = t("library.askToRecord");
  if (compact)
    return (
      <span title={label}>
        <Switch checked={event.ask} label={label} onChange={(v) => void setAsk(event.key, v)} />
      </span>
    );
  return (
    <span className="flex flex-none items-center gap-2">
      <Switch
        checked={event.ask}
        label={label}
        onChange={(v) => void setAsk(event.key, v)}
      />
      <span aria-hidden className="text-[12.5px] text-ink">
        {label}
      </span>
    </span>
  );
}
