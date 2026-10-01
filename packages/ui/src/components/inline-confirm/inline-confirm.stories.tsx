// SPDX-License-Identifier: Apache-2.0
import { useTranslation } from "react-i18next";
import type { Story, StoryMeta } from "../../story";
import { Button } from "../../primitives/button";
import { ConfirmArea, InlineConfirm } from "./inline-confirm";

export default { title: "Inline confirm", width: 640 } satisfies StoryMeta;

const noop = () => {};

function DeleteAll() {
  const { t } = useTranslation();
  return (
    <InlineConfirm
      question={t("settings.privacy.deleteConfirm", { count: 142, profiles: 5 })}
      confirmLabel={t("common.delete")}
      onConfirm={noop}
      onCancel={noop}
    />
  );
}

function DeleteVoice() {
  const { t } = useTranslation();
  return (
    <InlineConfirm
      question={t("people.deleteVoice.question", { name: "Linh", samples: 3, count: 18 })}
      confirmLabel={t("people.deleteVoice.confirm")}
      onConfirm={noop}
      onCancel={noop}
    />
  );
}

function Replaces() {
  const { t } = useTranslation();
  return (
    <ConfirmArea
      question={t("people.deleteVoice.question", { name: "Linh", samples: 3, count: 18 })}
      confirmLabel={t("people.deleteVoice.confirm")}
      onConfirm={noop}
      trigger={(p) => (
        <Button {...p} variant="danger" icon="delete">
          {t("people.deleteVoice.action")}
        </Button>
      )}
    />
  );
}

export const DeleteEverything: Story = { render: () => <DeleteAll /> };
export const DeleteVoiceData: Story = { render: () => <DeleteVoice /> };
export const ReplacesTrigger: Story = {
  render: () => <Replaces />,
  note: "Click the button: the panel takes its place. Cancel or Escape returns focus to it.",
};
