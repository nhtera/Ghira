// SPDX-License-Identifier: Apache-2.0
// Turning sensitive mode on deletes audio (what was recorded so far goes when
// the recording stops; a stored meeting loses it now), so it asks first, inline
// (never a modal mid-recording). Turning it off needs no question.
import { useQueryClient } from "@tanstack/react-query";
import { InlineConfirm, useToast } from "@ghi/ui";
import { useTranslation } from "react-i18next";
import { ipc } from "../../ipc";
import { invalidateMeeting } from "../../state/meeting-queries";
import { sensitiveError } from "./errors";

export function SensitiveConfirm({ meeting, recording, onClose }: { meeting: string; recording: boolean; onClose: () => void }) {
  const { t } = useTranslation();
  const { show } = useToast();
  const client = useQueryClient();
  const confirm = async () => {
    const r = await ipc.commands.setMeetingSensitive(meeting, true);
    if (r.status === "error") show({ tone: "warning", title: sensitiveError(t, r.error) });
    else void invalidateMeeting(client, meeting);
    onClose();
  };
  return (
    <div data-testid="sensitive-confirm">
      <InlineConfirm
        icon="visibility_off"
        question={recording ? t("sensitive.confirmLive") : t("sensitive.confirmStored")}
        confirmLabel={t("sensitive.confirm")}
        onConfirm={() => void confirm()}
        onCancel={onClose}
      />
    </div>
  );
}
