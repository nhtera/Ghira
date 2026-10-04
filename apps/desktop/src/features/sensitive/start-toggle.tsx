// SPDX-License-Identifier: Apache-2.0
// Before recording: arm sensitive mode for the next recording. The core keeps
// it, so the popover, the detection prompt and the tray start the same way; it
// clears once a recording started.
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Icon, cn } from "@ghi/ui";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { useLive } from "../../state/live";

const KEY = ["sensitiveNext"] as const;

export function SensitiveStartToggle() {
  const { t } = useTranslation();
  const client = useQueryClient();
  const state = useLive((s) => s.state);
  const armed = useQuery({
    queryKey: KEY,
    staleTime: 0,
    queryFn: async () => {
      const r = await ipc.commands.sensitiveNext();
      return r.status === "ok" && r.data;
    },
  });
  // A recording started (from here, the popover or the tray) cleared it.
  useEffect(() => {
    void client.invalidateQueries({ queryKey: KEY });
  }, [state, client]);
  const on = armed.data ?? false;
  const toggle = async () => {
    client.setQueryData(KEY, !on);
    const r = await ipc.commands.setSensitiveNext(!on);
    if (r.status === "error") void client.invalidateQueries({ queryKey: KEY });
  };
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      title={t("sensitive.tip")}
      onClick={() => void toggle()}
      data-testid="sensitive-start"
      className={cn(
        "inline-flex h-[30px] flex-none items-center gap-1.5 rounded-ctl border px-2.5 text-[12.5px] font-medium whitespace-nowrap",
        on ? "border-transparent bg-warn-soft text-warn" : "border-ctl bg-surface text-muted hover:bg-surface2",
      )}
    >
      <Icon name="visibility_off" size={16} />
      {t("sensitive.next")}
    </button>
  );
}
