// SPDX-License-Identifier: Apache-2.0
// A cloud model's listed price (USD per 1M input / output tokens, from the
// core's price table) for the model menus: "$2", "$0.10", "$1.25".
import type { TFunction } from "i18next";
import type { CloudModel } from "../../bindings";

export function perMillion(usd: number, lang: string): string {
  const cents = usd < 1 || !Number.isInteger(usd);
  return new Intl.NumberFormat(lang, { style: "currency", currency: "USD", minimumFractionDigits: cents ? 2 : 0, maximumFractionDigits: 2 }).format(usd);
}

/** "claude-sonnet-5-5 · $2 / $10", or the bare id when it has no price. */
export function modelOption(t: TFunction, m: CloudModel, lang: string): string {
  if (m.inputUsdPerM == null || m.outputUsdPerM == null) return m.model;
  return t("cloud.modelOption", { model: m.model, input: perMillion(m.inputUsdPerM, lang), output: perMillion(m.outputUsdPerM, lang) });
}

