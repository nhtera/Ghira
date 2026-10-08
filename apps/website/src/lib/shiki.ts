// SPDX-License-Identifier: Apache-2.0

import type { ThemeRegistration } from "shiki";

// Code colors are CSS variables from Ghira's tokens, so one theme serves
// dark and light and follows the theme toggle with no re-render. Speaker
// colors double as syntax colors (they are AA on the surfaces in both themes).
const v = (name: string) => `var(--${name})`;

export const ghiraTheme: ThemeRegistration = {
  name: "ghira",
  type: "dark",
  colors: {
    "editor.foreground": v("ink"),
    "editor.background": v("surface2"),
  },
  fg: v("ink"),
  bg: v("surface2"),
  tokenColors: [
    { scope: ["comment", "punctuation.definition.comment"], settings: { foreground: v("muted") } },
    { scope: ["string", "string.regexp", "markup.inline.raw"], settings: { foreground: v("s3") } },
    { scope: ["constant.numeric", "constant.language", "constant.character", "constant.other"], settings: { foreground: v("s2") } },
    { scope: ["entity.name.section", "markup.heading", "entity.name.tag"], settings: { foreground: v("s5") } },
    { scope: ["support.type.property-name", "meta.object-literal.key", "variable.other.property", "entity.name.tag.toml", "keyword.key.toml"], settings: { foreground: v("s1") } },
    { scope: ["variable.parameter", "variable.other.readwrite", "punctuation.definition.variable"], settings: { foreground: v("s7") } },
    { scope: ["support.function", "entity.name.function", "entity.other.attribute-name"], settings: { foreground: v("s1") } },
    { scope: ["keyword", "storage", "storage.type", "keyword.operator"], settings: { foreground: v("s4") } },
    { scope: ["punctuation", "meta.brace"], settings: { foreground: v("muted") } },
  ],
};
