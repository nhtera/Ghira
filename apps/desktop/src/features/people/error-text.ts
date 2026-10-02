// SPDX-License-Identifier: Apache-2.0
// The core refuses People and voice commands with a code, not English. Known
// codes get their own sentence; anything else (the app lock, an unknown
// failure) keeps the generic "command failed" line.
import type { TFunction } from "i18next";

const CODES = [
  "busyRecording", "notFound", "isMe", "samePerson", "noProfile", "storage", "liveMeeting", "notASpeaker", "farSide", "notMe", "noSuggestion",
  "thirdPartyOff", "notNamed", "noVoice", "invalidConsent", "noModel", "micPermission", "noMic", "notEnrolling", "tooShort", "tooQuiet",
] as const;
export type VoiceCode = (typeof CODES)[number];

export const isVoiceCode = (e: string): e is VoiceCode => (CODES as readonly string[]).includes(e);

export const errorText = (t: TFunction, error: string): string => (isVoiceCode(error) ? t(`people.errors.${error}`) : t("system.commandFailed", { message: error }));
