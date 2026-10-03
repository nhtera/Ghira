// SPDX-License-Identifier: Apache-2.0
// The transcript part of a request body: the user-role message text, for the
// "What is sent" excerpt and the word count. Mirrors `message_text` in
// crates/ghi-llm/src/preview.rs, minus the system prompt. Display only: the
// checksummed payload is what is sent. An unknown shape gives "" so the sheet
// claims nothing and opens the exact data instead.

const textOf = (c: unknown): string => {
  if (typeof c === "string") return c;
  if (Array.isArray(c)) return c.map((p) => (p && typeof p === "object" && typeof (p as { text?: unknown }).text === "string" ? (p as { text: string }).text : "")).filter(Boolean).join("\n");
  return "";
};

export function userText(payload: string): string {
  let v: unknown;
  try {
    v = JSON.parse(payload);
  } catch {
    return "";
  }
  if (!v || typeof v !== "object") return "";
  const o = v as { messages?: unknown; contents?: unknown };
  const out: string[] = [];
  // OpenAI and Anthropic: messages[role=user].content (a string or text parts).
  if (Array.isArray(o.messages)) {
    for (const m of o.messages as { role?: unknown; content?: unknown }[]) if (m?.role === "user") out.push(textOf(m.content));
  }
  // Gemini: contents[role=user or unset].parts[].text.
  if (Array.isArray(o.contents)) {
    for (const c of o.contents as { role?: unknown; parts?: unknown }[]) {
      if (c?.role !== undefined && c.role !== "user") continue;
      out.push(textOf(c?.parts));
    }
  }
  return out.filter(Boolean).join("\n");
}
