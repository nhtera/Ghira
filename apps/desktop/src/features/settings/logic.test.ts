// SPDX-License-Identifier: Apache-2.0
import { describe, expect, it } from "vitest";
import type { Licenses } from "../../generated/licenses";
import { addTerm, deleteWord, deleteWordMatches, editTerm, filterLicenses, licenseRows, passwordIssue, passwordStrength, retentionDeletes } from "./logic";

describe("vocabulary", () => {
  it("adds a trimmed term", () => {
    expect(addTerm(["Ghira"], "  Nguyễn   Văn An ", 200)).toEqual({ terms: ["Ghira", "Nguyễn Văn An"], status: "added" });
  });
  it("dedupes without caring for case or accents", () => {
    expect(addTerm(["Chốt"], "chot", 200).status).toBe("duplicate");
    expect(addTerm(["Đạt"], "DAT", 200).status).toBe("duplicate");
  });
  it("stops at the limit and ignores empty input", () => {
    expect(addTerm(["a", "b"], "c", 2).status).toBe("full");
    expect(addTerm(["a"], "   ", 2).status).toBe("empty");
  });
  it("edits in place, removes on empty, refuses a clash", () => {
    expect(editTerm(["a", "b"], 0, "c").terms).toEqual(["c", "b"]);
    expect(editTerm(["a", "b"], 0, "").terms).toEqual(["b"]);
    expect(editTerm(["a", "b"], 0, "B").status).toBe("duplicate");
    expect(editTerm(["a", "b"], 0, "A").terms).toEqual(["A", "b"]);
  });
});

describe("export password", () => {
  it("needs 8 characters and a matching repeat", () => {
    expect(passwordIssue("short", "short")).toBe("short");
    expect(passwordIssue("longenough", "different")).toBe("mismatch");
    expect(passwordIssue("longenough", "longenough")).toBeNull();
  });
  it("rates strength by length and variety", () => {
    expect(passwordStrength("abc")).toBe(0);
    expect(passwordStrength("abcdefgh")).toBe(1);
    expect(passwordStrength("Abcdefg1")).toBe(2);
    expect(passwordStrength("Abcdefgh1234!xyz")).toBe(3);
  });
});

describe("delete everything", () => {
  it("asks for the word of the app language, ignoring case and accents", () => {
    expect(deleteWord("en")).toBe("DELETE");
    expect(deleteWord("vi")).toBe("XÓA");
    expect(deleteWordMatches("delete", "DELETE")).toBe(true);
    expect(deleteWordMatches("xoa", "XÓA")).toBe(true);
    expect(deleteWordMatches("del", "DELETE")).toBe(false);
  });
});

describe("retention", () => {
  it("deletes audio only when the window shrinks", () => {
    expect(retentionDeletes(0, 30)).toBe(true);
    expect(retentionDeletes(90, 30)).toBe(true);
    expect(retentionDeletes(30, 90)).toBe(false);
    expect(retentionDeletes(30, 0)).toBe(false);
  });
});

const DATA: Licenses = {
  licenses: { mit: { id: "MIT", name: "MIT", text: "Permission is hereby granted" } },
  rust: [
    { name: "serde", version: "1.0.200", license: "MIT OR Apache-2.0", licenseKeys: ["mit"] },
    { name: "tokio", version: "1.40.0", license: "MIT", licenseKeys: ["mit"] },
  ],
  js: [{ name: "react", version: "19.0.0", license: "MIT", licenseKeys: ["mit"] }],
  models: [{ id: "m", name: "nvidia/model", license: "OpenMDW-1.1", url: "https://example.org/m", licenseKeys: [] }],
  assets: [{ name: "Be Vietnam Pro", license: "OFL-1.1", notice: "Copyright", licenseKeys: [] }],
};

describe("licenses", () => {
  const rows = licenseRows(DATA);
  it("flattens every group", () => {
    expect(rows.map((r) => r.group)).toEqual(["rust", "rust", "js", "models", "assets"]);
    expect(rows[0]!.text).toContain("Permission");
    expect(rows[3]!.url).toBe("https://example.org/m");
  });
  it("searches by name, version and license", () => {
    expect(filterLicenses(rows, "serde").map((r) => r.name)).toEqual(["serde"]);
    expect(filterLicenses(rows, "1.40").map((r) => r.name)).toEqual(["tokio"]);
    expect(filterLicenses(rows, "openmdw").length).toBe(1);
    expect(filterLicenses(rows, "vietnam pro").length).toBe(1);
    expect(filterLicenses(rows, "").length).toBe(5);
  });
});
