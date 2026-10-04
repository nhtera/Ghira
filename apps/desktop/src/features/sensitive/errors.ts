// SPDX-License-Identifier: Apache-2.0
// The core refuses sensitive mode with stable words; the user gets a sentence.
import type { TFunction } from "i18next";

export function sensitiveError(t: TFunction, error: string): string {
  if (error === "noTranscript") return t("sensitive.noTranscript");
  if (error === "transcriptPending") return t("sensitive.pending");
  return t("system.commandFailed", { message: error });
}
