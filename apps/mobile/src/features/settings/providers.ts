// SPDX-License-Identifier: Apache-2.0
// Cloud providers as the UI names them. The ids come from Rust (cloud_models).
const NAMES: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  google: "Google",
  openrouter: "OpenRouter",
};

export const providerName = (id: string): string =>
  NAMES[id] ?? id.charAt(0).toUpperCase() + id.slice(1);
