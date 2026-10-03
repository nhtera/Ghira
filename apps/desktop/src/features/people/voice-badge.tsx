// SPDX-License-Identifier: Apache-2.0
// A person's voice state, as an icon and words (never color alone): own
// voice, agreed (with the date), or no voice profile.
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { formatDate, type Locale } from "@ghi/i18n";
import { Icon, cn } from "@ghi/ui";
import type { PersonVoice } from "../../bindings";

export function voiceText(v: PersonVoice, t: TFunction, locale: Locale): string {
  if (v.kind === "self") return t("people.voiceProfile.self");
  if (v.kind === "agreed") return t("people.voiceProfile.agreed", { date: v.atMs != null ? formatDate(v.atMs, locale) : "" });
  return t("people.voiceProfile.none");
}

/** Day and month only, the list has no room for the year. */
const shortDate = (ms: number, l: Locale) => new Date(ms).toLocaleDateString(l === "vi" ? "vi-VN" : "en-GB", { day: "2-digit", month: "2-digit" });

export function VoiceBadge({ voice, compact }: { voice: PersonVoice; compact?: boolean }) {
  const { t, i18n } = useTranslation();
  const locale: Locale = i18n.language === "vi" ? "vi" : "en";
  const has = voice.kind !== "none";
  const text = voiceText(voice, t, locale);
  return (
    <span
      title={text}
      className={cn(
        "text-small inline-flex items-center gap-1 rounded-full font-semibold",
        compact ? "text-muted" : "h-6 px-2.5",
        !compact && (has ? "bg-accent-soft text-accent" : "bg-sunk text-muted"),
        compact && has && "text-accent",
      )}
    >
      <Icon name={has ? "verified_user" : "remove_moderator"} size={compact ? 15 : 14} />
      {compact ? (
        <>
          <span className="sr-only">{text}</span>
          {voice.kind === "agreed" && voice.atMs != null && <span aria-hidden className="text-[11.5px] font-normal">{shortDate(voice.atMs, locale)}</span>}
        </>
      ) : (
        <span>{text}</span>
      )}
    </span>
  );
}
