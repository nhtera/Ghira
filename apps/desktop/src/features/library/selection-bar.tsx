// SPDX-License-Identifier: Apache-2.0
// The bulk bar over a multi-selection: Export…, Delete (inline confirm), Clear.
import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button, Icon, InlineConfirm, usePlatform } from "@ghi/ui";

export function SelectionBar({ count, onExport, onDelete, onClear, organize }: { count: number; onExport: () => void; onDelete: () => void; onClear: () => void; organize?: ReactNode }) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const [asking, setAsking] = useState(false);
  if (asking) {
    return (
      <InlineConfirm
        question={t("library.deleteQuestion", { context: platform, count })}
        confirmLabel={t("common.delete")}
        onCancel={() => setAsking(false)}
        onConfirm={() => {
          setAsking(false);
          onDelete();
        }}
      />
    );
  }
  return (
    <div
      role="toolbar"
      aria-label={t("library.selected", { count })}
      className="flex h-[46px] items-center gap-2 rounded-row border border-line bg-surface2 pr-2 pl-3.5 text-[13px]"
    >
      <Icon name="check_box" size={19} className="text-accent" />
      <b className="flex-1 font-semibold">{t("library.selected", { count })}</b>
      {organize}
      <Button icon="ios_share" onClick={onExport}>
        {t("library.exportSel")}
      </Button>
      <Button icon="delete" className="border-rec text-rec" onClick={() => setAsking(true)}>
        {t("common.delete")}
      </Button>
      <Button variant="ghost" onClick={onClear}>
        {t("library.clearSelection")}
      </Button>
    </div>
  );
}
