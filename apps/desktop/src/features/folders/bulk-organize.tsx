// SPDX-License-Identifier: Apache-2.0
// The selection bar's organize actions: "Move to folder…" and "Add tag…".
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Popover } from "@ghi/ui";
import { MoveDialog } from "./move-dialog";
import { useOrganizeActions, useTags } from "./organize";
import { TagCombobox, type Choice } from "./tag-combobox";

const NONE: ReadonlySet<string> = new Set();

export function BulkOrganize({ meetings }: { meetings: string[] }) {
  const { t } = useTranslation();
  const tags = useTags().data ?? [];
  const actions = useOrganizeActions();
  const [moving, setMoving] = useState(false);
  const [tagging, setTagging] = useState(false);
  const choose = async (c: Choice) => {
    const tag = c.kind === "tag" ? c.tag : await actions.createTag(c.name);
    if (tag && (await actions.tag(meetings, tag))) setTagging(false);
  };
  return (
    <>
      <Button icon="folder" onClick={() => setMoving(true)}>
        {t("organize.moveTo")}
      </Button>
      <Popover
        open={tagging}
        onOpenChange={setTagging}
        label={t("organize.addTag")}
        trigger={
          <Button icon="label">{t("organize.addTag")}</Button>
        }
      >
        <TagCombobox tags={tags} exclude={NONE} autoFocus onChoose={(c) => void choose(c)} />
      </Popover>
      <MoveDialog open={moving} meetings={meetings} onClose={() => setMoving(false)} />
    </>
  );
}
