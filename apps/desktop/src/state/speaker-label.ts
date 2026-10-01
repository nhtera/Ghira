// SPDX-License-Identifier: Apache-2.0
// How a live speaker is shown: the core labels unnamed speakers "Speaker N"
// and the user "Me"; the UI shows those localized (and the number in the
// avatar). Names the user gave are shown as they are.
import { useCallback } from "react";
import { useTranslation } from "react-i18next";
import type { SpeakerInfo } from "../bindings";

export const speakerNumber = (s: SpeakerInfo): string | null => (s.isMe ? null : (/^Speaker (\d+)$/.exec(s.label)?.[1] ?? null));

export function useSpeakerLabel() {
  const { t } = useTranslation();
  return useCallback(
    (s: SpeakerInfo) => {
      const n = speakerNumber(s);
      return s.isMe ? t("speakers.me") : n ? t("speakers.numbered", { number: n }) : s.label;
    },
    [t],
  );
}
