// SPDX-License-Identifier: Apache-2.0
// Screen readers hear new speaker turns only ("Linh: first words…"), never
// every word or line (brief §8). A visually hidden polite region.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useLive } from "../state/live";
import { useSpeakerLabel } from "../state/speaker-label";

export function LiveAnnouncer() {
  const { t } = useTranslation();
  const lines = useLive((s) => s.lines);
  const speakers = useLive((s) => s.speakers);
  const last = useRef<number | null | undefined>(undefined);
  const [message, setMessage] = useState("");
  const labelOf = useSpeakerLabel();
  useEffect(() => {
    const line = lines[lines.length - 1];
    if (!line) {
      last.current = undefined;
      return;
    }
    if (line.speaker === last.current) return;
    last.current = line.speaker;
    const sp = line.speaker != null ? speakers[line.speaker] : undefined;
    const name = sp ? labelOf(sp) : t("speakers.identifying");
    const text = line.text.split(" ").slice(0, 8).join(" ");
    setMessage(t("shell.newSpeakerTurn", { name, text }));
  }, [lines, speakers, t, labelOf]);
  return (
    <div aria-live="polite" aria-atomic="true" className="sr-only" data-testid="speaker-announcer">
      {message}
    </div>
  );
}
