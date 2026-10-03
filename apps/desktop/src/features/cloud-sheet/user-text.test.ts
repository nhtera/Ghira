// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { userText } from "./user-text";

const system = "You write meeting notes. ".repeat(80);

describe("userText", () => {
  it("takes the user message, not a longer system prompt (OpenAI)", () => {
    const p = JSON.stringify({ messages: [{ role: "system", content: system }, { role: "user", content: "short talk" }] });
    expect(userText(p)).toBe("short talk");
  });
  it("Anthropic: top-level system is ignored, text parts are joined", () => {
    const p = JSON.stringify({ system, messages: [{ role: "user", content: [{ type: "text", text: "a b" }, { type: "text", text: "c" }] }] });
    expect(userText(p)).toBe("a b\nc");
  });
  it("Gemini: user parts only", () => {
    const p = JSON.stringify({ systemInstruction: { parts: [{ text: system }] }, contents: [{ role: "user", parts: [{ text: "hello there" }] }, { role: "model", parts: [{ text: "x" }] }] });
    expect(userText(p)).toBe("hello there");
  });
  it("an unknown shape or non-JSON claims nothing", () => {
    expect(userText(JSON.stringify({ prompt: system }))).toBe("");
    expect(userText("nope")).toBe("");
  });
});
