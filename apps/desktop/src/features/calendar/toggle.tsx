// SPDX-License-Identifier: Apache-2.0
// The "ask to record when it starts" switch of one event (Up next strip and popover).
import { useTranslation } from "react-i18next";
import type { EventView } from "../../bindings";
import { Switch } from "../settings/parts";
import { useSetEventAsk } from "./use-calendar";

export function AskToggle({ event }: { event: EventView }) {
  const { t } = useTranslation();
  const setAsk = useSetEventAsk();
  const label = t("library.askToRecord");
  return (
    <span className="flex flex-none items-center gap-2">
      <span aria-hidden className="text-small text-muted">
        {label}
      </span>
      <Switch
        checked={event.ask}
        label={label}
        onChange={(v) => void setAsk(event.key, v)}
      />
    </span>
  );
}
