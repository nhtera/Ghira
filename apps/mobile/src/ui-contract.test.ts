// SPDX-License-Identifier: Apache-2.0
// @ghi/ui draws a chip for every MeetingChip the core sends: the two types must
// stay identical, so a new kind in bindings.ts fails the typecheck here.
import type { SyncChipKind } from "@ghi/ui";
import { expect, it } from "vitest";
import type { MeetingChip } from "./bindings";

type Same<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;
const same: Same<MeetingChip, SyncChipKind> = true;

it("SyncChipKind matches the MeetingChip contract", () => {
  expect(same).toBe(true);
});
