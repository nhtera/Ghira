// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { TemplateForm } from "../../bindings";
import { MAX_GUIDANCE, MAX_INSTRUCTION, MAX_NAME, MAX_SECTIONS, emptyForm, formIssue } from "./template-form";

const ok = (): TemplateForm => ({ name: "Retro", language: "en", guidance: "", sections: [{ id: null, title: "A", instruction: "B" }] });

describe("formIssue", () => {
  it("names the real problem", () => {
    expect(formIssue(ok())).toBeNull();
    expect(formIssue(emptyForm("en"))).toBe("nameRequired");
    expect(formIssue({ ...ok(), name: "x".repeat(MAX_NAME + 1) })).toBe("nameRequired");
    expect(formIssue({ ...ok(), guidance: "x".repeat(MAX_GUIDANCE + 1) })).toBe("guidanceTooLong");
    expect(formIssue({ ...ok(), sections: [{ id: null, title: "A", instruction: "" }] })).toBe("titleRequired");
    expect(formIssue({ ...ok(), sections: [{ id: null, title: "A", instruction: "x".repeat(MAX_INSTRUCTION + 1) }] })).toBe("titleRequired");
    expect(formIssue({ ...ok(), sections: Array.from({ length: MAX_SECTIONS + 1 }, () => ({ id: null, title: "A", instruction: "B" })) })).toBe("titleRequired");
  });

  it("counts characters the way the core does (one line, spaces collapsed, accents as one)", () => {
    expect(formIssue({ ...ok(), name: "  a   b  " })).toBeNull();
    expect(formIssue({ ...ok(), sections: [{ id: null, title: "đ", instruction: "đ".repeat(MAX_INSTRUCTION) }] })).toBeNull();
  });
});
