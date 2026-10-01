// SPDX-License-Identifier: Apache-2.0
// tokens.json → src/tokens/tokens.css: theme variables per theme, the Tailwind
// v4 theme bound to them (`@theme inline`, so utilities follow data-theme),
// and the type scale as utilities. The CSS is committed; a test checks it is
// in sync. Run after editing tokens.json: pnpm --filter @ghi/ui gen:tokens
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = join(dirname(fileURLToPath(import.meta.url)), "../src/tokens");

/** accentSoft → accent-soft, onS → on-s, s1 → s1 */
export const cssName = (k) => k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);

export function tokensCss(t) {
  const vars = (theme) =>
    Object.entries(t.color[theme])
      .map(([k, v]) => `  --${cssName(k)}: ${v};`)
      .join("\n");
  const colors = Object.keys(t.color.light)
    .filter((k) => !["shadow", "scrim"].includes(k))
    .map((k) => `  --color-${cssName(k)}: var(--${cssName(k)});`)
    .join("\n");
  const radii = Object.entries(t.radius)
    .map(([k, v]) => `  --radius-${k}: ${v};`)
    .join("\n");
  const fonts = Object.entries(t.font)
    .map(([k, v]) => `  --font-${k}: ${v};`)
    .join("\n");
  const type = Object.entries(t.type)
    .map(([k, s]) => {
      const lines = [
        `  font-family: var(--font-${s.font});`,
        `  font-size: ${s.size};`,
        `  font-weight: ${s.weight};`,
        `  line-height: ${s.lh};`,
        s.tracking ? `  letter-spacing: ${s.tracking};` : null,
        s.case ? `  text-transform: ${s.case};` : null,
        s.font === "mono" ? "  font-variant-numeric: tabular-nums;" : null,
      ].filter(Boolean);
      // Vietnamese stacks diacritics: a little more line height (brief §8).
      const vi = s.lhVi !== s.lh ? `\n  &:lang(vi) {\n    line-height: ${s.lhVi};\n  }` : "";
      return `@utility text-${k} {\n${lines.join("\n")}${vi}\n}`;
    })
    .join("\n\n");
  return `/* SPDX-License-Identifier: Apache-2.0 */
/* Generated from tokens.json by scripts/build-tokens-css.mjs: do not edit. */

:root,
[data-theme="light"] {
  color-scheme: light;
${vars("light")}
  --motion-fast: ${t.motion.fast};
  --motion-base: ${t.motion.base};
  --motion-slow: ${t.motion.slow};
  --ease: ${t.motion.ease};
}

[data-theme="dark"] {
  color-scheme: dark;
${vars("dark")}
}

@custom-variant dark (&:where([data-theme="dark"], [data-theme="dark"] *));

@theme inline {
${colors}
${radii}
${fonts}
  --shadow-float: var(--shadow);
  --ease-out: var(--ease);
}

${type}
`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const t = JSON.parse(readFileSync(join(dir, "tokens.json"), "utf8"));
  writeFileSync(join(dir, "tokens.css"), tokensCss(t));
  console.log("wrote src/tokens/tokens.css");
}
