// SPDX-License-Identifier: Apache-2.0
/// <reference types="node" />
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { RAW_COMMANDS, scriptedCommands } from "./mock";

describe("mock ipc", () => {
  it("lists exactly the commands that return raw values", () => {
    const bindings = readFileSync(join(__dirname, "../bindings.ts"), "utf8");
    const raw = [...bindings.matchAll(/^\t(\w+): .*__TAURI_INVOKE/gm)]
      .filter((m) => !m[0].includes("typedError"))
      .map((m) => m[1])
      .sort();
    expect([...RAW_COMMANDS].sort()).toEqual(raw);
  });

  it("answers unscripted result commands with an error result", async () => {
    // An empty script, so the test doesn't depend on what mock-*.ts script.
    const r = await scriptedCommands({}).inboxList();
    expect(r).toEqual({ status: "error", error: "not mocked: inboxList" });
  });

  it("rejects unscripted raw commands instead of answering a result", async () => {
    await expect(scriptedCommands({}).requestMicPermission()).rejects.toThrow("not mocked");
  });
});
