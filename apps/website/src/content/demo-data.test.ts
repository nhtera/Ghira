// SPDX-License-Identifier: Apache-2.0

// The landing demo's data must not drift from the app: the sample meeting's
// timings and speakers equal packages/ui/mocks/sample-meeting.json, and every
// label copied into demo-data.ts equals the current locale value. Plain file
// reads, nothing imported from packages/.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { clock, COPIED, COPIED_KEYS, END, LANGS, LINES, laneSegments, MY_NOTES, NOTES, nextTime, shownLines, typedNotes } from "./demo-data.ts";

const here = new URL(".", import.meta.url);
const readJson = (rel: string) => JSON.parse(readFileSync(new URL(rel, here), "utf8"));

const sample = readJson("../../../../packages/ui/mocks/sample-meeting.json") as { transcript: { t: number; s: number }[]; durationSeconds: number };
const locales = { en: readJson("../../../../packages/i18n/locales/en.json"), vi: readJson("../../../../packages/i18n/locales/vi.json") } as Record<string, unknown>;

function lookup(root: unknown, dotted: string): unknown {
  return dotted.split(".").reduce<unknown>((node, key) => (node && typeof node === "object" ? (node as Record<string, unknown>)[key] : undefined), root);
}

test("the demo has the sample meeting's lines, times and speakers", () => {
  assert.equal(LINES.length, sample.transcript.length);
  assert.deepEqual(
    LINES.map((l) => l.t),
    sample.transcript.map((l) => l.t),
  );
  assert.deepEqual(
    LINES.map((l) => l.s),
    sample.transcript.map((l) => l.s),
  );
  assert.ok(END > LINES[LINES.length - 1].t);
});

test("every line has text in both languages", () => {
  for (const line of LINES) for (const lang of LANGS) assert.ok(line.text[lang].length > 0);
});

test("every copied label equals the current locale value", () => {
  for (const lang of LANGS) {
    for (const [field, key] of Object.entries(COPIED_KEYS)) {
      const copy = lookup(COPIED[lang], field);
      const real = lookup(locales[lang], key);
      assert.equal(typeof real, "string", `${lang}.json has no ${key}`);
      assert.equal(copy, real, `${field} (${lang}) differs from ${key}`);
    }
  }
});

test("every citation points at a line, and every typed note exists", () => {
  const cites = [...NOTES.summary, ...NOTES.decisions, ...NOTES.actions, ...NOTES.questions, ...NOTES.mine.flatMap((m) => (m.miss ? [] : [m]))];
  for (const n of cites) for (const i of n.c) assert.ok(LINES[i], `line ${i}`);
  for (const m of NOTES.mine) assert.ok(MY_NOTES[m.k], `typed note ${m.k}`);
});

test("the page is drawn at 27 s with three lines, the third still being recognized", () => {
  const shown = shownLines(27, "en");
  assert.deepEqual(
    shown.map((s) => s.i),
    [0, 1, 2],
  );
  assert.equal(shown[2].partial, true);
});

test("the demo clock loops after the end", () => {
  assert.ok(nextTime(27) > 27);
  assert.equal(nextTime(107), 2);
});

test("lanes, typed notes and the clock behave", () => {
  assert.deepEqual(laneSegments(3, 27), []);
  assert.equal(laneSegments(0, 27).length, 1);
  assert.equal(typedNotes(10, "en").length, 0);
  assert.equal(typedNotes(100, "en").at(-1)?.typing, false);
  assert.equal(clock(28.9), "00:28");
  assert.equal(clock(101), "01:41");
});
