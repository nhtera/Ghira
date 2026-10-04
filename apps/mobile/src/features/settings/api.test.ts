// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import { errCode, knownError, unwrap } from "./api";
import { providerName } from "./providers";

describe("unwrap", () => {
  it("returns the data of an ok result", () => {
    expect(unwrap({ status: "ok", data: 7 })).toBe(7);
  });
  it("throws the error code of an error result", () => {
    expect(() => unwrap({ status: "error", error: "busy" })).toThrow("busy");
    expect(errCode(new Error("busy"))).toBe("busy");
  });
});

describe("knownError", () => {
  it("keeps the codes the screens have words for", () => {
    for (const code of [
      "busy",
      "passwordTooShort",
      "locked",
      "unsupportedType",
    ])
      expect(knownError(code)).toBe(code);
  });
  it("maps the core's locked refusal", () => {
    expect(knownError("the app is locked")).toBe("locked");
  });
  it("maps Rust's strict-offline text and falls back to generic", () => {
    expect(knownError("strict offline is on: nothing can be sent")).toBe(
      "offline",
    );
    expect(knownError("something unexpected")).toBe("generic");
    expect(knownError(null)).toBe("generic");
  });
});

describe("providerName", () => {
  it("names the known providers and capitalizes others", () => {
    expect(providerName("openai")).toBe("OpenAI");
    expect(providerName("mistral")).toBe("Mistral");
  });
});
