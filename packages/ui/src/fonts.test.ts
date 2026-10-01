// SPDX-License-Identifier: Apache-2.0
// The bundled fonts draw every Vietnamese letter (brief §10 sample lines and
// the whole Latin Extended Additional block), so nothing falls back mid-word.
// @vitest-environment node
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { createRequire } from "node:module";
import type * as Fontkit from "fontkit";

// Node 23.4's TextDecoder("ascii") returns a Buffer, not a string (Node 22
// is fine), which breaks fontkit's format probe; decode those as latin1.
if (typeof new TextDecoder("ascii").decode(Uint8Array.of(65)) !== "string") {
  const Base = TextDecoder;
  globalThis.TextDecoder = class extends Base {
    decode(input?: AllowSharedBufferSource, options?: TextDecodeOptions): string {
      const out: unknown = super.decode(input, options);
      return typeof out === "string" ? out : Buffer.from(out as Uint8Array).toString("latin1");
    }
  } as typeof TextDecoder;
}

// The Node build (the browser build has no WOFF2 decoder).
const fontkit = createRequire(import.meta.url)("fontkit") as typeof Fontkit;

const css = readFileSync(new URL("./fonts.css", import.meta.url), "utf8");
const here = dirname(fileURLToPath(import.meta.url));

const SAMPLES = [
  "Okay, bắt đầu nhé. Hôm nay mình chốt scope cho bản beta tháng 11.",
  "Em đã gom feedback từ 12 user test. Vấn đề lớn nhất là nhận diện người nói khi họp trực tiếp.",
  "Rename thì dễ. Khó là nhớ giọng qua các cuộc họp, cần voice profile.",
  "Khoảng hai tuần, gồm cả queue xử lý nền. Tốt. Chốt lại: live rename, voice profile có consent.",
  "Âm thanh không bao giờ rời khỏi máy. Đã dùng cloud cho cuộc họp này. Đang nhận diện người nói…",
  "ĂăÂâĐđÊêÔôƠơƯư",
];

function codePoints(): number[] {
  const set = new Set<number>();
  for (const s of SAMPLES) for (const ch of s.normalize("NFC")) set.add(ch.codePointAt(0)!);
  for (let cp = 0x1ea0; cp <= 0x1ef9; cp++) set.add(cp);
  return [...set].filter((cp) => cp > 0x20);
}

/** Every bundled file of a family, normal style (the union covers the subsets). */
function files(family: string): string[] {
  const out: string[] = [];
  for (const m of css.matchAll(/@font-face \{([^}]*)\}/g)) {
    const rule = m[1];
    if (!rule.includes(`font-family: '${family}'`) || rule.includes("font-style: italic")) continue;
    const url = /url\(([^)]+\.woff2)\)/.exec(rule)?.[1];
    if (url) out.push(join(here, url));
  }
  return [...new Set(out)];
}

describe.each(["Be Vietnam Pro", "Source Serif 4 Variable"])("%s", (family) => {
  it("has a glyph for every Vietnamese code point", () => {
    const fonts = files(family).map((f) => fontkit.create(readFileSync(f)) as Fontkit.Font);
    expect(fonts.length).toBeGreaterThan(0);
    const missing = codePoints().filter((cp) => !fonts.some((f) => f.hasGlyphForCodePoint(cp)));
    expect(missing.map((cp) => String.fromCodePoint(cp))).toEqual([]);
  });
});
