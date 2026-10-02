// SPDX-License-Identifier: Apache-2.0
// Provider brand names are proper nouns, not UI copy.
const NAMES: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  gemini: "Gemini",
};

export const providerName = (id: string) => NAMES[id] ?? id;
